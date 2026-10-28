//! Simple Live 宿主层：把「真实网络连接」与「会话装配」从核心 crate 分离出来。
//!
//! 分层（ADR-001 / ADR-016）：
//! - `slive-core` 只做协议与纯逻辑，不持有任何连接；
//! - 本 crate 提供 tokio-tungstenite / 明文 TCP 的真实帧源，以及五平台弹幕会话装配；
//! - CLI、FRB 桥、C ABI 三者共用本 crate，消除原先各自复制一份传输实现的重复。

pub mod danmaku;
pub mod logging;
pub mod runtime;
pub mod tcp;
pub mod ws;
