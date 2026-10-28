//! ABI 契约测试：直接以普通 Rust 调用 `extern "C"` 函数（`rlib` 链接），无需 C 编译器。

use slive_ffi::abi::*;
use slive_ffi::danmaku::*;
use slive_ffi::mask::*;
use slive_ffi::site::*;
use slive_ffi::*;

fn s(text: &str) -> SliveStr {
    to_slive_str(text)
}

unsafe fn take(v: SliveStr) -> String {
    let out = from_slive_str(v).unwrap();
    free_slive_str(v);
    out
}

#[test]
fn init_then_create_site_and_read_names() {
    assert_eq!(slive_init(std::ptr::null()), SLIVE_OK);
    let mut h: SliveHandle = 0;
    assert_eq!(slive_site_create(0, &mut h), SLIVE_OK);
    assert_ne!(h, 0);

    let mut id = SliveStr::EMPTY;
    assert_eq!(slive_site_id(h, &mut id), SLIVE_OK);
    assert_eq!(unsafe { take(id) }, "bilibili");

    let mut name = SliveStr::EMPTY;
    assert_eq!(slive_site_name(h, &mut name), SLIVE_OK);
    assert_eq!(unsafe { take(name) }, "哔哩哔哩直播");

    assert_eq!(slive_site_free(h), SLIVE_OK);
    assert_eq!(slive_site_free(h), SLIVE_ERR_INVALID_HANDLE, "二次 free 应报无效句柄");
}

#[test]
fn invalid_platform_is_rejected() {
    let mut h: SliveHandle = 0;
    assert_eq!(slive_site_create(99, &mut h), SLIVE_ERR_INVALID_ARG);
}

#[test]
fn invalid_utf8_argument_is_rejected() {
    let bad = [0xffu8, 0xfe];
    let s = SliveStr {
        ptr: bad.as_ptr() as *const std::os::raw::c_char,
        len: bad.len(),
    };
    assert_eq!(slive_site_set_attrs(12345, s), SLIVE_ERR_INVALID_UTF8);
}

#[test]
fn unknown_method_is_unsupported() {
    let mut h: SliveHandle = 0;
    slive_site_create(0, &mut h);
    let r = slive_site_call(h, s("nope"), s("{}"), 1);
    assert_eq!(r, SLIVE_ERR_UNSUPPORTED);
    slive_site_free(h);
}

#[test]
fn mask_allow_and_merge_work() {
    let mut h: SliveHandle = 0;
    assert_eq!(slive_mask_create(10_000, 10, 1, 1, 3, &mut h), SLIVE_OK);

    let mut out = SliveStr::EMPTY;
    assert_eq!(
        slive_mask_allow(h, s(r#"["666","666","777"]"#), 1000, &mut out),
        SLIVE_OK
    );
    assert_eq!(unsafe { take(out) }, "[1,1,1]");

    let mut out2 = SliveStr::EMPTY;
    assert_eq!(
        slive_mask_merge(h, s(r#"["666","777","666"]"#), 2000, &mut out2),
        SLIVE_OK
    );
    let json: serde_json::Value = serde_json::from_str(&unsafe { take(out2) }).unwrap();
    assert_eq!(json[0]["key"], "666");
    // 首轮 allow 已计入 2 次 666（max_frequency=3，均放行），本轮 merge 再加 2 次
    assert_eq!(json[0]["window_count"], 4, "2（首轮）+ 2（本轮）");

    assert_eq!(slive_mask_free(h), SLIVE_OK);
}

#[test]
fn danmaku_create_requires_valid_platform() {
    let mut h: SliveHandle = 0;
    assert_eq!(slive_danmaku_create(99, &mut h), SLIVE_ERR_INVALID_ARG);
    assert_eq!(slive_danmaku_create(0, &mut h), SLIVE_OK);
    assert_eq!(slive_danmaku_stop(h), SLIVE_OK);
    assert_eq!(slive_danmaku_free(h), SLIVE_OK);
}
