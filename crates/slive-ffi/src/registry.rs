//! 句柄用 u64 而非裸指针：悬垂使用变成查表失败（返回 INVALID_HANDLE），不是 UB；free 幂等。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use slive_core::platforms::LiveSite;

pub enum Entry {
    Site(Arc<dyn LiveSite>),
    Danmaku(DanmakuHandle),
    Mask(Arc<Mutex<slive_core::danmaku::mask::DanmakuMask>>),
}

pub struct DanmakuHandle {
    pub stop_tx: tokio::sync::watch::Sender<bool>,
    pub platform: slive_core::platforms::Platform,
}

struct Registry {
    next: u64,
    entries: HashMap<u64, Entry>,
}

fn registry() -> &'static Mutex<Registry> {
    static R: OnceLock<Mutex<Registry>> = OnceLock::new();
    R.get_or_init(|| {
        Mutex::new(Registry {
            next: 1,
            entries: HashMap::new(),
        })
    })
}

fn lock() -> std::sync::MutexGuard<'static, Registry> {
    // 绝不 unwrap：poisoned 也继续（panic 已在上层被 catch_unwind 记录）
    registry().lock().unwrap_or_else(|e| e.into_inner())
}

pub fn insert(entry: Entry) -> u64 {
    let mut g = lock();
    let id = g.next;
    g.next += 1;
    g.entries.insert(id, entry);
    id
}

pub fn remove(id: u64) -> Option<Entry> {
    lock().entries.remove(&id)
}

/// 在锁内取条目执行闭包（唯一访问方式）。
///
/// ⚠️ **闭包内禁止调用任何宿主回调** —— 回调必须在释放锁之后执行，
/// 否则宿主在回调里再调 `slive_*` 会自死锁。故这里只做「取出快照」。
pub fn with_entry<R>(id: u64, f: impl FnOnce(&Entry) -> R) -> Option<R> {
    let g = lock();
    g.entries.get(&id).map(f)
}

/// 取出站点的 `Arc` 克隆（锁外使用，不受注册表生命周期约束）。
pub fn clone_site(id: u64) -> Option<Arc<dyn LiveSite>> {
    with_entry(id, |e| match e {
        Entry::Site(s) => Some(Arc::clone(s)),
        _ => None,
    })
    .flatten()
}

#[cfg(test)]
pub fn len() -> usize {
    lock().entries.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn double_free_is_idempotent() {
        let mask = slive_core::danmaku::mask::DanmakuMask::new(1000, 4, true, true, 2);
        let id = insert(Entry::Mask(Arc::new(Mutex::new(mask))));
        assert!(remove(id).is_some());
        assert!(remove(id).is_none(), "第二次 free 应返回 None 而非 panic");
    }

    #[test]
    fn unknown_handle_is_none() {
        assert!(with_entry(999_999, |_| ()).is_none());
    }
}
