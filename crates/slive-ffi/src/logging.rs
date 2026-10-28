//! `log` → dispatcher 转发：宿主通过 `on_log` 回调接收 Rust 侧诊断。

use crate::dispatcher::{self, Event};

struct FfiLogger;

impl log::Log for FfiLogger {
    fn enabled(&self, _: &log::Metadata) -> bool {
        true
    }

    fn log(&self, record: &log::Record) {
        dispatcher::send(Event::Log {
            level: record.level() as i32,
            target: record.target().to_string(),
            message: record.args().to_string(),
        });
    }

    fn flush(&self) {}
}

static LOGGER: FfiLogger = FfiLogger;

/// 安装转发 logger（幂等；失败无副作用）。
pub fn install() {
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Debug);
}
