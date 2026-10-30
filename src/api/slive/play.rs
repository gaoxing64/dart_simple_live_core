//! 播放线路门面：静态优选 / 实测优选 / 续流主机序重排。
//!
//! 桥层只做参数翻译与转发，排序与探测策略全部在 `slive_core::play`（ADR-008）。

use std::collections::HashMap;
use std::time::Duration;

use flutter_rust_bridge::frb;
use slive_core::play::{self, PlayLine, RankPolicy};

/// 预算下限/上限（防止调用方传离谱值把探测变成摆设/卡死）。
/// 注意：下限会**抬高**调用方的意图（100ms -> 200ms），Dart 桥的文档需与之一致。
const MIN_PROBE_BUDGET_MS: u32 = 200;
const MAX_PROBE_BUDGET_MS: u32 = 3000;
/// 失败复核轮的上界，与 `play::probe` 内部一致；用于给整条起播路径兜一个总 deadline。
const RETRY_ROUND_MS: u64 = 600;

fn policy_of(prefer_hls: bool) -> RankPolicy {
    RankPolicy {
        prefer_hls,
        ..RankPolicy::default()
    }
}

/// 静态优选（零网络、零延迟）：flv 优先，mcdn/scdn 降级，只重排不丢弃。
///
/// 起播路径默认走这个 —— 不引入任何额外时延。
#[frb(sync)]
pub fn rank_play_lines(platform_id: String, urls: Vec<String>, prefer_hls: bool) -> Vec<PlayLine> {
    play::rank_urls(&platform_id, &urls, &policy_of(prefer_hls))
}

/// 实测优选：并发探测各线路首字节时延后排序（失败项自动复核一次）。
///
/// `budget_ms` 是**单条线路**的预算，全部线路并发，所以总耗时约等于最慢一条。
/// 建议 400~800ms；超时线路保留在列表末尾而不是被丢弃。
pub async fn probe_play_lines(
    platform_id: String,
    urls: Vec<String>,
    headers: HashMap<String, String>,
    budget_ms: u32,
    prefer_hls: bool,
) -> Vec<PlayLine> {
    let lines = play::classify(&platform_id, &urls);
    if lines.is_empty() {
        return lines;
    }
    let budget = Duration::from_millis(
        budget_ms.clamp(MIN_PROBE_BUDGET_MS, MAX_PROBE_BUDGET_MS) as u64,
    );
    let headers: Vec<(String, String)> = headers.into_iter().collect();
    let prober = play::HttpProbe::new();

    // 兜总 deadline：`probe_all` 对失败线路会再跑一轮复核，单线预算 != 总耗时
    // （最坏 `budget + min(budget, 600ms)`）。超时直接退回静态序 ——
    // 优选绝不能把起播卡住。
    let deadline = budget + Duration::from_millis(RETRY_ROUND_MS);
    match tokio::time::timeout(deadline, play::probe_all(&prober, &lines, &headers, budget)).await {
        Ok(scores) => play::rank_scored(&lines, &scores, &policy_of(prefer_hls)),
        Err(_) => play::rank(&lines, &policy_of(prefer_hls)),
    }
}

/// 提取主机顺序（配合 [`reorder_play_urls`] 沉淀「哪条 CDN 更快」）。
#[frb(sync)]
pub fn host_order_of_lines(lines: Vec<PlayLine>) -> Vec<String> {
    play::host_order_of(&lines)
}

/// 按主机顺序重排刷新后的直链（续流路径专用）。
///
/// 直链刷新后字符串全变但主机稳定，靠它保证 `urls[line]` 仍指向同一条 CDN。
#[frb(sync)]
pub fn reorder_play_urls(urls: Vec<String>, host_order: Vec<String>) -> Vec<String> {
    play::reorder_by_host_order(&urls, &host_order)
}
