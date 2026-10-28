//! 头文件一致性：不用 cbindgen —— 用 `include_str!` 断言常量与 Rust 侧逐字一致。

use slive_ffi::methods::*;

const HEADER: &str = include_str!("../include/slive.h");

#[test]
fn abi_version_matches_header() {
    let expected = format!("{:#010x}", slive_ffi::slive_abi_version());
    assert!(
        HEADER.contains(&expected),
        "头文件的 SLIVE_ABI_VERSION 与 Rust 不一致（期望 {expected}）"
    );
}

#[test]
fn every_method_constant_appears_in_header() {
    for m in ALL_METHODS {
        assert!(HEADER.contains(m), "头文件缺少方法常量 {m}");
    }
}

#[test]
fn error_codes_match_header() {
    for name in [
        "SLIVE_ERR_INVALID_HANDLE",
        "SLIVE_ERR_INVALID_ARG",
        "SLIVE_ERR_INVALID_UTF8",
        "SLIVE_ERR_NOT_INIT",
        "SLIVE_ERR_BUSY",
        "SLIVE_ERR_PANIC",
        "SLIVE_ERR_NOT_FOUND",
        "SLIVE_ERR_CANCELLED",
        "SLIVE_ERR_UNSUPPORTED",
    ] {
        assert!(HEADER.contains(&format!("#define {name}")), "头文件缺少 {name}");
        assert!(
            slive_ffi::abi::code_by_name(name).is_some(),
            "Rust 侧缺少 {name} 的码值映射"
        );
    }
}

#[test]
fn danmaku_event_kinds_match_header() {
    for (name, value) in [
        ("SLIVE_DANMAKU_READY", slive_ffi::danmaku::DANMAKU_READY),
        ("SLIVE_DANMAKU_RECONNECTING", slive_ffi::danmaku::DANMAKU_RECONNECTING),
        ("SLIVE_DANMAKU_CLOSED", slive_ffi::danmaku::DANMAKU_CLOSED),
        ("SLIVE_DANMAKU_MESSAGE", slive_ffi::danmaku::DANMAKU_MESSAGE),
    ] {
        assert!(HEADER.contains(&format!("#define {name}")), "头文件缺少 {name}");
        assert_eq!(
            value,
            match name {
                "SLIVE_DANMAKU_READY" => 0,
                "SLIVE_DANMAKU_RECONNECTING" => 1,
                "SLIVE_DANMAKU_CLOSED" => 2,
                _ => 3,
            }
        );
    }
}

#[test]
fn envelope_schema_version_is_1() {
    assert_eq!(slive_ffi::envelope::SCHEMA_VERSION, 1);
    assert!(HEADER.contains("#define SLIVE_SCHEMA_VERSION 1u"));
}
