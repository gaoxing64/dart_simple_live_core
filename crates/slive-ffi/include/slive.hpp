// slive.hpp — Simple Live C ABI 的 C++17 RAII 封装（Qt / MyGUI 直接 include）
//
// 用法：
//   slive::Runtime rt;                              // 构造即 init，析构即 shutdown
//   auto site = slive::Site::create(slive::Platform::Bilibili);
//   auto json = site.call("get_room_detail", R"({"room_id":"6"})").get();   // 阻塞等 envelope
//   // 或异步：site.call_async(..., [](std::string json){ ... });
//
// 线程：所有回调在库内专用线程串行触发。若要在 UI 线程处理，请在 on_complete 内做一次投递
//   （Qt: QMetaObject::invokeMethod(obj, ..., Qt::QueuedConnection)；
//    WinUI3: DispatcherQueue.TryEnqueue）。
#pragma once

#include <atomic>
#include <condition_variable>
#include <cstdint>
#include <cstring>
#include <functional>
#include <future>
#include <memory>
#include <mutex>
#include <stdexcept>
#include <string>
#include <unordered_map>
#include <utility>

#include "slive.h"

namespace slive {

enum class Platform : int32_t {
    Bilibili = SLIVE_PLATFORM_BILIBILI,
    Douyu = SLIVE_PLATFORM_DOUYU,
    Huya = SLIVE_PLATFORM_HUYA,
    Douyin = SLIVE_PLATFORM_DOUYIN,
    Twitch = SLIVE_PLATFORM_TWITCH,
};

inline std::string to_string(SliveStr s) {
    return (s.ptr && s.len) ? std::string(s.ptr, s.len) : std::string();
}

inline SliveStr view(const std::string& s) {
    return SliveStr{s.data(), s.size()};
}

namespace detail {

/// 在途请求表：由 Runtime 的 on_complete 负责 fulfill。
class PendingTable {
public:
    void put(uint64_t id, std::shared_ptr<std::promise<std::string>> p) {
        std::lock_guard<std::mutex> lk(mu_);
        map_[id] = std::move(p);
    }

    /// 取出并 fulfill；找不到返回 false。
    bool fulfill(uint64_t id, std::string payload) {
        std::shared_ptr<std::promise<std::string>> p;
        {
            std::lock_guard<std::mutex> lk(mu_);
            auto it = map_.find(id);
            if (it == map_.end()) return false;
            p = std::move(it->second);
            map_.erase(it);
        }
        p->set_value(std::move(payload));
        return true;
    }

    void erase(uint64_t id) {
        std::lock_guard<std::mutex> lk(mu_);
        map_.erase(id);
    }

private:
    std::mutex mu_;
    std::unordered_map<uint64_t, std::shared_ptr<std::promise<std::string>>> map_;
};

}  // namespace detail

/// 全局运行时：构造即 init，析构即 shutdown。进程内只需一个。
class Runtime {
public:
    using CompleteFn = std::function<void(uint64_t, int32_t, std::string)>;
    using EventFn = std::function<void(SliveHandle, int32_t, std::string)>;
    using LogFn = std::function<void(int32_t, std::string, std::string)>;

    explicit Runtime(CompleteFn on_complete = {}, EventFn on_event = {}, LogFn on_log = {},
                     uint32_t worker_threads = 0)
        : on_complete_(std::move(on_complete)),
          on_event_(std::move(on_event)),
          on_log_(std::move(on_log)) {
        SliveConfig cfg{};
        cfg.struct_size = sizeof(SliveConfig);
        cfg.abi_version = SLIVE_ABI_VERSION;
        cfg.worker_threads = worker_threads;
        cfg.on_complete = &Runtime::c_complete;
        cfg.on_event = &Runtime::c_event;
        cfg.on_log = &Runtime::c_log;
        cfg.user_data = this;
        if (slive_init(&cfg) != SLIVE_OK) {
            throw std::runtime_error("slive_init failed");
        }
        current_ = this;
    }

    ~Runtime() {
        if (current_ == this) current_ = nullptr;
        slive_shutdown(2000);
    }

    Runtime(const Runtime&) = delete;
    Runtime& operator=(const Runtime&) = delete;

    /// 当前 Runtime 实例（由 Site::call 使用；未创建时抛异常）。
    static Runtime& current() {
        auto* r = current_;
        if (!r) throw std::runtime_error("slive::Runtime not constructed");
        return *r;
    }

    detail::PendingTable& pending() { return pending_; }

private:
    static void c_complete(uint64_t id, int32_t status, SliveStr payload, void* ud) {
        auto* self = static_cast<Runtime*>(ud);
        if (!self) return;
        auto json = to_string(payload);
        // 先 fulfill 内部 promise（Site::call 的 future），再交给宿主的自定义回调
        self->pending_.fulfill(id, json);
        if (self->on_complete_) self->on_complete_(id, status, std::move(json));
    }
    static void c_event(SliveHandle h, int32_t kind, SliveStr payload, void* ud) {
        auto* self = static_cast<Runtime*>(ud);
        if (self && self->on_event_) self->on_event_(h, kind, to_string(payload));
    }
    static void c_log(int32_t lv, SliveStr target, SliveStr msg, void* ud) {
        auto* self = static_cast<Runtime*>(ud);
        if (self && self->on_log_) self->on_log_(lv, to_string(target), to_string(msg));
    }

    CompleteFn on_complete_;
    EventFn on_event_;
    LogFn on_log_;
    detail::PendingTable pending_;

    static inline Runtime* current_ = nullptr;
};

/// 站点句柄（移动语义，析构自动 free）。
class Site {
public:
    static Site create(Platform p) {
        SliveHandle h = 0;
        if (slive_site_create(static_cast<int32_t>(p), &h) != SLIVE_OK) {
            throw std::runtime_error("slive_site_create failed");
        }
        return Site(h);
    }

    Site(Site&& o) noexcept : handle_(o.handle_) { o.handle_ = 0; }

    Site& operator=(Site&& o) noexcept {
        if (this != &o) {
            reset();
            handle_ = o.handle_;
            o.handle_ = 0;
        }
        return *this;
    }

    Site(const Site&) = delete;
    Site& operator=(const Site&) = delete;

    ~Site() { reset(); }

    std::string id() const { return read_str(&slive_site_id); }
    std::string name() const { return read_str(&slive_site_name); }

    void set_attrs(const std::string& json) {
        if (slive_site_set_attrs(handle_, view(json)) != SLIVE_OK) {
            throw std::runtime_error("slive_site_set_attrs failed");
        }
    }

    /// 异步调用：返回 envelope JSON 的 future。
    ///
    /// 由 `Runtime` 的 `on_complete` 负责 fulfill；因此**必须先构造 `slive::Runtime`**。
    std::future<std::string> call(const std::string& method, const std::string& args_json) {
        auto promise = std::make_shared<std::promise<std::string>>();
        auto future = promise->get_future();
        auto& rt = Runtime::current();

        uint64_t id = slive_request_id_new();
        rt.pending().put(id, promise);

        int32_t rc = slive_site_call(handle_, view(method), view(args_json), id);
        if (rc < 0) {
            rt.pending().erase(id);
            promise->set_value(R"({"schema_version":1,"ok":false,"error":{"kind":"abi","message":"call rejected","status_code":-2}})");
        }
        return future;
    }

    /// 非阻塞：结果通过回调送达（在库内 dispatcher 线程上）。
    void call_async(const std::string& method, const std::string& args_json,
                    std::function<void(std::string)> on_done) {
        auto fut = call(method, args_json);
        std::thread([fut = std::move(fut), cb = std::move(on_done)]() mutable {
            cb(fut.get());
        }).detach();
    }

    SliveHandle handle() const { return handle_; }

private:
    explicit Site(SliveHandle h) : handle_(h) {}

    void reset() {
        if (handle_) {
            slive_site_free(handle_);
            handle_ = 0;
        }
    }

    std::string read_str(int32_t (*f)(SliveHandle, SliveStr*)) const {
        SliveStr out{nullptr, 0};
        if (f(handle_, &out) != SLIVE_OK) return {};
        std::string s = to_string(out);
        slive_str_free(out);
        return s;
    }

    SliveHandle handle_ = 0;
};

/// 弹幕句柄。
class Danmaku {
public:
    static Danmaku create(Platform p) {
        SliveHandle h = 0;
        if (slive_danmaku_create(static_cast<int32_t>(p), &h) != SLIVE_OK) {
            throw std::runtime_error("slive_danmaku_create failed");
        }
        return Danmaku(h);
    }

    Danmaku(Danmaku&& o) noexcept : handle_(o.handle_) { o.handle_ = 0; }

    Danmaku& operator=(Danmaku&& o) noexcept {
        if (this != &o) {
            reset();
            handle_ = o.handle_;
            o.handle_ = 0;
        }
        return *this;
    }

    Danmaku(const Danmaku&) = delete;
    Danmaku& operator=(const Danmaku&) = delete;

    ~Danmaku() { reset(); }

    void start(const std::string& args) {
        if (slive_danmaku_start(handle_, view(args)) != SLIVE_OK) {
            throw std::runtime_error("slive_danmaku_start failed");
        }
    }
    void stop() { slive_danmaku_stop(handle_); }
    SliveHandle handle() const { return handle_; }

private:
    explicit Danmaku(SliveHandle h) : handle_(h) {}
    void reset() {
        if (handle_) {
            slive_danmaku_free(handle_);
            handle_ = 0;
        }
    }
    SliveHandle handle_ = 0;
};

/// 弹幕去重/合并（滑动窗口）。
class Mask {
public:
    Mask(uint32_t window_ms, uint16_t buckets, bool normalize, bool freq_control, uint16_t max_freq) {
        SliveHandle h = 0;
        if (slive_mask_create(window_ms, buckets, normalize ? 1 : 0, freq_control ? 1 : 0, max_freq, &h) != SLIVE_OK) {
            throw std::runtime_error("slive_mask_create failed");
        }
        handle_ = h;
    }

    Mask(Mask&& o) noexcept : handle_(o.handle_) { o.handle_ = 0; }
    Mask& operator=(Mask&& o) noexcept {
        if (this != &o) {
            reset();
            handle_ = o.handle_;
            o.handle_ = 0;
        }
        return *this;
    }
    Mask(const Mask&) = delete;

    ~Mask() { reset(); }

    std::string allow(const std::string& texts_json, uint64_t now_ms) {
        return read(&slive_mask_allow, texts_json, now_ms);
    }
    std::string merge(const std::string& texts_json, uint64_t now_ms) {
        return read(&slive_mask_merge, texts_json, now_ms);
    }

private:
    void reset() {
        if (handle_) {
            slive_mask_free(handle_);
            handle_ = 0;
        }
    }

    std::string read(int32_t (*f)(SliveHandle, SliveStr, uint64_t, SliveStr*),
                     const std::string& texts, uint64_t now) {
        SliveStr out{nullptr, 0};
        if (f(handle_, view(texts), now, &out) != SLIVE_OK) return {};
        std::string s = to_string(out);
        slive_str_free(out);
        return s;
    }
    SliveHandle handle_ = 0;
};

}  // namespace slive
