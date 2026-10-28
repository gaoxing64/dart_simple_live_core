//! F-11 虎牙弹幕会话桥：生产 WS 连接（tokio-tungstenite + rustls）注入核心 `run_session`。
//!
//! 与 B站桥（`crate::api::slive::danmaku`）**并列独立**：本文件只服务虎牙，
//! 不修改 B站的 `DanmakuConnection`（共享文件接线由总监统一处理）。
//!
//! 桥层只做连接装配与事件转发；URL / headers / 帧编解码 / 重连策略全部来自
//! `slive-core::platforms::huya`（AC-39）。

use flutter_rust_bridge::frb;
use tokio::sync::watch;

use crate::api::slive::error::CoreError;
use crate::api::slive::models::LiveDanmuItem;
use crate::frb_generated::StreamSink;

/// 句柄被释放（Dart 侧丢弃且未显式 stop）时发送端 drop，会话随句柄一起结束。
#[frb(opaque)]
pub struct HuyaDanmakuConnection {
    stop_tx: watch::Sender<bool>,
}

impl HuyaDanmakuConnection {
    /// 构造连接句柄（尚未连接；`start` 之后才持有会话）。
    #[frb(sync)]
    pub fn new() -> Self {
        let (stop_tx, _stop_rx) = watch::channel(false);
        Self { stop_tx }
    }

    /// 启动虎牙弹幕会话（F-11）。
    ///
    /// `args_json` 是 [`HuyaDanmakuArgs`] 的 JSON 串（`danmaku_data` 通道）；解析失败 ->
    /// `CoreError(Decode)`，对应 Dart 侧 `FormatException`（J-10）。
    /// 多路播放各自持句柄调用，会话之间互不停止。
    pub fn start(&self, args_json: String, sink: StreamSink<LiveDanmuItem>) -> Result<(), CoreError> {
        slive_host::logging::ensure_stderr_logger();
        slive_host::danmaku::spawn(
            slive_core::platforms::Platform::Huya,
            &args_json,
            self.stop_tx.subscribe(),
            move |item| {
                let _ = sink.add(item);
            },
        )
        .map_err(CoreError::from)
    }
    #[frb(sync)]
    pub fn stop(&self) {
        let _ = self.stop_tx.send(true);
    }
}
