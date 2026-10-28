//! 全局多线程 runtime：C ABI 的异步调用都在这里跑。
//!
//! `init` 幂等；`shutdown` 丢弃 runtime（worker 线程随之关闭，未完成任务被取消）。

use std::sync::Mutex;

use tokio::runtime::{Handle, Runtime};

/// runtime 与其句柄同处一个锁 —— 避免两把锁的顺序依赖（init/shutdown 路径相反会构成死锁环）。
struct Slot {
    rt: Option<Runtime>,
    handle: Option<Handle>,
}

static SLOT: Mutex<Slot> = Mutex::new(Slot {
    rt: None,
    handle: None,
});

fn lock() -> std::sync::MutexGuard<'static, Slot> {
    SLOT.lock().unwrap_or_else(|e| e.into_inner())
}

/// 初始化全局 runtime（幂等）。
pub fn init(worker_threads: u32) -> Result<(), String> {
    let mut g = lock();
    if g.rt.is_some() {
        return Ok(());
    }
    let mut builder = tokio::runtime::Builder::new_multi_thread();
    builder.enable_all();
    if worker_threads > 0 {
        builder.worker_threads(worker_threads as usize);
    }
    let rt = builder.build().map_err(|e| e.to_string())?;
    g.handle = Some(rt.handle().clone());
    g.rt = Some(rt);
    Ok(())
}

/// 取 runtime 句柄的克隆（`Handle` 是引用计数，调用方持有期间 runtime 不会被提前释放）。
pub fn handle() -> Option<Handle> {
    lock().handle.clone()
}

/// 关闭 runtime：丢弃句柄与 runtime，worker 线程随之退出。
pub fn shutdown() {
    let rt = {
        let mut g = lock();
        g.handle = None;
        g.rt.take()
    };
    if let Some(rt) = rt {
        // 在 runtime 线程内 drop 会 panic（"Cannot drop a runtime in a context where blocking is not allowed"），
        // 故放到独立线程执行；不 join，避免 shutdown 被长时间阻塞。
        std::thread::Builder::new()
            .name("slive-rt-teardown".into())
            .spawn(move || drop(rt))
            .expect("runtime teardown thread");
    }
}

#[cfg(test)]
pub fn is_running() -> bool {
    lock().rt.is_some()
}
