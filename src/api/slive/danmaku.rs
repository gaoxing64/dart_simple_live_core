//! F-11 弹幕会话桥：生产 WS 连接（tokio-tungstenite + rustls）注入核心 `run_session`。
//!
//! 桥层只做连接装配与事件转发；URL / headers / 编解码 / 重连策略全部来自 `slive-core`（AC-39）。

use flutter_rust_bridge::frb;
use tokio::sync::watch;

use crate::api::slive::error::CoreError;
use crate::api::slive::models::LiveDanmuItem;
use crate::frb_generated::StreamSink;

pub use slive_host::ws::connect_ws_frame_source;

/// 弹幕会话句柄（FRB opaque）：**每实例一份**，持有本会话的停止信号。
///
/// 2026-10-18 修复多开异常：旧实现用全局单例 `active_stop` 登记「当前会话」，
/// 启动第二路会先把第一路的 stop 置 `true` —— 多开时只有最后启动的那路有弹幕，
/// 其余路聊天区显示「已手动停止」。改为句柄自持 stop 后各停各的、互不影响；
/// 句柄被释放（Dart 侧丢弃且未显式 stop）时发送端 drop，会话随句柄一起结束。
#[frb(opaque)]
pub struct DanmakuConnection {
    stop_tx: watch::Sender<bool>,
}

impl DanmakuConnection {
    /// 构造连接句柄（尚未连接；`start` 之后才持有会话）。
    #[frb(sync)]
    pub fn new() -> Self {
        let (stop_tx, _stop_rx) = watch::channel(false);
        Self { stop_tx }
    }

    /// 启动弹幕会话（F-11）：连接 `args` 指定的 WS 地址，事件推入 stream。
    ///
    /// `args_json` 是 [`BiliBiliDanmakuArgs`] 的 JSON 串（J-2 契约）；解析失败 →
    /// `CoreError(Decode)`，对应 Dart 侧 `FormatException`（J-10）。
    /// 多路播放各自持句柄调用，会话之间互不停止。
    pub fn start(&self, args_json: String, sink: StreamSink<LiveDanmuItem>) -> Result<(), CoreError> {
        slive_host::logging::ensure_stderr_logger();
        slive_host::danmaku::spawn(
            slive_core::platforms::Platform::Bilibili,
            &args_json,
            self.stop_tx.subscribe(),
            move |item| {
                let _ = sink.add(item);
            },
        )
        .map_err(CoreError::from)
    }

    /// 手动停止（对齐 Dart `LiveDanmaku.stop()`：发出 `Closed`，不再重连）。
    #[frb(sync)]
    pub fn stop(&self) {
        let _ = self.stop_tx.send(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 多开回归（2026-10-18）：每个句柄的 stop 通道必须互相独立 ——
    /// 旧全局单例在这里会表现为「停 A 把 B 也停掉」。
    #[test]
    fn stop_channels_are_per_connection() {
        let a = DanmakuConnection::new();
        let b = DanmakuConnection::new();
        let mut rx_a = a.stop_tx.subscribe();

        a.stop();

        assert!(*rx_a.borrow_and_update(), "停 A 只应影响 A 自己");
        assert!(!*b.stop_tx.subscribe().borrow(), "B 的停止信号不得被 A 的 stop 触发");
    }
}
