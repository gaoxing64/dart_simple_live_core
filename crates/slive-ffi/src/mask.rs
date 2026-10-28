//! 弹幕去重/合并 C ABI。

use std::os::raw::c_int;
use std::sync::{Arc, Mutex};

use crate::abi::*;
use crate::registry::{self, Entry};

#[no_mangle]
pub extern "C" fn slive_mask_create(
    base_window_ms: u32,
    bucket_count: u16,
    use_normalization: u8,
    use_frequency_control: u8,
    max_frequency: u16,
    out: *mut SliveHandle,
) -> c_int {
    guard(|| {
        if out.is_null() {
            return SLIVE_ERR_INVALID_ARG;
        }
        let mask = slive_core::danmaku::mask::DanmakuMask::new(
            base_window_ms,
            bucket_count,
            use_normalization != 0,
            use_frequency_control != 0,
            max_frequency,
        );
        let id = registry::insert(Entry::Mask(Arc::new(Mutex::new(mask))));
        unsafe { *out = id };
        SLIVE_OK
    })
}

fn with_mask<R>(
    handle: SliveHandle,
    f: impl FnOnce(&mut slive_core::danmaku::mask::DanmakuMask) -> R,
) -> Option<R> {
    // 先克隆 Arc（释放注册表锁），再取 mask 锁 —— 避免两层锁嵌套，
    // 也避免在持有注册表锁时执行耗时逻辑。
    let arc = registry::with_entry(handle, |e| match e {
        Entry::Mask(m) => Some(Arc::clone(m)),
        _ => None,
    })
    .flatten()?;
    let mut g = arc.lock().unwrap_or_else(|p| p.into_inner());
    Some(f(&mut g))
}

fn mask_batch(handle: SliveHandle, texts_json: SliveStr, now_ms: u64, out_json: *mut SliveStr, merge: bool) -> c_int {
    guard(|| {
        if out_json.is_null() {
            return SLIVE_ERR_INVALID_ARG;
        }
        let Ok(raw) = (unsafe { from_slive_str(texts_json) }) else {
            return SLIVE_ERR_INVALID_UTF8;
        };
        let texts: Vec<String> = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(_) => {
                return SLIVE_ERR_INVALID_ARG;
            }
        };
        let result = with_mask(handle, |m| {
            if merge {
                serde_json::to_string(&m.merge_list_batch(texts, now_ms))
            } else {
                serde_json::to_string(&m.allow_list_batch(texts, now_ms))
            }
        });
        match result {
            Some(Ok(json)) => {
                unsafe { *out_json = to_slive_str(&json) };
                SLIVE_OK
            }
            Some(Err(_)) => SLIVE_ERR_PANIC, // 序列化失败属内部错误，非入参问题
            None => SLIVE_ERR_INVALID_HANDLE,
        }
    })
}

#[no_mangle]
pub extern "C" fn slive_mask_allow(
    handle: SliveHandle,
    texts_json: SliveStr,
    now_ms: u64,
    out_json: *mut SliveStr,
) -> c_int {
    mask_batch(handle, texts_json, now_ms, out_json, false)
}

#[no_mangle]
pub extern "C" fn slive_mask_merge(
    handle: SliveHandle,
    texts_json: SliveStr,
    now_ms: u64,
    out_json: *mut SliveStr,
) -> c_int {
    mask_batch(handle, texts_json, now_ms, out_json, true)
}

#[no_mangle]
pub extern "C" fn slive_mask_reset(handle: SliveHandle) -> c_int {
    guard(|| match with_mask(handle, |m| m.reset()) {
        Some(()) => SLIVE_OK,
        None => SLIVE_ERR_INVALID_HANDLE,
    })
}

#[no_mangle]
pub extern "C" fn slive_mask_free(handle: SliveHandle) -> c_int {
    guard(|| {
        // 先确认类型再摘除：remove + 回插会让句柄换号
        let is_mask = registry::with_entry(handle, |e| matches!(e, Entry::Mask(_))).unwrap_or(false);
        if !is_mask {
            return SLIVE_ERR_INVALID_HANDLE;
        }
        registry::remove(handle);
        SLIVE_OK
    })
}
