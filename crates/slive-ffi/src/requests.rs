//! 在途请求登记表：支撑 `slive_request_cancel` 与「恰好一次 completion」保证。
//!
//! 每个已受理的请求登记一个 `watch` 发送端；取消 = 置 `true`，任务侧 `select!` 抢先返回。
//! 任务结束时调用 [`finish`] 摘除登记 —— 重复 ID 返回 `SLIVE_ERR_BUSY`。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use tokio::sync::watch;

type Inflight = HashMap<u64, watch::Sender<bool>>;

fn inflight() -> &'static Mutex<Inflight> {
    static R: OnceLock<Mutex<Inflight>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock() -> std::sync::MutexGuard<'static, Inflight> {
    inflight().lock().unwrap_or_else(|e| e.into_inner())
}

/// 登记新请求。ID 已存在（在途）→ `None`。
pub fn register(id: u64) -> Option<watch::Receiver<bool>> {
    let mut g = lock();
    if g.contains_key(&id) {
        return None;
    }
    let (tx, rx) = watch::channel(false);
    g.insert(id, tx);
    Some(rx)
}

/// 摘除登记（任务结束或请求被拒绝时调用）。幂等。
pub fn finish(id: u64) {
    lock().remove(&id);
}

/// 取消：置 `true`。不存在 → `false`。
pub fn cancel(id: u64) -> bool {
    match lock().get(&id) {
        Some(tx) => {
            let _ = tx.send(true);
            true
        }
        None => false,
    }
}

/// 关闭全部在途请求（shutdown 时调用）。
pub fn cancel_all() {
    let mut g = lock();
    for tx in g.values() {
        let _ = tx.send(true);
    }
    g.clear();
}

/// 当前在途请求数（供测试与诊断）。
pub fn inflight_count() -> usize {
    lock().len()
}
