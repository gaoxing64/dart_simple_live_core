//! stderr logger：供宿主在启动时安装一次。
//!
//! 核心 crate 用 `log::` 宏输出诊断，但**从不自行安装 logger** —— 由宿主决定输出到哪里。
//! 这里的实现保持与原 `eprintln!` 相同的 `[前缀]` 格式，保证既有可观测性不退化。
//!
//! 级别策略：**我们自己的** crate（`slive_core` / `slive_host`）放开到 Debug；
//! 第三方库（reqwest / rustls / hyper …）只保留 Warn 以上 —— 否则 TLS 握手的
//! 内部细节会刷满屏，淹没真正有用的诊断信息。

use std::sync::Once;

/// 目标 crate 名前缀：这些 crate 的日志放开到 Debug。
const OWN_CRATES: [&str; 2] = ["slive_core", "slive_host"];

/// 安装 stderr logger（幂等）。
pub fn ensure_stderr_logger() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        struct StderrLogger;
        impl log::Log for StderrLogger {
            fn enabled(&self, metadata: &log::Metadata) -> bool {
                // 第三方库只放 Warn 及以上；自有 crate 全放行。
                if OWN_CRATES
                    .iter()
                    .any(|c| metadata.target().starts_with(c))
                {
                    true
                } else {
                    metadata.level() <= log::Level::Warn
                }
            }

            fn log(&self, record: &log::Record) {
                if !self.enabled(record.metadata()) {
                    return;
                }
                eprintln!(
                    "[danmaku-rs] {} {}: {}",
                    record.level(),
                    record.target(),
                    record.args()
                );
            }

            fn flush(&self) {}
        }
        let _ = log::set_logger(&StderrLogger);
        // 全局上限保持 Trace，让 `enabled` 的 per-target 过滤真正起作用。
        log::set_max_level(log::LevelFilter::Trace);
    });
}
