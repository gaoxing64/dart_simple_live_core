//! `slive` —— Simple Live 命令行工具。
//!
//! 直连 `slive-core`（不经 flutter_rust_bridge），因此可做纯 Rust 单文件分发。

mod danmaku;
mod url;

use std::collections::HashMap;

use slive_core::models::{LivePlayQuality, LiveRoomDetail};
use slive_core::play::{self, RankPolicy};
use slive_core::platforms::LiveSite;

use url::{Platform, Target};

const HELP: &str = "\
slive —— Simple Live 命令行工具

用法:
  slive info <URL> [-q <清晰度>] [--all]   获取房间信息与播放直链
  slive danmaku <URL> [--no-color]         持续输出弹幕（Ctrl-C 退出）
  slive help                               显示本帮助

支持站点:
  B站     https://live.bilibili.com/<房间号>
  斗鱼    https://www.douyu.com/<房间号>
  虎牙    https://www.huya.com/<房间号>
  抖音    https://live.douyin.com/<web_rid>
  Twitch  https://www.twitch.tv/<频道名>

选项:
  -q, --quality <名称>  仅取指定清晰度的直链（名称可为片段，如 原画）
      --all             取全部清晰度的直链（默认为最高档）
      --top <n>         只显示优选顺序的前 n 条线路
      --url-only        只输出直链本身（每行一条，按优选顺序），便于管道
      --probe           起播前并发探测各线路首字节时延，按实测结果排序
      --no-color        弹幕不输出 ANSI 颜色

示例:
  slive info https://live.bilibili.com/7734200
  slive info https://www.douyu.com/9999 -q 蓝光
  slive danmaku https://live.bilibili.com/6
";

#[tokio::main]
async fn main() {
    // core 用 log:: 输出诊断但不自行安装 logger；CLI 是宿主，必须装一次，
    // 否则弹幕会话里的连接/重连/帧错误全部静默丢失。
    slive_host::logging::ensure_stderr_logger();
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if let Err(message) = dispatch(&argv).await {
        eprintln!("错误：{message}");
        eprintln!("使用 `slive help` 查看用法。");
        std::process::exit(1);
    }
}

async fn dispatch(argv: &[String]) -> Result<(), String> {
    let Some(command) = argv.first().map(|s| s.as_str()) else {
        print!("{HELP}");
        return Ok(());
    };

    match command {
        "-h" | "--help" | "help" => {
            print!("{HELP}");
            return Ok(());
        }
        "info" | "i" | "danmaku" | "d" => {}
        other => return Err(format!("未知命令「{other}」")),
    }

    let mut positional: Vec<&str> = Vec::new();
    let mut quality: Option<String> = None;
    let mut all = false;
    let mut color = true;
    let mut top: Option<usize> = None;
    let mut url_only = false;
    let mut probe = false;

    let mut index = 1;
    while index < argv.len() {
        match argv[index].as_str() {
            "-q" | "--quality" => {
                index += 1;
                quality = Some(argv.get(index).ok_or("`--quality` 缺少参数")?.to_string());
            }
            "--all" => all = true,
            "--top" => {
                index += 1;
                let raw = argv.get(index).ok_or("`--top` 缺少参数")?;
                let parsed = raw.parse::<usize>().map_err(|_| "`--top` 需要正整数")?;
                if parsed == 0 {
                    return Err("`--top` 需要正整数".to_string());
                }
                top = Some(parsed);
            }
            "--url-only" => url_only = true,
            "--probe" => probe = true,
            "--no-color" => color = false,
            "-h" | "--help" => {
                print!("{HELP}");
                return Ok(());
            }
            other if other.starts_with('-') && other.len() > 1 => {
                return Err(format!("未知选项「{other}」"));
            }
            other => positional.push(other),
        }
        index += 1;
    }

    let raw = positional.first().ok_or("缺少直播间 URL")?;
    let target = url::parse(raw)?;

    match command {
        "info" | "i" => info(&target, quality.as_deref(), all, top, url_only, probe).await,
        _ => {
            let site = build_site(target.platform);
            let detail = site
                .get_room_detail(target.room.clone())
                .await
                .map_err(|e| e.to_string())?;
            print_detail(site.as_ref(), &detail);
            if !detail.status {
                return Err("房间未开播，无法获取弹幕".to_string());
            }
            println!("\n【开始获取弹幕】（Ctrl-C 退出）");
            danmaku::run(&target, &detail, color).await
        }
    }
}

async fn info(
    target: &Target,
    quality: Option<&str>,
    all: bool,
    top: Option<usize>,
    url_only: bool,
    probe: bool,
) -> Result<(), String> {
    let site = build_site(target.platform);
    let detail = site
        .get_room_detail(target.room.clone())
        .await
        .map_err(|e| e.to_string())?;
    if !url_only {
        print_detail(site.as_ref(), &detail);
    }

    if !detail.status {
        // `--url-only` 下 stdout 必须只有 URL：诊断走 stderr，
        // 管道消费方才能区分「失败」和「成功但零条线路」。
        if url_only {
            eprintln!("房间未开播，无可用清晰度。");
        } else {
            println!("\n房间未开播，无可用清晰度。");
        }
        return Ok(());
    }

    let qualities = site
        .get_play_qualities(detail.clone())
        .await
        .map_err(|e| e.to_string())?;
    if qualities.is_empty() {
        if url_only {
            eprintln!("无可用清晰度。");
        } else {
            println!("\n无可用清晰度。");
        }
        return Ok(());
    }

    if !url_only {
        println!("\n可用清晰度：");
        for (index, item) in qualities.iter().enumerate() {
            println!("  【{}】{}", index + 1, item.quality);
        }
    }

    let picked: Vec<LivePlayQuality> = if all {
        qualities.clone()
    } else if let Some(want) = quality {
        let exact: Vec<LivePlayQuality> = qualities.iter().filter(|item| item.quality == want).cloned().collect();
        if !exact.is_empty() {
            exact
        } else {
            let fuzzy: Vec<LivePlayQuality> = qualities
                .iter()
                .filter(|item| item.quality.contains(want))
                .cloned()
                .collect();
            if fuzzy.is_empty() {
                return Err(format!("未找到清晰度「{want}」"));
            }
            fuzzy
        }
    } else {
        vec![qualities[0].clone()]
    };

    // 探测器复用同一个 Client：逐清晰度重建会丢掉连接池与 TLS 预热，
    // 让后面的清晰度凭空多付一次握手成本。
    let prober = probe.then(play::HttpProbe::new);

    for item in picked {
        let play = site
            .get_play_urls(detail.clone(), item.clone())
            .await
            .map_err(|e| e.to_string())?;

        // 结构化：只解析 URL，不重发请求。
        let lines = site.play_lines(&play);
        let policy = RankPolicy::default();

        // 可选：并发探测首字节时延，用实测成绩覆盖静态权重。
        let scores: Vec<play::ProbeResult> = match (&prober, lines.is_empty()) {
            (Some(prober), false) => {
                let headers: Vec<(String, String)> = play
                    .headers
                    .as_ref()
                    .map(|map| map.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                    .unwrap_or_default();
                play::probe_all(prober, &lines, &headers, play::DEFAULT_PROBE_BUDGET).await
            }
            _ => Vec::new(),
        };

        // 优选：只重排、不丢弃，`origin_index` 保留平台原始顺序。
        let ranked = play::rank_scored(&lines, &scores, &policy);
        let shown: &[play::PlayLine] = match top {
            Some(n) => &ranked[..n.min(ranked.len())],
            None => &ranked,
        };

        if url_only {
            for line in shown {
                println!("{}", line.url);
            }
            // 直链离不开请求头（B站要 referer、虎牙要 HYSDK UA）：写到 stderr，
            // 既不污染 stdout 的纯 URL 流，又不让管道消费方拿到放不了的地址。
            print_headers_to(&play.headers, true);
            continue;
        }

        let mode = if scores.is_empty() { "按优选顺序" } else { "按实测优选" };
        println!("\n直链（{}，共 {} 条，{mode}）：", item.quality, ranked.len());
        if shown.is_empty() {
            println!("  （无）");
        }
        let probing = !scores.is_empty();
        for (index, line) in shown.iter().enumerate() {
            let score = score_of(&lines, &scores, line);
            println!(
                "  {}. {:<3} {:<8} [原序 {}] {} {}",
                index + 1,
                line.kind.label(),
                hint_label(line.hint),
                line.origin_index + 1,
                latency_label(score, probing),
                line.host
            );
            println!("     {}", line.url);
            if let Some(score) = score {
                if let Some(reason) = score.error.as_ref() {
                    println!("     ↳ 探测失败：{reason}");
                }
            }
        }
        print_headers(&play.headers);
    }

    Ok(())
}

/// 按 `origin_index` 找对应探测成绩（`scores` 与 `lines` 同序）。
///
/// 用 `origin_index` 而不是 URL 字符串：`origin_index` 唯一且与 `probe_all` 的
/// 返回顺序一一对应，不必依赖「URL 不会重复」这个会被策略改动的假设。
fn score_of<'a>(
    lines: &[play::PlayLine],
    scores: &'a [play::ProbeResult],
    line: &play::PlayLine,
) -> Option<&'a play::ProbeResult> {
    if scores.is_empty() {
        return None;
    }
    let index = lines
        .iter()
        .position(|item| item.origin_index == line.origin_index)?;
    scores.get(index)
}

/// 线路标记的 ASCII 标签（CJK 是双宽字符，`{:<6}` 按字符数填充会让列左右漂移）。
fn hint_label(hint: play::LineHint) -> &'static str {
    match hint {
        play::LineHint::Primary => "primary",
        play::LineHint::Mcdn => "mcdn",
        play::LineHint::Scdn => "scdn",
    }
}

/// 探测成绩标签：可用显示首字节时延，失败显示「失败」。
///
/// 未开启探测时返回等宽空白，保证两种模式的列对齐一致。
fn latency_label(score: Option<&play::ProbeResult>, probing: bool) -> String {
    if !probing {
        return format!("{:>8}", "");
    }
    let text = match score {
        Some(score) if score.ok => format!("{}ms", score.first_byte_ms),
        Some(_) => "失败".to_string(),
        None => "-".to_string(),
    };
    format!("{text:>8}")
}

fn print_detail(site: &dyn LiveSite, detail: &LiveRoomDetail) {
    println!("来源：{}", site.name());
    println!("房间号：{}", detail.room_id);
    println!("房间标题：{}", detail.title);
    println!("直播用户：{}", detail.user_name);
    println!("人气值：{}", detail.online);
    println!("状态：{}", if detail.status { "直播中" } else { "未开播" });
}

fn print_headers(headers: &Option<HashMap<String, String>>) {
    print_headers_to(headers, false);
}

/// 打印请求头；`to_stderr` 供 `--url-only` 使用（stdout 只留纯 URL 流）。
fn print_headers_to(headers: &Option<HashMap<String, String>>, to_stderr: bool) {
    let Some(headers) = headers else {
        return;
    };
    if headers.is_empty() {
        return;
    }
    let mut out = String::from("  请求头：\n");
    for (key, value) in headers {
        out.push_str(&format!("    {key}: {value}\n"));
    }
    if to_stderr {
        eprint!("{out}");
    } else {
        print!("{out}");
    }
}

fn build_site(platform: Platform) -> Box<dyn LiveSite> {
    slive_core::platforms::create_site(to_core_platform(platform))
}

/// CLI 的 `Platform`（URL 主机名解析专用）→ core 的 `Platform`（C ABI 稳定判别值）。
fn to_core_platform(p: Platform) -> slive_core::platforms::Platform {
    match p {
        Platform::Bilibili => slive_core::platforms::Platform::Bilibili,
        Platform::Douyu => slive_core::platforms::Platform::Douyu,
        Platform::Huya => slive_core::platforms::Platform::Huya,
        Platform::Douyin => slive_core::platforms::Platform::Douyin,
        Platform::Twitch => slive_core::platforms::Platform::Twitch,
    }
}
