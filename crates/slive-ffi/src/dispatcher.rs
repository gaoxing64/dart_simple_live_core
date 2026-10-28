//! 所有回调在**专用非 runtime 线程**上串行执行：
//! - 宿主不必处理多线程回调（WinUI3/Qt 只需在包装层 marshal 一次）；
//! - 绝不在 tokio worker 上回调，避免宿主在回调里阻塞拖垮 reactor。
//!
//! `start` 幂等（重复 `slive_init` 不会起第二个线程）；`stop` 关闭通道并 join 线程。

use std::cell::Cell;
use std::sync::Mutex;
use std::thread::JoinHandle;

use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

thread_local! {
    static IN_DISPATCHER: Cell<bool> = const { Cell::new(false) };
}

pub enum Event {
    Completion {
        request_id: u64,
        status: i32,
        payload: String,
    },
    Danmaku {
        conn: u64,
        kind: i32,
        payload: String,
    },
    Log {
        level: i32,
        target: String,
        message: String,
    },
}

struct State {
    tx: Option<UnboundedSender<Event>>,
    join: Option<JoinHandle<()>>,
}

static STATE: Mutex<State> = Mutex::new(State {
    tx: None,
    join: None,
});

fn lock() -> std::sync::MutexGuard<'static, State> {
    STATE.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn in_dispatcher_thread() -> bool {
    IN_DISPATCHER.with(|c| c.get())
}

/// 启动 dispatcher 线程（幂等：已启动则复用）。
pub fn start(handler: Box<dyn Fn(Event) + Send>) {
    let mut g = lock();
    if g.tx.is_some() {
        return;
    }
    let (tx, rx) = unbounded_channel::<Event>();
    let join = std::thread::Builder::new()
        .name("slive-dispatcher".into())
        .spawn(move || run(rx, handler))
        .expect("dispatcher thread");
    g.tx = Some(tx);
    g.join = Some(join);
}

fn run(mut rx: UnboundedReceiver<Event>, handler: Box<dyn Fn(Event) + Send>) {
    IN_DISPATCHER.with(|c| c.set(true));
    while let Some(ev) = rx.blocking_recv() {
        // 接收端借用已在此释放（ev 是拥有所有权的值），回调内可安全重入
        handler(ev);
    }
    IN_DISPATCHER.with(|c| c.set(false));
}

pub fn send(ev: Event) {
    // 只克隆发送端，锁内不做任何回调
    let tx = lock().tx.clone();
    if let Some(tx) = tx {
        let _ = tx.send(ev);
    }
}

/// 停止 dispatcher：关闭通道（线程随之退出）并 join。可在任意非 dispatcher 线程调用。
///
/// ⚠️ **不可在 dispatcher 线程内 join 自己**（会永久死锁）。本函数检测到当前就在
/// dispatcher 线程时，只关闭通道而不 join —— 线程会在当前回调返回后自行退出。
pub fn stop() {
    let (tx, join) = {
        let mut g = lock();
        (g.tx.take(), g.join.take())
    };
    // 先丢弃发送端：run 的 blocking_recv 返回 None，循环退出
    drop(tx);
    if !in_dispatcher_thread() {
        if let Some(join) = join {
            let _ = join.join();
        }
    }
    // 在 dispatcher 线程内时丢弃 JoinHandle：线程会在当前回调返回后退出，
    // 不 join（避免自我等待）。丢弃 handle 不会泄漏线程资源，线程结束即回收。
}
