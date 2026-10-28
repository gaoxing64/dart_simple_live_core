//! 五平台弹幕会话装配：入参解析 → URL/头 → 连接工厂 → 会话主循环。
//!
//! 这里只做「接线」，零解析逻辑（帧编解码 / 心跳 / 重连全在 `slive-core`）。
//! 原先在 `crates/slive-cli/src/danmaku.rs` 与 `src/api/slive/*.rs` 各写一遍，此处收敛为一份。

use std::sync::Arc;

use tokio::sync::watch;

use slive_core::error::SliveError;
use slive_core::models::LiveDanmuItem;
use slive_core::platforms::Platform;
use slive_core::time::SystemClock;
use slive_core::ws::SessionTuning;

use crate::{runtime, ws};

/// 启动弹幕会话（立即返回；会话在独立线程 + 自建 runtime 中运行）。
///
/// - `Platform::Douyu` 的 `args` 是**裸房间号**；其余平台是 `danmaku_data` JSON。
/// - 抖音的 WS 签名在**起线程之前**计算：失败直接返回，避免错误被吞进无人观察的线程。
pub fn spawn(
    platform: Platform,
    args: &str,
    stop: watch::Receiver<bool>,
    on_item: impl FnMut(LiveDanmuItem) + Send + 'static,
) -> Result<(), SliveError> {
    match platform {
        Platform::Bilibili => spawn_bilibili(args, stop, on_item),
        Platform::Douyu => spawn_douyu(args, stop, on_item),
        Platform::Huya => spawn_huya(args, stop, on_item),
        Platform::Douyin => spawn_douyin(args, stop, on_item),
        Platform::Twitch => spawn_twitch(args, stop, on_item),
    }
}

fn spawn_bilibili(
    args_json: &str,
    stop: watch::Receiver<bool>,
    on_item: impl FnMut(LiveDanmuItem) + Send + 'static,
) -> Result<(), SliveError> {
    use slive_core::platforms::bilibili::danmaku::BiliBiliDanmakuArgs;

    let args = BiliBiliDanmakuArgs::from_json(args_json)?;
    let connect = ws::binary_factory(args.ws_url(), args.connect_headers());
    runtime::spawn_session("slive-bilibili-danmaku", async move {
        slive_core::ws::run_session(
            args,
            connect,
            Arc::new(SystemClock),
            SessionTuning::bilibili(),
            stop,
            on_item,
        )
        .await;
    });
    Ok(())
}

fn spawn_douyu(
    room_id: &str,
    stop: watch::Receiver<bool>,
    on_item: impl FnMut(LiveDanmuItem) + Send + 'static,
) -> Result<(), SliveError> {
    use slive_core::platforms::douyu::danmaku::args::DouyuDanmakuArgs;
    use slive_core::platforms::douyu::danmaku::session::{douyu_tuning, run_session};

    // 斗鱼弹幕入参就是房间号本身（非 JSON）；空房间号 → InvalidInput。
    let args = DouyuDanmakuArgs::new(room_id)?;
    // 明文 TCP（ADR-015），不走 WS/TLS。
    let connect = crate::tcp::factory(args.connect_url());
    runtime::spawn_session("slive-douyu-danmaku", async move {
        run_session(args, connect, Arc::new(SystemClock), douyu_tuning(), stop, on_item).await;
    });
    Ok(())
}

fn spawn_huya(
    args_json: &str,
    stop: watch::Receiver<bool>,
    on_item: impl FnMut(LiveDanmuItem) + Send + 'static,
) -> Result<(), SliveError> {
    use slive_core::http::DEFAULT_USER_AGENT;
    use slive_core::platforms::huya::common::{BASE_URL, WS_SERVER_URL};
    use slive_core::platforms::huya::danmaku::emoticon::HuyaEmoticonTable;
    use slive_core::platforms::huya::danmaku::{run_session_with_emoticons, HuyaDanmakuArgs, SuperChatFetcher};
    use slive_core::platforms::huya::{super_chat, tup, HuyaSite};

    let args = HuyaDanmakuArgs::from_json(args_json)?;
    let headers = vec![
        ("user-agent".to_string(), DEFAULT_USER_AGENT.to_string()),
        ("origin".to_string(), BASE_URL.to_string()),
    ];
    let connect = ws::binary_factory(WS_SERVER_URL.to_string(), headers);

    // 醒目留言增量拉取：借独立站点句柄走 core 的 TUP 链路；失败返回空表，不中断主循环。
    // 必须是异步钩子 —— 它会在会话 runtime 内被 await，内部再 block_on 会 panic。
    let sc_site = Arc::new(HuyaSite::new());
    let fetch_super_chat: SuperChatFetcher = Arc::new(move |pid: i64| {
        let site = Arc::clone(&sc_site);
        Box::pin(async move {
            match super_chat::fetch_by_pid(&site, pid, false).await {
                Ok(list) => list,
                Err(e) => {
                    log::warn!("[huya-danmaku] SC 拉取失败: {e}");
                    Vec::new()
                }
            }
        })
    });

    let emote_site = Arc::new(HuyaSite::new());
    let top_sid = args.top_sid;

    runtime::spawn_session("slive-huya-danmaku", async move {
        // 表情表与房间绑定（lPid = topSid），必须在会话 runtime 内异步拉取；
        // 失败时 core 内部回退静态 97 项，不影响弹幕主链路。
        let emoticons: HuyaEmoticonTable = if top_sid != 0 {
            tup::fetch_emoticon_table(&emote_site, top_sid).await
        } else {
            slive_core::platforms::huya::danmaku::emoticon::static_table()
        };
        run_session_with_emoticons(
            args,
            connect,
            Arc::new(SystemClock),
            SessionTuning::bilibili(),
            stop,
            Some(fetch_super_chat),
            emoticons,
            on_item,
        )
        .await;
    });
    Ok(())
}

fn spawn_douyin(
    args_json: &str,
    stop: watch::Receiver<bool>,
    on_item: impl FnMut(LiveDanmuItem) + Send + 'static,
) -> Result<(), SliveError> {
    use slive_core::platforms::douyin::danmaku::args::{SDK_VERSION, VERSION_CODE};
    use slive_core::platforms::douyin::danmaku::session::run_session;
    use slive_core::platforms::douyin::danmaku::DouyinDanmakuArgs;
    use slive_core::platforms::douyin::signing::build_ws_signature;

    let args = DouyinDanmakuArgs::from_json(args_json)?;

    // 签名必须在起线程前算好：失败直接返回给调用方（否则错误被吞进无人观察的线程，
    // 表现为「直播了但连不上」且无任何提示）。
    let signature = build_ws_signature(&args.room_id, &args.user_id, VERSION_CODE, SDK_VERSION)?;
    let ts_millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or_default();
    let ws_url = args.ws_url(args.primary_host(), ts_millis, &signature);
    let connect = ws::binary_factory(ws_url, args.connect_headers());

    runtime::spawn_session("slive-douyin-danmaku", async move {
        run_session(args, connect, Arc::new(SystemClock), stop, on_item).await;
    });
    Ok(())
}

fn spawn_twitch(
    channel_or_args: &str,
    stop: watch::Receiver<bool>,
    on_item: impl FnMut(LiveDanmuItem) + Send + 'static,
) -> Result<(), SliveError> {
    use slive_core::platforms::twitch::danmaku::session::{run_session, tuning};

    let args = parse_twitch_args(channel_or_args)?;
    // IRC over WS：文本帧（TextFrames），Binary 帧也照收。
    let connect = ws::text_factory(args.ws_url(), args.connect_headers());
    runtime::spawn_session("slive-twitch-danmaku", async move {
        run_session(args, connect, Arc::new(SystemClock), tuning(), stop, on_item).await;
    });
    Ok(())
}

/// Twitch `danmakuData` 双形态：裸频道名（主形态，QA 夹具钉死 `"alice"`）或 JSON 契约串。
/// 两者都不成立 → `InvalidInput`（不静默兜底）。
fn parse_twitch_args(input: &str) -> Result<slive_core::platforms::twitch::danmaku::TwitchDanmakuArgs, SliveError> {
    use slive_core::platforms::twitch::danmaku::TwitchDanmakuArgs;
    let trimmed = input.trim();
    if !trimmed.is_empty() && !trimmed.starts_with('{') {
        return Ok(TwitchDanmakuArgs::new(trimmed));
    }
    TwitchDanmakuArgs::from_json(trimmed)
}
