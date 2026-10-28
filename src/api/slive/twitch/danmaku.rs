//! Twitch 弹幕 FRB 桥：生产 IRC-over-WebSocket 连接注入核心 `run_twitch_session`。
//!
//! 桥层只做**连接装配与事件转发**；URL / 握手帧 / IRC 解析 / 心跳 / 重连策略全部来自
//! `slive-core::platforms::twitch`（AC-39 / 裁决 J-6）。
//!
//! ⚠️ **FRB 的 FFI 调用线程没有 tokio runtime** —— 在那里 `tokio::spawn` 会 panic
//! （"there is no reactor running"，B站真机踩过）。本文件沿用同一修法：
//! **独立线程 + 自建 current_thread runtime**。
//!
//! 与 B站弹幕桥（`../danmaku.rs`）**完全独立命名**：本文件不引用、不修改其任何符号
//! （`DanmakuConnection` 为 B站私有；本侧是 `TwitchDanmakuConnection`）。

use flutter_rust_bridge::frb;
use tokio::sync::watch;

use crate::api::slive::error::CoreError;
use crate::api::slive::models::LiveDanmuItem;
use crate::frb_generated::StreamSink;

/// Twitch 弹幕连接句柄（FRB opaque）：**每实例一份**，持有本会话的停止信号。
///
/// 2026-10-18 修复多开异常：旧实现用全局单例 `twitch_active_stop` 登记「当前会话」，
/// 启动第二路会先把第一路的 stop 置 `true` —— 多开时只有最后启动的那路有弹幕，
/// 其余路聊天区显示「已手动停止」。改为句柄自持 stop 后各停各的、互不影响；
/// 句柄被释放（Dart 侧丢弃且未显式 stop）时发送端 drop，会话随句柄一起结束。
#[frb(opaque)]
pub struct TwitchDanmakuConnection {
    stop_tx: watch::Sender<bool>,
}

impl TwitchDanmakuConnection {
    /// 构造连接句柄（尚未连接；`start` 之后才持有会话）。
    #[frb(sync)]
    pub fn new() -> Self {
        let (stop_tx, _stop_rx) = watch::channel(false);
        Self { stop_tx }
    }

    /// 启动 Twitch 弹幕会话：连接 IRC over WebSocket，事件推入 stream。
    ///
    /// `channel_or_args` 是 `LiveRoomDetail.danmakuData` 的原样透参。
    /// ⚠️ **照搬 Dart**（`twitch_site.dart:241` + `twitch_danmaku.dart:56`）：Twitch 的
    /// `danmakuData` 就是 **roomId 字符串**（QA 夹具 `gql_twitch_tv_room_shell` 已钉死
    /// `expected.danmaku_data == "alice"`），**不带** `schema_version` 的 JSON 包装 ——
    /// Twitch 弹幕唯一入参就是频道名，无需版本化契约。
    /// 为兼容两种形态（裸频道名 / JSON 契约串），这里**显式**先按裸频道名解析，
    /// 失败再回退契约串；两者都非法 -> `CoreError(InvalidInput)`（**不静默兜底**，J-10 精神）。
    /// 多路播放各自持句柄调用，会话之间互不停止。
    pub fn start(&self, channel_or_args: String, sink: StreamSink<LiveDanmuItem>) -> Result<(), CoreError> {
        slive_host::logging::ensure_stderr_logger();
        slive_host::danmaku::spawn(
            slive_core::platforms::Platform::Twitch,
            &channel_or_args,
            self.stop_tx.subscribe(),
            move |item| {
                let _ = sink.add(item);
            },
        )
        .map_err(CoreError::from)
    }

    /// 手动停止本会话（对齐 Dart `TwitchDanmaku.stop()`：发出 `Closed`，不再重连）。
    #[frb(sync)]
    pub fn stop(&self) {
        let _ = self.stop_tx.send(true);
    }
}
