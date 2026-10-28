/* slive.h — Simple Live C ABI
 *
 * 所有字符串为 UTF-8 + 显式长度（非 NUL 结尾）：日志可能含 '\0'。
 * 所有出参 SliveStr 必须由调用方 slive_str_free 释放。
 *
 * ⚠️ 内存归属：出参 SliveStr 由 Rust 分配，**必须调用本库的 slive_str_free**，
 *    不可用宿主自己的 free() —— 跨分配器释放在 CRT 不一致时会损坏堆
 *    （Windows 上 Rust 与宿主可能链到不同的 msvcrt）。
 *
 * ⚠️ 入参契约：传入的 SliveStr 必须指向非空且可读至少 len 字节的已初始化内存，
 *    且 len 不得超过实际分配长度（越界读无法被库校验）。字节须为合法 UTF-8。
 *
 * 线程：所有回调（on_complete / on_event / on_log）在**同一个专用线程**上串行触发，
 *       宿主需自行 marshal 到 UI 线程（WinUI3: DispatcherQueue；Qt: QMetaObject::invokeMethod）。
 */
#ifndef SLIVE_H
#define SLIVE_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

#define SLIVE_ABI_VERSION 0x00010001u
#define SLIVE_SCHEMA_VERSION 1u

/* 返回码：0 = 已受理/成功；1 = core 业务错误（解析 envelope）；负数 = ABI 错误 */
#define SLIVE_OK                  0
#define SLIVE_ERR_CORE            1
#define SLIVE_ERR_INVALID_HANDLE (-1)
#define SLIVE_ERR_INVALID_ARG    (-2)
#define SLIVE_ERR_INVALID_UTF8   (-3)
#define SLIVE_ERR_NOT_INIT       (-4)
#define SLIVE_ERR_BUSY           (-5)
#define SLIVE_ERR_PANIC          (-6)
#define SLIVE_ERR_NOT_FOUND      (-7)
#define SLIVE_ERR_CANCELLED      (-8)
#define SLIVE_ERR_UNSUPPORTED    (-9)

/* 平台判别值（与 Rust Platform 一致） */
#define SLIVE_PLATFORM_BILIBILI 0
#define SLIVE_PLATFORM_DOUYU    1
#define SLIVE_PLATFORM_HUYA     2
#define SLIVE_PLATFORM_DOUYIN   3
#define SLIVE_PLATFORM_TWITCH   4

/* 站点方法名（传给 slive_site_call） */
#define SLIVE_METHOD_GET_CATEGORIES            "get_categories"
#define SLIVE_METHOD_GET_CATEGORY_ROOMS        "get_category_rooms"
#define SLIVE_METHOD_GET_RECOMMEND_ROOMS       "get_recommend_rooms"
#define SLIVE_METHOD_SEARCH_ROOMS              "search_rooms"
#define SLIVE_METHOD_SEARCH_ANCHORS            "search_anchors"
#define SLIVE_METHOD_GET_ROOM_DETAIL           "get_room_detail"
#define SLIVE_METHOD_GET_PLAY_QUALITIES        "get_play_qualities"
#define SLIVE_METHOD_GET_PLAY_URLS             "get_play_urls"
#define SLIVE_METHOD_GET_LIVE_STATUS           "get_live_status"
#define SLIVE_METHOD_GET_SUPER_CHAT_MESSAGES   "get_super_chat_messages"
#define SLIVE_METHOD_GET_QR_CODE               "get_qr_code"
#define SLIVE_METHOD_GET_QR_POLL_RESULT        "get_qr_poll_result"
#define SLIVE_METHOD_GET_ACCOUNT_INFO          "get_account_info"

/* 弹幕事件 kind（on_event 的 event_kind） */
#define SLIVE_DANMAKU_READY        0
#define SLIVE_DANMAKU_RECONNECTING 1
#define SLIVE_DANMAKU_CLOSED       2
#define SLIVE_DANMAKU_MESSAGE      3

typedef uint64_t SliveHandle;            /* 0 = 无效 */

typedef struct SliveStr { const char* ptr; size_t len; } SliveStr;

typedef void (*SliveCompletionFn)(uint64_t request_id, int32_t status, SliveStr payload, void* user_data);
typedef void (*SliveEventFn)(SliveHandle conn, int32_t event_kind, SliveStr payload, void* user_data);
typedef void (*SliveLogFn)(int32_t level, SliveStr target, SliveStr message, void* user_data);

typedef struct SliveConfig {
    uint32_t struct_size;     /* = sizeof(SliveConfig)，前向兼容 */
    uint32_t abi_version;     /* = SLIVE_ABI_VERSION */
    uint32_t worker_threads;  /* 0 = 默认（CPU 核数） */
    uint32_t reserved;
    SliveCompletionFn on_complete;
    SliveEventFn      on_event;
    SliveLogFn        on_log;
    void*             user_data;
} SliveConfig;

/* --- 生命周期 --- */
uint32_t slive_abi_version(void);
int32_t  slive_init(const SliveConfig* cfg);
int32_t  slive_shutdown(uint32_t timeout_ms);
uint64_t slive_request_id_new(void);
int32_t  slive_request_cancel(uint64_t request_id);

/* --- 站点 --- */
int32_t slive_site_create(int32_t platform, SliveHandle* out);
int32_t slive_site_free(SliveHandle h);
int32_t slive_site_id(SliveHandle h, SliveStr* out);
int32_t slive_site_name(SliveHandle h, SliveStr* out);
int32_t slive_site_set_attrs(SliveHandle h, SliveStr json);
int32_t slive_site_call(SliveHandle h, SliveStr method, SliveStr args_json, uint64_t request_id);

/* --- 弹幕 --- */
int32_t slive_danmaku_create(int32_t platform, SliveHandle* out);
int32_t slive_danmaku_start(SliveHandle h, SliveStr args);
int32_t slive_danmaku_stop(SliveHandle h);
int32_t slive_danmaku_free(SliveHandle h);

/* --- 弹幕去重/合并 --- */
int32_t slive_mask_create(uint32_t base_window_ms, uint16_t bucket_count,
                          uint8_t use_normalization, uint8_t use_frequency_control,
                          uint16_t max_frequency, SliveHandle* out);
int32_t slive_mask_allow(SliveHandle h, SliveStr texts_json, uint64_t now_ms, SliveStr* out_json);
int32_t slive_mask_merge(SliveHandle h, SliveStr texts_json, uint64_t now_ms, SliveStr* out_json);
int32_t slive_mask_reset(SliveHandle h);
int32_t slive_mask_free(SliveHandle h);

/* --- 字符串 --- */
void slive_str_free(SliveStr s);

#ifdef __cplusplus
} /* extern "C" */
#endif
#endif /* SLIVE_H */
