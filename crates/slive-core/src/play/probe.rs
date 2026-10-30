//! 线路探测：真实网络测量（首字节时延），供优选排序使用。
//!
//! # 为什么单独成模块
//!
//! [`super::rank`] 是纯函数，只能表达「静态偏好」（flv 优先、mcdn 降级）。真正的
//! CDN 好坏取决于**从这台机器到那个边缘节点**的时延与可用性，必须实测。这里把
//! 测量抽成 [`PlayProbe`] seam：生产用 [`HttpProbe`]，测试注入固定成绩，排序逻辑
//! 仍留在纯函数里可断言。
//!
//! # 成本控制（重要）
//!
//! 探测跑在起播路径旁边，必须便宜：
//! - **只读一个 chunk 就断开**，不拉整段流；FLV 的首个 chunk 通常是一两 KB；
//! - 每条线路一个**独立预算**（默认 1s），超时即判失败，不拖住整体；
//! - 全部线路**并发**探测，总耗时约等于最慢的一条，而非求和；
//! - 不带 `Range` 头 —— 部分 CDN 对 Range 返回 416/403，会把好线路误判成坏线路，
//!   得不偿失（少读的那点流量远不如误判的代价大）。
//!
//! # 纪律
//!
//! 本模块只做「测」，不做「选」。选序在 [`super::rank_scored`]，与静态策略共用
//! 同一套权重，避免两处各写一份排序。

use std::time::{Duration, Instant};

use async_trait::async_trait;

use super::PlayLine;

/// 默认单线探测预算。
pub const DEFAULT_PROBE_BUDGET: Duration = Duration::from_millis(1200);

/// 失败复核的单线预算上限（此时连接池已热，不需要完整预算）。
const RETRY_BUDGET: Duration = Duration::from_millis(600);

/// 单条线路的探测成绩。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeResult {
    /// 是否可用（HTTP 2xx 且拿到了首个数据块）。
    pub ok: bool,
    /// 从发请求到收到**响应头**的毫秒数（含 DNS/连接/TLS/服务端首包）。
    pub ttfb_ms: u32,
    /// 从发请求到收到**首个数据块**的毫秒数。
    pub first_byte_ms: u32,
    /// HTTP 状态码（0 表示没拿到响应）。
    pub status: u16,
    /// 首个数据块字节数。
    pub bytes: u32,
    /// 失败原因（`ok=false` 时非空）。
    pub error: Option<String>,
}

impl ProbeResult {
    /// 构造一条失败成绩。
    pub fn failed(error: impl Into<String>) -> Self {
        Self {
            ok: false,
            ttfb_ms: 0,
            first_byte_ms: 0,
            status: 0,
            bytes: 0,
            error: Some(error.into()),
        }
    }

    /// 排序用权重：可用线路按首字节时延，失败线路排到所有可用线路之后。
    pub(crate) fn sort_weight(&self) -> u32 {
        if self.ok {
            self.first_byte_ms.max(1)
        } else {
            u32::MAX
        }
    }
}

/// 线路探测器（seam）。
#[async_trait]
pub trait PlayProbe: Send + Sync {
    /// 在 `budget` 内探测一条线路。
    ///
    /// 实现方**不得** panic，也不得在超时后继续占用连接：超时即返回失败成绩。
    async fn probe(
        &self,
        line: &PlayLine,
        headers: &[(String, String)],
        budget: Duration,
    ) -> ProbeResult;
}

/// 生产实现：`reqwest` + 与 HTTP 传输层同一个 Client 配置（rustls / http1 / 无代理）。
pub struct HttpProbe {
    client: reqwest::Client,
}

impl HttpProbe {
    /// 生产构造。
    pub fn new() -> Self {
        Self {
            client: crate::http::build_client(),
        }
    }
}

impl Default for HttpProbe {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl PlayProbe for HttpProbe {
    async fn probe(
        &self,
        line: &PlayLine,
        headers: &[(String, String)],
        budget: Duration,
    ) -> ProbeResult {
        let mut request = self.client.get(&line.url).timeout(budget);
        for (key, value) in headers {
            request = request.header(key.as_str(), value.as_str());
        }

        let started = Instant::now();
        let mut response = match request.send().await {
            Ok(response) => response,
            Err(e) => return ProbeResult::failed(short_error(&e.to_string())),
        };
        let ttfb_ms = elapsed_ms(started);

        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            return ProbeResult {
                ok: false,
                ttfb_ms,
                first_byte_ms: 0,
                status,
                bytes: 0,
                error: Some(format!("HTTP {status}")),
            };
        }

        // 只取一个 chunk 就断开：够判断「有没有数据流出」，又不拉整段流。
        let remaining = budget.saturating_sub(started.elapsed());
        let bytes = match tokio::time::timeout(remaining, response.chunk()).await {
            Ok(Ok(Some(chunk))) => chunk.len() as u32,
            Ok(Ok(None)) => 0,
            Ok(Err(e)) => return ProbeResult::failed(short_error(&e.to_string())),
            Err(_) => 0,
        };
        let first_byte_ms = elapsed_ms(started);

        if bytes == 0 {
            // 头通了但一个字节都没来：对直播流而言等价于不可用。
            return ProbeResult {
                ok: false,
                ttfb_ms,
                first_byte_ms,
                status,
                bytes: 0,
                error: Some("首块超时/空响应".to_string()),
            };
        }

        ProbeResult {
            ok: true,
            ttfb_ms,
            first_byte_ms,
            status,
            bytes,
            error: None,
        }
    }
}

/// 并发探测全部线路，返回与 `lines` **同序**的成绩。
///
/// **单轮**耗时约等于最慢一条（各自独立预算），不会随线路数线性增长；
/// 但失败复核是第二轮，所以整体上界是 `budget + min(budget, RETRY_BUDGET)`。
///
/// # 失败复核（重要）
///
/// 单次探测有假阴性：冷连接首次 DNS+TCP+TLS 偶尔会吃掉整个预算（实测抖音 HLS
/// 首测超时、复测 313ms 正常）。若不复核，一条好线路会被误判成不可用并排到最后。
/// 因此对**失败项**再做一轮并发复核（预算收紧到 600ms）—— 此时连接池已热，
/// 代价很小；真死的线路复核仍失败，保留首次的错误原因。
pub async fn probe_all(
    prober: &dyn PlayProbe,
    lines: &[PlayLine],
    headers: &[(String, String)],
    budget: Duration,
) -> Vec<ProbeResult> {
    let futures = lines.iter().map(|line| prober.probe(line, headers, budget));
    let mut results: Vec<ProbeResult> = futures_util::future::join_all(futures).await;

    let failed: Vec<usize> = results
        .iter()
        .enumerate()
        .filter(|(_, result)| !result.ok)
        .map(|(index, _)| index)
        .collect();
    if failed.is_empty() {
        return results;
    }

    let retry_budget = budget.min(RETRY_BUDGET);
    let retries = failed
        .iter()
        .map(|&index| prober.probe(&lines[index], headers, retry_budget));
    let retried: Vec<ProbeResult> = futures_util::future::join_all(retries).await;
    for (slot, result) in failed.into_iter().zip(retried) {
        if result.ok {
            results[slot] = result;
        }
    }
    results
}

fn elapsed_ms(started: Instant) -> u32 {
    u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX)
}

/// 错误串可能很长（含完整 URL 与证书链），截断到可读长度。
fn short_error(message: &str) -> String {
    const CAP: usize = 120;
    if message.chars().count() <= CAP {
        message.to_string()
    } else {
        message.chars().take(CAP).collect::<String>() + "…"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::play::{rank_scored, LineHint, LineKind, RankPolicy};

    fn line(url: &str, kind: LineKind, origin_index: usize) -> PlayLine {
        PlayLine {
            url: url.to_string(),
            kind,
            host: crate::play::host_of(url),
            hint: LineHint::Primary,
            origin_index,
        }
    }

    fn ok(first_byte_ms: u32) -> ProbeResult {
        ProbeResult {
            ok: true,
            ttfb_ms: first_byte_ms / 2,
            first_byte_ms,
            status: 200,
            bytes: 1024,
            error: None,
        }
    }

    #[test]
    fn sort_weight_puts_failures_last() {
        assert!(ok(50).sort_weight() < ProbeResult::failed("x").sort_weight());
    }

    #[test]
    fn scored_ranking_prefers_lowest_first_byte() {
        let lines = vec![
            line("https://a/x.flv", LineKind::Flv, 0),
            line("https://b/x.flv", LineKind::Flv, 1),
            line("https://c/x.flv", LineKind::Flv, 2),
        ];
        let scores = vec![ok(300), ok(80), ok(150)];
        let ranked = rank_scored(&lines, &scores, &RankPolicy::default());
        assert_eq!(
            ranked.iter().map(|l| l.origin_index).collect::<Vec<_>>(),
            vec![1, 2, 0],
            "按首字节时延升序"
        );
    }

    #[test]
    fn failed_lines_are_kept_but_last() {
        let lines = vec![
            line("https://a/x.flv", LineKind::Flv, 0),
            line("https://b/x.flv", LineKind::Flv, 1),
        ];
        let scores = vec![ProbeResult::failed("timeout"), ok(500)];
        let ranked = rank_scored(&lines, &scores, &RankPolicy::default());
        assert_eq!(ranked.len(), 2, "失败线路不丢弃");
        assert_eq!(ranked[0].origin_index, 1);
        assert_eq!(ranked[1].origin_index, 0);
    }

    #[test]
    fn unprobed_lines_sit_between_ok_and_failed() {
        let lines = vec![
            line("https://a/x.flv", LineKind::Flv, 0),
            line("https://b/x.flv", LineKind::Flv, 1),
            line("https://c/x.flv", LineKind::Flv, 2),
        ];
        // 只探了两条：第 3 条没有成绩。
        let scores = vec![ProbeResult::failed("timeout"), ok(200)];
        let ranked = rank_scored(&lines, &scores, &RankPolicy::default());
        assert_eq!(
            ranked.iter().map(|l| l.origin_index).collect::<Vec<_>>(),
            vec![1, 2, 0],
            "可用 → 未探测 → 失败"
        );
    }

    #[test]
    fn empty_scores_fall_back_to_static_order() {
        let lines = vec![
            line("https://a/x.m3u8", LineKind::Hls, 0),
            line("https://b/x.flv", LineKind::Flv, 1),
        ];
        let ranked = rank_scored(&lines, &[], &RankPolicy::default());
        assert_eq!(
            ranked[0].kind,
            LineKind::Flv,
            "无成绩时退回静态策略（flv 优先）"
        );
    }

    /// 可编排的探测器：按顺序吐预设成绩，用于验证复核逻辑。
    struct ScriptedProbe {
        script: std::sync::Mutex<std::collections::VecDeque<ProbeResult>>,
        calls: std::sync::atomic::AtomicUsize,
    }

    #[async_trait]
    impl PlayProbe for ScriptedProbe {
        async fn probe(
            &self,
            _line: &PlayLine,
            _headers: &[(String, String)],
            _budget: Duration,
        ) -> ProbeResult {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.script
                .lock()
                .expect("script")
                .pop_front()
                .unwrap_or_else(|| ProbeResult::failed("脚本耗尽"))
        }
    }

    #[tokio::test]
    async fn probe_all_retries_failures_once() {
        let probe = ScriptedProbe {
            script: std::sync::Mutex::new(
                vec![ProbeResult::failed("冷连接超时"), ok(200)]
                    .into_iter()
                    .collect(),
            ),
            calls: std::sync::atomic::AtomicUsize::new(0),
        };
        let lines = vec![line("https://a/x.flv", LineKind::Flv, 0)];
        let results = probe_all(&probe, &lines, &[], Duration::from_millis(50)).await;
        assert!(results[0].ok, "首测失败应被复核救回");
        assert_eq!(probe.calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn probe_all_keeps_failure_when_retry_also_fails() {
        let probe = ScriptedProbe {
            script: std::sync::Mutex::new(
                vec![
                    ProbeResult::failed("HTTP 403"),
                    ProbeResult::failed("HTTP 403"),
                ]
                .into_iter()
                .collect(),
            ),
            calls: std::sync::atomic::AtomicUsize::new(0),
        };
        let lines = vec![line("https://a/x.flv", LineKind::Flv, 0)];
        let results = probe_all(&probe, &lines, &[], Duration::from_millis(50)).await;
        assert!(!results[0].ok);
        assert_eq!(
            results[0].error.as_deref(),
            Some("HTTP 403"),
            "保留首次错误原因"
        );
    }

    #[tokio::test]
    async fn probe_all_does_not_retry_successes() {
        let probe = ScriptedProbe {
            script: std::sync::Mutex::new(vec![ok(100)].into_iter().collect()),
            calls: std::sync::atomic::AtomicUsize::new(0),
        };
        let lines = vec![line("https://a/x.flv", LineKind::Flv, 0)];
        let results = probe_all(&probe, &lines, &[], Duration::from_millis(50)).await;
        assert!(results[0].ok);
        assert_eq!(
            probe.calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "成功不重测"
        );
    }

    #[test]
    fn probe_result_records_http_error_status() {
        let failed = ProbeResult::failed("HTTP 403");
        assert!(!failed.ok);
        assert_eq!(failed.status, 0, "构造器不写状态码，由实现方按需填");
    }
}
