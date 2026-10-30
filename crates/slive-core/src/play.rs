//! 播放线路的结构化与优选排序（CDN 优选的地基）。
//!
//! # 为什么需要这一层
//!
//! 各平台 `get_play_urls` 只回 `Vec<String>`，线路语义在拼串时丢失：哪条是 B站
//! `mcdn`、哪条是斗鱼 `scdn`、这条是 FLV 还是 HLS，下游只能靠 `contains(".flv")`
//! 猜。排序规则也散落在各平台内部（B站 `sort_by_key(contains("mcdn"))`、斗鱼
//! `sort_by_key(starts_with("scdn"))`、虎牙靠遍历顺序、抖音靠 `hlsFirst`）。
//!
//! 本模块把两件事收敛到一处：
//!
//! 1. [`classify`]：把裸 URL 还原成 [`PlayLine`]（容器类型 / 主机 / 平台标记）；
//! 2. [`rank`]：按 [`RankPolicy`] 给出确定的优选顺序。
//!
//! # 边界
//!
//! 本模块**不做任何 I/O**，是纯函数层。真正的网络探测（RTT / TTFB / 小段吞吐）
//! 属下一阶段，以 `PlayProbe` seam 注入；届时 [`rank`] 只需多接一个成绩切片，
//! 调用方与策略结构不变。
//!
//! 排序**只重排、不丢弃**（除完全相同的 URL），保证任何情况下线路条数不减 ——
//! 优选不能让可用线路变少。

use std::collections::HashSet;

pub mod probe;

pub use probe::{probe_all, HttpProbe, PlayProbe, ProbeResult, DEFAULT_PROBE_BUDGET};

/// 线路容器 / 协议类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineKind {
    /// FLV（HTTP-FLV，起播快、延迟低）。
    Flv,
    /// HLS（m3u8，兼容性好、延迟高）。
    Hls,
    /// 无法判定（保持原序，排在两类之后）。
    Other,
}

impl LineKind {
    /// 短标签（CLI / 日志用）。
    pub fn label(self) -> &'static str {
        match self {
            LineKind::Flv => "flv",
            LineKind::Hls => "hls",
            LineKind::Other => "???",
        }
    }
}

/// 线路来源标记（平台语义，由 URL 主机推导）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LineHint {
    /// 平台主推线路。
    Primary,
    /// B站 mcdn 线路（P2P/边缘节点，实测最易失败，默认降级）。
    Mcdn,
    /// 斗鱼 scdn 线路（实测最易失败，默认降级）。
    Scdn,
}

impl LineHint {
    /// 短标签（CLI / 日志用）。
    pub fn label(self) -> &'static str {
        match self {
            LineHint::Primary => "主线路",
            LineHint::Mcdn => "mcdn",
            LineHint::Scdn => "scdn",
        }
    }
}

/// 一条结构化播放线路。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PlayLine {
    /// 播放地址（原样，不做任何改写）。
    pub url: String,
    /// 容器 / 协议类型。
    pub kind: LineKind,
    /// 主机（小写，已去端口与 userinfo）。
    pub host: String,
    /// 平台标记。
    pub hint: LineHint,
    /// 在平台原始列表中的下标（0 基），用于「优选后仍能回到原序」。
    pub origin_index: usize,
}

/// 优选策略。
///
/// 默认值对应「直播优先」口径：FLV 在前（起播快、延迟低），mcdn/scdn 降级，
/// 完全重复的 URL 去重。这些都与各平台现有硬编码规则一致，只是收敛到了这里。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RankPolicy {
    /// 是否让 HLS 排在 FLV 之前（App 的「HLS 优先」偏好映射到此）。
    pub prefer_hls: bool,
    /// 是否把 B站 mcdn 线路降级。
    pub demote_mcdn: bool,
    /// 是否把斗鱼 scdn 线路降级。
    pub demote_scdn: bool,
    /// 是否丢弃完全相同的 URL（仅字符串级去重，不做主机折叠）。
    pub drop_duplicate_urls: bool,
}

impl Default for RankPolicy {
    fn default() -> Self {
        Self {
            prefer_hls: false,
            demote_mcdn: true,
            demote_scdn: true,
            // 默认**不**去重：契约是「只重排不丢弃」，且续流 `reorder_by_host_order`
            // 不做去重 —— 两边长度一旦不同，「第 N 条线路」就不再指向同一条 CDN。
            // 需要折叠重复 URL 的调用方显式打开。
            drop_duplicate_urls: false,
        }
    }
}

impl RankPolicy {
    /// HLS 优先（对齐 App 的 `hlsFirst` 偏好）。
    pub fn hls_first() -> Self {
        Self {
            prefer_hls: true,
            ..Self::default()
        }
    }
}

/// 从 URL 提取主机（小写；去 userinfo 与端口；保留 IPv6 字面量的方括号）。
///
/// 手写而非引入 `url` crate：只需 authority 段，且要能容忍平台下发的
/// 非严格 URL（历史数据里出现过裸 host）。
pub fn host_of(url: &str) -> String {
    let rest = match url.split_once("://") {
        Some((_, tail)) => tail,
        None => url,
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    let authority = authority.rsplit('@').next().unwrap_or(authority);
    let host = match authority.find(']') {
        // IPv6 字面量：`[::1]:8080` -> `[::1]`
        Some(end) => &authority[..=end],
        None => authority.split(':').next().unwrap_or(authority),
    };
    host.to_ascii_lowercase()
}

/// 由 URL 路径判定容器类型（查询串不参与判定）。
pub fn kind_of(url: &str) -> LineKind {
    let path = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase();
    if path.ends_with(".m3u8") {
        LineKind::Hls
    } else if path.ends_with(".flv") {
        LineKind::Flv
    } else {
        LineKind::Other
    }
}

/// 由平台 id + 主机判定线路标记。
///
/// 只覆盖有实测依据的两类：
/// - B站 mcdn 走 `*.mcdn.bilivideo.cn` 一类主机；
/// - 斗鱼 scdn 线路的 CDN 名以 `scdn` 开头，直接体现在主机首段。
///
/// 其余平台（虎牙/抖音/Twitch）的线路本身就是平台下发的并列 CDN，一律 `Primary`。
fn hint_of(platform_id: &str, host: &str) -> LineHint {
    match platform_id {
        "bilibili" if host.contains("mcdn") => LineHint::Mcdn,
        "douyu" if host.starts_with("scdn") => LineHint::Scdn,
        _ => LineHint::Primary,
    }
}

/// 把平台返回的裸 URL 列表还原成结构化线路（保持原序，空串跳过）。
pub fn classify(platform_id: &str, urls: &[String]) -> Vec<PlayLine> {
    urls.iter()
        .enumerate()
        .filter_map(|(origin_index, url)| {
            // 校验与存储必须用同一个值：只 trim 校验、存原值会把带空白的 URL
            // 交给探测器和播放器，失败后还会被误判成「这条线路是死的」。
            let url = url.trim();
            if url.is_empty() {
                return None;
            }
            let host = host_of(url);
            Some(PlayLine {
                url: url.to_string(),
                kind: kind_of(url),
                hint: hint_of(platform_id, &host),
                host,
                origin_index,
            })
        })
        .collect()
}

/// 标记权重（越小越靠前）。
fn hint_rank(hint: LineHint, policy: &RankPolicy) -> u8 {
    match hint {
        LineHint::Primary => 0,
        LineHint::Mcdn if policy.demote_mcdn => 1,
        LineHint::Scdn if policy.demote_scdn => 1,
        _ => 0,
    }
}

/// 容器权重（越小越靠前）。
fn kind_rank(kind: LineKind, policy: &RankPolicy) -> u8 {
    match (kind, policy.prefer_hls) {
        (LineKind::Other, _) => 2,
        (LineKind::Hls, true) | (LineKind::Flv, false) => 0,
        _ => 1,
    }
}

/// 按策略给出优选顺序。
///
/// 排序键为 `(标记权重, 容器权重, 原序)`，稳定排序 —— 同权重的线路保持平台
/// 给出的相对顺序（平台顺序通常已含服务端调度意图，不能随意打乱）。
pub fn rank(lines: &[PlayLine], policy: &RankPolicy) -> Vec<PlayLine> {
    let mut kept: Vec<&PlayLine> = Vec::with_capacity(lines.len());
    if policy.drop_duplicate_urls {
        let mut seen: HashSet<&str> = HashSet::new();
        for line in lines {
            if seen.insert(line.url.as_str()) {
                kept.push(line);
            }
        }
    } else {
        kept.extend(lines.iter());
    }

    let mut out: Vec<PlayLine> = kept.into_iter().cloned().collect();
    out.sort_by_key(|line| {
        (
            hint_rank(line.hint, policy),
            kind_rank(line.kind, policy),
            line.origin_index,
        )
    });
    out
}

/// [`classify`] + [`rank`] 的便捷组合。
pub fn rank_urls(platform_id: &str, urls: &[String], policy: &RankPolicy) -> Vec<PlayLine> {
    rank(&classify(platform_id, urls), policy)
}

/// 按主机顺序重排 URL 列表（续流路径专用）。
///
/// 直链带签名，刷新后**字符串全变**，但 CDN 主机稳定。因此「哪条线路更好」应以
/// **主机顺序**沉淀：首次起播拿到 `[host_a, host_b, ...]`，续流时用同一顺序重排
/// 新直链，`urls[line]` 仍指向同一条 CDN —— 否则续流会悄悄换到别的线路。
///
/// 未出现在 `host_order` 里的主机保持原有相对顺序、排在最后（保守：新 CDN 未知）。
pub fn reorder_by_host_order(urls: &[String], host_order: &[String]) -> Vec<String> {
    if host_order.is_empty() || urls.is_empty() {
        return urls.to_vec();
    }
    // 同一主机可能对应多条线路（B站 format×host、虎牙多 stream），因此不能只按
    // 「主机首次出现的位置」匹配 —— 那会把同主机的线路全折叠到同一个槽位，导致
    // 下标整体前移（`[h1a,h2a,h1b]` 变成 `[h1a,h1b,h2a]`，第 2 条线路换了 CDN）。
    // 这里按**槽位消费**：每条 URL 占用 host_order 里第一个尚未被占用的同名槽位。
    let mut used = vec![false; host_order.len()];
    let mut indexed: Vec<(usize, usize, &String)> = urls
        .iter()
        .enumerate()
        .map(|(index, url)| {
            let host = host_of(url);
            let rank = host_order
                .iter()
                .enumerate()
                .position(|(slot, candidate)| !used[slot] && candidate == &host)
                .unwrap_or(usize::MAX);
            if rank != usize::MAX {
                used[rank] = true;
            }
            (rank, index, url)
        })
        .collect();
    indexed.sort_by_key(|(rank, index, _)| (*rank, *index));
    indexed.into_iter().map(|(_, _, url)| url.clone()).collect()
}

/// 从一组线路里提取主机顺序（供 [`reorder_by_host_order`] 沉淀）。
pub fn host_order_of(lines: &[PlayLine]) -> Vec<String> {
    lines.iter().map(|line| line.host.clone()).collect()
}

/// 排序键：`(分组, 时延权重, 标记权重, 容器权重, 原序)`。
type RankKey = (u8, u32, u8, u8, usize);

/// 带探测成绩的优选排序：**实测时延优先于静态标记**。
///
/// 分组口径：
/// 1. 探测可用 —— 按首字节时延升序；
/// 2. 未探测（`scores` 比 `lines` 短）—— 保持静态权重顺序；
/// 3. 探测失败 —— 排在最后，但**不丢弃**（换个时间点可能就好了）。
///
/// 只有时延相同时才回落到 `hint`/`kind` 权重，所以「mcdn 实测更快」时它会被
/// 正常排到前面 —— 这是刻意的：有了实测就不该再被静态偏见压住。
///
/// `scores[i]` 对应 `lines[i]`；`scores` 为空时退化为纯静态 [`rank`]。
pub fn rank_scored(
    lines: &[PlayLine],
    scores: &[ProbeResult],
    policy: &RankPolicy,
) -> Vec<PlayLine> {
    if scores.is_empty() {
        return rank(lines, policy);
    }

    let mut seen: HashSet<&str> = HashSet::new();
    let mut kept: Vec<(&PlayLine, Option<&ProbeResult>)> = Vec::with_capacity(lines.len());
    for (index, line) in lines.iter().enumerate() {
        if policy.drop_duplicate_urls && !seen.insert(line.url.as_str()) {
            continue;
        }
        kept.push((line, scores.get(index)));
    }

    let mut keyed: Vec<(RankKey, &PlayLine)> = kept
        .into_iter()
        .map(|(line, score)| {
            let (group, weight) = match score {
                Some(result) if result.ok => (0u8, result.sort_weight()),
                Some(_) => (2u8, u32::MAX),
                None => (1u8, u32::MAX),
            };
            let key = (
                group,
                weight,
                hint_rank(line.hint, policy),
                kind_rank(line.kind, policy),
                line.origin_index,
            );
            (key, line)
        })
        .collect();
    keyed.sort_by_key(|(key, _)| *key);
    keyed.into_iter().map(|(_, line)| line.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn urls(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn host_parsing_handles_scheme_port_and_query() {
        assert_eq!(
            host_of("https://d1--cn-gotcha04.bilivideo.com/live/x.flv?sign=1"),
            "d1--cn-gotcha04.bilivideo.com"
        );
        assert_eq!(
            host_of("http://al-game.flv.huya.com/src/a.flv?wsSecret=x"),
            "al-game.flv.huya.com"
        );
        assert_eq!(host_of("https://example.com:8443/a.flv"), "example.com");
        assert_eq!(host_of("https://user:pw@example.com/a.flv"), "example.com");
        assert_eq!(host_of("https://[::1]:8443/a.flv"), "[::1]");
        assert_eq!(host_of("EXAMPLE.com/a.flv"), "example.com");
    }

    #[test]
    fn kind_detection_ignores_query_string() {
        assert_eq!(kind_of("https://h/a.flv?x=.m3u8"), LineKind::Flv);
        assert_eq!(
            kind_of("https://h/live/index.m3u8?wsSecret=1"),
            LineKind::Hls
        );
        assert_eq!(kind_of("https://h/a.mp4"), LineKind::Other);
    }

    #[test]
    fn bilibili_mcdn_is_marked() {
        let lines = classify(
            "bilibili",
            &urls(&[
                "https://d1--cn-gotcha04.bilivideo.com/live/a.flv?cdn=cn-gotcha04",
                "https://cn-hbxy-cu-01-11.mcdn.bilivideo.cn:4483/playurl?cdn=mcdn",
            ]),
        );
        assert_eq!(lines[0].hint, LineHint::Primary);
        assert_eq!(lines[1].hint, LineHint::Mcdn);
        assert_eq!(lines[1].kind, LineKind::Other, "mcdn 的 playurl 无扩展名");
    }

    #[test]
    fn douyu_scdn_is_marked_by_host_prefix() {
        let lines = classify(
            "douyu",
            &urls(&[
                "https://hw-h5.douyucdn2.cn/live/a.flv?x=1",
                "https://scdn-h5.douyucdn2.cn/live/a.flv?x=1",
            ]),
        );
        assert_eq!(lines[0].hint, LineHint::Primary);
        assert_eq!(lines[1].hint, LineHint::Scdn);
    }

    #[test]
    fn huya_lines_stay_primary_and_split_by_container() {
        let lines = classify(
            "huya",
            &urls(&[
                "http://al-game.flv.huya.com/src/a.flv?wsSecret=1",
                "http://al.hls.huya.com/src/a.m3u8?wsSecret=1",
            ]),
        );
        assert!(lines.iter().all(|l| l.hint == LineHint::Primary));
        assert_eq!(lines[0].kind, LineKind::Flv);
        assert_eq!(lines[1].kind, LineKind::Hls);
    }

    #[test]
    fn rank_puts_flv_before_hls_by_default() {
        let lines = classify(
            "huya",
            &urls(&[
                "http://al.hls.huya.com/src/a.m3u8?wsSecret=1",
                "http://al-game.flv.huya.com/src/a.flv?wsSecret=1",
            ]),
        );
        let ranked = rank(&lines, &RankPolicy::default());
        assert_eq!(ranked[0].kind, LineKind::Flv, "默认 FLV 优先");
        assert_eq!(ranked[0].origin_index, 1);
        assert_eq!(ranked[1].origin_index, 0);
    }

    #[test]
    fn hls_first_policy_flips_container_order() {
        let lines = classify(
            "huya",
            &urls(&[
                "http://al-game.flv.huya.com/src/a.flv?wsSecret=1",
                "http://al.hls.huya.com/src/a.m3u8?wsSecret=1",
            ]),
        );
        let ranked = rank(&lines, &RankPolicy::hls_first());
        assert_eq!(ranked[0].kind, LineKind::Hls);
    }

    #[test]
    fn mcdn_goes_last_but_survives() {
        let lines = classify(
            "bilibili",
            &urls(&[
                "https://cn-hbxy-cu-01-11.mcdn.bilivideo.cn:4483/playurl?cdn=mcdn",
                "https://d1--cn-gotcha04.bilivideo.com/live/a.flv?cdn=cn-gotcha04",
            ]),
        );
        let ranked = rank(&lines, &RankPolicy::default());
        assert_eq!(ranked.len(), 2, "降级不等于丢弃");
        assert_eq!(ranked[1].hint, LineHint::Mcdn);
    }

    #[test]
    fn demote_switch_controls_mcdn_priority() {
        // 直接构造：真实 mcdn 地址是无扩展名的 `playurl`，会被 `kind_of` 判为 Other，
        // 干扰「只测标记降级」的意图，故这里固定 kind 相同。
        let lines = vec![
            PlayLine {
                url: "https://mcdn.example/a.flv".to_string(),
                kind: LineKind::Flv,
                host: "mcdn.example".to_string(),
                hint: LineHint::Mcdn,
                origin_index: 0,
            },
            PlayLine {
                url: "https://cdn.example/a.flv".to_string(),
                kind: LineKind::Flv,
                host: "cdn.example".to_string(),
                hint: LineHint::Primary,
                origin_index: 1,
            },
        ];

        let demoted = rank(&lines, &RankPolicy::default());
        assert_eq!(demoted[0].origin_index, 1, "默认把 mcdn 降到后面");

        let kept = rank(
            &lines,
            &RankPolicy {
                demote_mcdn: false,
                ..RankPolicy::default()
            },
        );
        assert_eq!(kept[0].origin_index, 0, "关闭降级后回到平台原序");
    }

    #[test]
    fn duplicate_urls_are_kept_by_default_and_dropped_on_demand() {
        let lines = classify(
            "douyu",
            &urls(&[
                "https://hw-h5.douyucdn2.cn/a.flv",
                "https://hw-h5.douyucdn2.cn/a.flv",
            ]),
        );
        assert_eq!(
            rank(&lines, &RankPolicy::default()).len(),
            2,
            "默认只重排不丢弃"
        );

        let drop_dup = RankPolicy {
            drop_duplicate_urls: true,
            ..RankPolicy::default()
        };
        assert_eq!(rank(&lines, &drop_dup).len(), 1);
    }

    #[test]
    fn same_host_is_not_folded() {
        // 同主机不同 stream（虎牙常见：一个 CDN 下发多个 stream）必须都保留。
        let lines = classify(
            "huya",
            &urls(&[
                "http://tx.flv.huya.com/src/1.flv?wsSecret=a",
                "http://tx.flv.huya.com/src/2.flv?wsSecret=b",
            ]),
        );
        assert_eq!(rank(&lines, &RankPolicy::default()).len(), 2);
    }

    #[test]
    fn empty_urls_are_skipped_and_index_preserved() {
        let lines = classify("bilibili", &urls(&["", "https://h/a.flv", "  "]));
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].origin_index, 1, "空串跳过后下标仍是原始位置");
    }

    #[test]
    fn host_order_reorders_refreshed_urls_by_cdn() {
        // 刷新后直链全变，但主机不变：应按缓存的主机顺序重排。
        let fresh = urls(&[
            "http://al.hls.huya.com/src/9.m3u8?wsSecret=NEW",
            "http://tx.flv.huya.com/src/9.flv?wsSecret=NEW",
            "http://al-game.flv.huya.com/src/9.flv?wsSecret=NEW",
        ]);
        let order = vec![
            "tx.flv.huya.com".to_string(),
            "al-game.flv.huya.com".to_string(),
            "al.hls.huya.com".to_string(),
        ];
        let reordered = reorder_by_host_order(&fresh, &order);
        assert!(reordered[0].contains("tx.flv.huya.com"));
        assert!(reordered[1].contains("al-game.flv.huya.com"));
        assert!(reordered[2].contains("al.hls.huya.com"));
    }

    #[test]
    fn repeated_hosts_consume_distinct_slots() {
        // 虎牙真实形态：同一 CDN 主机下发多条 stream。
        // 主机序 [h1,h2,h1] 重排 [h1a,h2a,h1b] 必须原序返回，不能折叠成 [h1a,h1b,h2a]。
        let fresh = urls(&[
            "http://al.flv.huya.com/a.flv",
            "http://tx.flv.huya.com/b.flv",
            "http://al.flv.huya.com/c.flv",
        ]);
        let order = vec![
            "al.flv.huya.com".to_string(),
            "tx.flv.huya.com".to_string(),
            "al.flv.huya.com".to_string(),
        ];
        assert_eq!(reorder_by_host_order(&fresh, &order), fresh);

        // 优选把两条 al 排在一起后，续流也必须跟着走同样的分组。
        let ranked_order = vec![
            "al.flv.huya.com".to_string(),
            "al.flv.huya.com".to_string(),
            "tx.flv.huya.com".to_string(),
        ];
        let reordered = reorder_by_host_order(&fresh, &ranked_order);
        assert!(reordered[0].contains("al.flv.huya.com"));
        assert!(reordered[1].contains("al.flv.huya.com"));
        assert!(reordered[2].contains("tx.flv.huya.com"));
    }

    #[test]
    fn unknown_hosts_keep_original_order_and_go_last() {
        let fresh = urls(&["http://new-cdn.example/a.flv", "http://known.example/b.flv"]);
        let order = vec!["known.example".to_string()];
        let reordered = reorder_by_host_order(&fresh, &order);
        assert!(reordered[0].contains("known.example"));
        assert!(reordered[1].contains("new-cdn.example"));
    }

    #[test]
    fn empty_host_order_is_identity() {
        let fresh = urls(&["http://a/x.flv", "http://b/y.flv"]);
        assert_eq!(reorder_by_host_order(&fresh, &[]), fresh);
    }

    #[test]
    fn host_order_of_follows_line_order() {
        let lines = classify(
            "huya",
            &urls(&[
                "http://tx.flv.huya.com/a.flv",
                "http://al-game.flv.huya.com/b.flv",
            ]),
        );
        assert_eq!(
            host_order_of(&lines),
            vec!["tx.flv.huya.com", "al-game.flv.huya.com"]
        );
    }

    #[test]
    fn ranking_is_deterministic() {
        let input = urls(&[
            "https://a.hls/x.m3u8",
            "https://b.flv/x.flv",
            "https://c.hls/x.m3u8",
            "https://d.flv/x.flv",
        ]);
        let first = rank_urls("huya", &input, &RankPolicy::default());
        let second = rank_urls("huya", &input, &RankPolicy::default());
        assert_eq!(first, second);
        assert_eq!(
            first.iter().map(|l| l.origin_index).collect::<Vec<_>>(),
            vec![1, 3, 0, 2],
            "flv 稳定在前，hls 保持相对顺序"
        );
    }
}
