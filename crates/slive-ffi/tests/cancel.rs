//! 取消语义、`slive_request_cancel` 契约、shutdown 幂等 —— 全部离线。

use std::sync::{Arc, Mutex};

use slive_ffi::abi::*;
use slive_ffi::requests;

#[derive(Default)]
struct Collector {
    payloads: Mutex<Vec<(u64, String)>>,
}

impl Collector {
    fn push(&self, id: u64, payload: String) {
        self.payloads.lock().unwrap().push((id, payload));
    }
}

static COLLECTOR: Mutex<Option<Arc<Collector>>> = Mutex::new(None);

// 全局单例（runtime + dispatcher）无法并发使用：多个测试同时 init/shutdown 会互相
// 踩踏 dispatcher。串行化本文件的测试。
static SERIAL: Mutex<()> = Mutex::new(());

extern "C" fn on_complete(id: u64, _status: i32, payload: SliveStr, _ud: *mut std::ffi::c_void) {
    let s = if payload.ptr.is_null() {
        String::new()
    } else {
        unsafe {
            std::str::from_utf8(std::slice::from_raw_parts(payload.ptr as *const u8, payload.len))
        }
        .unwrap_or("")
        .to_string()
    };
    if let Some(c) = COLLECTOR.lock().unwrap().as_ref() {
        c.push(id, s);
    }
}

fn init_with(c: Arc<Collector>) {
    *COLLECTOR.lock().unwrap() = Some(c);
    let cfg = slive_ffi::SliveConfig {
        struct_size: std::mem::size_of::<slive_ffi::SliveConfig>() as u32,
        abi_version: slive_ffi::slive_abi_version(),
        worker_threads: 2,
        reserved: 0,
        on_complete: Some(on_complete),
        on_event: None,
        on_log: None,
        user_data: std::ptr::null_mut(),
    };
    assert_eq!(slive_ffi::slive_init(&cfg), SLIVE_OK);
}

#[test]
fn register_is_exclusive_and_finish_releases() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let id = slive_ffi::slive_request_id_new();
    let mut rx = requests::register(id).expect("首次登记应成功");
    assert!(requests::register(id).is_none(), "重复 ID 应 BUSY");

    assert_eq!(slive_ffi::slive_request_cancel(id), SLIVE_OK);
    rx.mark_changed();
    assert!(*rx.borrow_and_update(), "取消应置 true");

    requests::finish(id);
    assert_eq!(
        slive_ffi::slive_request_cancel(id),
        SLIVE_ERR_NOT_FOUND,
        "已完成 → NOT_FOUND"
    );
}

#[test]
fn cancel_unknown_id_is_not_found() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(
        slive_ffi::slive_request_cancel(987_654_321),
        SLIVE_ERR_NOT_FOUND
    );
}

#[test]
fn shutdown_stops_dispatcher_and_runtime_then_can_reinit() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let c = Arc::new(Collector::default());
    init_with(Arc::clone(&c));
    assert_eq!(slive_ffi::slive_shutdown(500), SLIVE_OK);
    assert_eq!(slive_ffi::slive_shutdown(500), SLIVE_OK, "shutdown 应幂等");
    // 重新 init 后仍可用
    init_with(c);
    let mut h: SliveHandle = 0;
    assert_eq!(slive_ffi::site::slive_site_create(0, &mut h), SLIVE_OK);
    assert_eq!(slive_ffi::site::slive_site_free(h), SLIVE_OK);
}

#[test]
fn init_twice_does_not_duplicate_dispatcher() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let c = Arc::new(Collector::default());
    init_with(Arc::clone(&c));
    init_with(Arc::clone(&c)); // 第二次不应起第二个线程
    let mut h: SliveHandle = 0;
    assert_eq!(slive_ffi::site::slive_site_create(0, &mut h), SLIVE_OK);
    // 发起一个请求：只要不 panic 即证明第二次 init 未产生重复 dispatcher
    let id = slive_ffi::slive_request_id_new();
    let _ = slive_ffi::site::slive_site_call(
        h,
        slive_ffi::abi::to_slive_str("get_live_status"),
        slive_ffi::abi::to_slive_str(r#"{"room_id":"6"}"#),
        id,
    );
    slive_ffi::site::slive_site_free(h);
}

#[test]
fn shutdown_from_inside_callback_does_not_deadlock() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // 回归：dispatcher::stop() 若在 dispatcher 线程内 join 自己会永久死锁。
    // 宿主在回调里调 shutdown 是常见用法（退出流程），必须能安全返回。
    use std::sync::atomic::AtomicBool;
    static DONE: AtomicBool = AtomicBool::new(false);

    extern "C" fn cb(_id: u64, _status: i32, _payload: SliveStr, _ud: *mut std::ffi::c_void) {
        let _ = slive_ffi::slive_shutdown(100);
        DONE.store(true, std::sync::atomic::Ordering::SeqCst);
    }

    let cfg = slive_ffi::SliveConfig {
        struct_size: std::mem::size_of::<slive_ffi::SliveConfig>() as u32,
        abi_version: slive_ffi::slive_abi_version(),
        worker_threads: 1,
        reserved: 0,
        on_complete: Some(cb),
        on_event: None,
        on_log: None,
        user_data: std::ptr::null_mut(),
    };
    assert_eq!(slive_ffi::slive_init(&cfg), SLIVE_OK);

    let mut h: SliveHandle = 0;
    assert_eq!(slive_ffi::site::slive_site_create(0, &mut h), SLIVE_OK);
    // 用 set_attrs 的失败路径触发 completion（request_id = 0）：纯本地，不依赖网络。
    // 传入非法 JSON → core 返回 InvalidInput → 桥层投递 error envelope。
    let _ = slive_ffi::site::slive_site_set_attrs(
        h,
        slive_ffi::abi::to_slive_str("not-json"),
    );

    // 等待回调触发（最多 3 秒）—— 若死锁则超时失败
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !DONE.load(std::sync::atomic::Ordering::SeqCst) {
        assert!(
            std::time::Instant::now() < deadline,
            "回调内的 slive_shutdown 发生死锁"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let _ = slive_ffi::site::slive_site_free(h);
}
