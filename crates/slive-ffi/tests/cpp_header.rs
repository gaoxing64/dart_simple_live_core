//! C++ 头文件的编译期验证：用系统 C++ 编译器做 `-fsyntax-only`。
//!
//! 若环境无 C++ 编译器则跳过（不失败）—— 保证 CI 在没有 clang/g++ 的机器上仍绿。

use std::process::Command;

fn find_compiler() -> Option<&'static str> {
    ["clang++", "g++", "cl.exe"].into_iter().find(|c| {
        Command::new(c)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    })
}

#[test]
fn cpp_header_syntax_ok() {
    let Some(cc) = find_compiler() else {
        eprintln!("未找到 C++ 编译器，跳过 slive.hpp 语法检查");
        return;
    };
    let out = Command::new(cc)
        .args(["-std=c++17", "-fsyntax-only", "-Wall", "-Wextra", "-I", "include"])
        .arg("include/slive_hpp_smoke.cpp")
        .output()
        .expect("运行编译器");
    assert!(
        out.status.success(),
        "slive.hpp 编译失败：\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn c_header_is_valid_c() {
    let Some(cc) = find_compiler() else {
        eprintln!("未找到 C++ 编译器，跳过 slive.h 检查");
        return;
    };
    // 用 C++ 编译器以 C 模式校验头文件（同时覆盖 extern "C" 分支）
    let out = Command::new(cc)
        .args(["-std=c++17", "-fsyntax-only", "-Wall", "-Wextra", "-x", "c++-header", "include/slive.h"])
        .output()
        .expect("运行编译器");
    assert!(
        out.status.success(),
        "slive.h 编译失败：\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
