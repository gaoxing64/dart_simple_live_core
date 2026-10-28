//! C ABI 边界：字符串、返回码、panic 守卫。跨边界只允许 C 基础类型。

use std::os::raw::c_char;

/// 非 NUL 结尾的 UTF-8 串（日志可能含 `\0`，故不用 CString）。
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct SliveStr {
    pub ptr: *const c_char,
    pub len: usize,
}

impl SliveStr {
    pub const EMPTY: SliveStr = SliveStr {
        ptr: std::ptr::null(),
        len: 0,
    };
}

/// 句柄：0 = 无效。用 u64 而非裸指针 —— 悬垂使用变成查表失败，不是 UB。
pub type SliveHandle = u64;

// 返回码（负数 = ABI 层错误；1 = core 业务错误已封装进 envelope）
pub const SLIVE_OK: i32 = 0;
pub const SLIVE_ERR_CORE: i32 = 1;
pub const SLIVE_ERR_INVALID_HANDLE: i32 = -1;
pub const SLIVE_ERR_INVALID_ARG: i32 = -2;
pub const SLIVE_ERR_INVALID_UTF8: i32 = -3;
pub const SLIVE_ERR_NOT_INIT: i32 = -4;
pub const SLIVE_ERR_BUSY: i32 = -5;
pub const SLIVE_ERR_PANIC: i32 = -6;
pub const SLIVE_ERR_NOT_FOUND: i32 = -7;
pub const SLIVE_ERR_CANCELLED: i32 = -8;
pub const SLIVE_ERR_UNSUPPORTED: i32 = -9;

/// 按名字查返回码（供头文件一致性测试比对）。
pub fn code_by_name(name: &str) -> Option<i32> {
    Some(match name {
        "SLIVE_ERR_INVALID_HANDLE" => SLIVE_ERR_INVALID_HANDLE,
        "SLIVE_ERR_INVALID_ARG" => SLIVE_ERR_INVALID_ARG,
        "SLIVE_ERR_INVALID_UTF8" => SLIVE_ERR_INVALID_UTF8,
        "SLIVE_ERR_NOT_INIT" => SLIVE_ERR_NOT_INIT,
        "SLIVE_ERR_BUSY" => SLIVE_ERR_BUSY,
        "SLIVE_ERR_PANIC" => SLIVE_ERR_PANIC,
        "SLIVE_ERR_NOT_FOUND" => SLIVE_ERR_NOT_FOUND,
        "SLIVE_ERR_CANCELLED" => SLIVE_ERR_CANCELLED,
        "SLIVE_ERR_UNSUPPORTED" => SLIVE_ERR_UNSUPPORTED,
        _ => return None,
    })
}

/// 把 `&str` 转成需由调用方 `slive_str_free` 释放的 `SliveStr`。
///
/// 空串返回 `EMPTY`（null+0），不分配 —— 与 [`free_slive_str`] 对 `len == 0` 直接返回保持一致，
/// 避免「分配了但 free 时跳过」的泄漏。
pub fn to_slive_str(s: &str) -> SliveStr {
    if s.is_empty() {
        return SliveStr::EMPTY;
    }
    let boxed = s.as_bytes().to_vec().into_boxed_slice();
    let len = boxed.len();
    SliveStr {
        ptr: Box::into_raw(boxed) as *const c_char,
        len,
    }
}

/// # Safety
///
/// 释放由 [`to_slive_str`] 分配的字符串。
///
/// - `s.ptr` 必须来自 [`to_slive_str`]（或为 null / `len == 0`）；
/// - 只能释放一次 —— 二次释放是 double free（UB）；
/// - **必须调用本库导出的 `slive_str_free`**，不可用宿主自己的 `free()`：
///   内存由 Rust 分配器分配，跨分配器释放会在 CRT 不一致时损坏堆
///   （Windows 上 Rust 与宿主可能链到不同的 `msvcrt`）。
pub unsafe fn free_slive_str(s: SliveStr) {
    if s.ptr.is_null() || s.len == 0 {
        return;
    }
    let slice = std::slice::from_raw_parts_mut(s.ptr as *mut u8, s.len);
    drop(Box::from_raw(slice as *mut [u8]));
}

/// # Safety
///
/// 把宿主传入的字符串读入 Rust。
///
/// - `s.ptr` 必须是**非空且可读至少 `s.len` 字节**的已初始化内存；
/// - `s.len` 不得超过实际分配的长度 —— 越界读是 UB，无法被校验；
/// - 字节内容必须是合法 UTF-8，否则返回 [`SLIVE_ERR_INVALID_UTF8`]（不丢字节、不做 lossy 转换）。
///
/// 指针为 null 且 `s.len == 0` 视为空串；指针为 null 而 `s.len > 0` 是非法参数。
pub unsafe fn from_slive_str(s: SliveStr) -> Result<String, i32> {
    if s.ptr.is_null() {
        return if s.len == 0 {
            Ok(String::new())
        } else {
            Err(SLIVE_ERR_INVALID_ARG)
        };
    }
    let bytes = std::slice::from_raw_parts(s.ptr as *const u8, s.len);
    std::str::from_utf8(bytes)
        .map(|v| v.to_owned())
        .map_err(|_| SLIVE_ERR_INVALID_UTF8)
}

/// 所有 `extern "C"` 函数体必须包一层：unwinding 跨 FFI 是 UB。
pub fn guard<F: FnOnce() -> i32>(f: F) -> i32 {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).unwrap_or(SLIVE_ERR_PANIC)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn str_roundtrip_and_free() {
        let s = to_slive_str("hello 世界");
        assert_eq!(unsafe { from_slive_str(s) }.unwrap(), "hello 世界");
        unsafe { free_slive_str(s) };
    }

    #[test]
    fn invalid_utf8_is_rejected() {
        let bad = [0xffu8, 0xfe];
        let s = SliveStr {
            ptr: bad.as_ptr() as *const c_char,
            len: bad.len(),
        };
        assert_eq!(unsafe { from_slive_str(s) }, Err(SLIVE_ERR_INVALID_UTF8));
    }

    #[test]
    fn empty_str_is_empty_not_error() {
        assert_eq!(unsafe { from_slive_str(SliveStr::EMPTY) }.unwrap(), "");
    }

    #[test]
    fn guard_catches_panic() {
        assert_eq!(guard(|| panic!("boom")), SLIVE_ERR_PANIC);
    }

    #[test]
    fn code_lookup_matches_constants() {
        assert_eq!(code_by_name("SLIVE_ERR_CANCELLED"), Some(-8));
        assert_eq!(code_by_name("NOPE"), None);
    }
}
