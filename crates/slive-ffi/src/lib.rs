//! Simple Live C ABI。
//!
//! 设计要点（ADR-016）：
//! - 句柄是 `u64` 而非裸指针 —— 悬垂使用返回 `INVALID_HANDLE`，不是 UB；
//! - 回调在**专用 dispatcher 线程**串行触发，宿主只需 marshal 一次到 UI 线程；
//! - 每个 `extern "C"` 函数体都包 `catch_unwind`（unwinding 跨 FFI 是 UB）；
//! - 字符串是 `{ptr,len}`（非 NUL 结尾），由调用方 `slive_str_free` 释放。

#![allow(clippy::missing_safety_doc)]
// C ABI 的出参指针由宿主保证有效；Rust 侧签名保持非 unsafe 以匹配 C 调用约定。
#![allow(clippy::not_unsafe_ptr_arg_deref)]

#[cfg(panic = "abort")]
compile_error!("slive-ffi 需要 panic=unwind：panic=abort 会让 catch_unwind 守卫失效");

pub mod abi;
pub mod args;
pub mod danmaku;
pub mod dispatcher;
pub mod envelope;
pub mod logging;
pub mod mask;
pub mod methods;
pub mod registry;
pub mod requests;
pub mod runtime;
pub mod site;

use std::os::raw::c_int;

use abi::*;

/// ABI 版本（高 16 位主版本，低 16 位次版本）。
#[no_mangle]
pub extern "C" fn slive_abi_version() -> u32 {
    0x0001_0001
}

/// 回调类型（与 `include/slive.h` 一一对应）。
pub type SliveCompletionFn =
    extern "C" fn(request_id: u64, status: i32, payload: SliveStr, user_data: *mut std::ffi::c_void);
pub type SliveEventFn =
    extern "C" fn(conn: SliveHandle, event_kind: i32, payload: SliveStr, user_data: *mut std::ffi::c_void);
pub type SliveLogFn = extern "C" fn(level: i32, target: SliveStr, message: SliveStr, user_data: *mut std::ffi::c_void);

/// 初始化配置（首字段 `struct_size` 便于前向兼容）。
#[repr(C)]
pub struct SliveConfig {
    pub struct_size: u32,
    pub abi_version: u32,
    pub worker_threads: u32,
    pub reserved: u32,
    pub on_complete: Option<SliveCompletionFn>,
    pub on_event: Option<SliveEventFn>,
    pub on_log: Option<SliveLogFn>,
    pub user_data: *mut std::ffi::c_void,
}

struct Callbacks {
    on_complete: Option<SliveCompletionFn>,
    on_event: Option<SliveEventFn>,
    on_log: Option<SliveLogFn>,
    // 裸指针不是 Send/Sync；用 AtomicPtr 包裹后，Callbacks 本身可安全共享。
    // 指针指向的对象的线程安全由宿主保证（FFI 固有约束，已在头文件注明）。
    user_data: std::sync::atomic::AtomicPtr<std::ffi::c_void>,
}

// 回调表：Mutex 而非 OnceLock —— 允许 `slive_init` 再次调用时**更新**回调
// （OnceLock::set 只生效一次，第二次 init 会静默沿用旧回调）。
static CALLBACKS: std::sync::Mutex<Option<Callbacks>> = std::sync::Mutex::new(None);

/// 初始化：启动 runtime 与 dispatcher 线程。
///
/// `cfg` 为空指针时使用默认配置（无回调、默认线程数）。
#[no_mangle]
pub extern "C" fn slive_init(cfg: *const SliveConfig) -> c_int {
    guard(|| {
        let (worker_threads, on_complete, on_event, on_log, user_data) = if cfg.is_null() {
            (0u32, None, None, None, std::ptr::null_mut())
        } else {
            let c = unsafe { &*cfg };
            (c.worker_threads, c.on_complete, c.on_event, c.on_log, c.user_data)
        };

        if runtime::init(worker_threads).is_err() {
            return SLIVE_ERR_INVALID_ARG;
        }

        *CALLBACKS.lock().unwrap_or_else(|e| e.into_inner()) = Some(Callbacks {
            on_complete,
            on_event,
            on_log,
            user_data: std::sync::atomic::AtomicPtr::new(user_data),
        });

        dispatcher::start(Box::new(dispatch_event));
        logging::install();
        SLIVE_OK
    })
}

fn dispatch_event(ev: dispatcher::Event) {
    // 先把回调与 user_data 克隆出来并**立即释放锁** ——
    // 回调是宿主代码，宿主在回调里调 slive_init / slive_shutdown 时会再取这把锁，
    // 持锁调用必然死锁。
    let (on_complete, on_event, on_log, user_data) = {
        let guard = CALLBACKS.lock().unwrap_or_else(|e| e.into_inner());
        match guard.as_ref() {
            None => return,
            Some(cb) => (
                cb.on_complete,
                cb.on_event,
                cb.on_log,
                cb.user_data.load(std::sync::atomic::Ordering::Acquire),
            ),
        }
    };
    match ev {
        dispatcher::Event::Completion {
            request_id,
            status,
            payload,
        } => {
            if let Some(f) = on_complete {
                let s = to_slive_str(&payload);
                f(request_id, status, s, user_data);
            }
        }
        dispatcher::Event::Danmaku { conn, kind, payload } => {
            if let Some(f) = on_event {
                let s = to_slive_str(&payload);
                f(conn, kind, s, user_data);
            }
        }
        dispatcher::Event::Log { level, target, message } => {
            if let Some(f) = on_log {
                let t = to_slive_str(&target);
                let m = to_slive_str(&message);
                f(level, t, m, user_data);
            }
        }
    }
}

/// 关闭：取消全部在途请求、停止 dispatcher 线程、释放 runtime。
///
/// 在回调线程内调用时立即返回（由 dispatcher 自行收尾，绝不自我 join）。
#[no_mangle]
pub extern "C" fn slive_shutdown(_timeout_ms: u32) -> c_int {
    guard(|| {
        requests::cancel_all();
        // dispatcher::stop() 自保护：在 dispatcher 线程内只关通道不 join（避免自我等待），
        // 线程会在当前回调返回后退出。
        dispatcher::stop();
        runtime::shutdown();
        SLIVE_OK
    })
}

/// 取消一个在途请求（该请求会收到 `SLIVE_ERR_CANCELLED` 的 completion）。
///
/// 不存在或已完成 → `SLIVE_ERR_NOT_FOUND`。
#[no_mangle]
pub extern "C" fn slive_request_cancel(request_id: u64) -> c_int {
    guard(|| {
        if requests::cancel(request_id) {
            SLIVE_OK
        } else {
            SLIVE_ERR_NOT_FOUND
        }
    })
}

static REQUEST_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// 生成单调递增的请求 ID。
#[no_mangle]
pub extern "C" fn slive_request_id_new() -> u64 {
    REQUEST_SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn abi_version_is_1_1() {
        assert_eq!(slive_abi_version(), 0x0001_0001);
    }

    #[test]
    fn request_ids_are_monotonic() {
        let a = slive_request_id_new();
        let b = slive_request_id_new();
        assert!(b > a);
    }

    #[test]
    fn init_is_idempotent() {
        assert_eq!(slive_init(std::ptr::null()), SLIVE_OK);
        assert_eq!(slive_init(std::ptr::null()), SLIVE_OK);
    }
}
