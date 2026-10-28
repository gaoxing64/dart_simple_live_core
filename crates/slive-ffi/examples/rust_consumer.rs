//! 纯 Rust 消费示例：不经过 C ABI，直接依赖 slive-core / slive-host。
//!
//! 用法：`cargo run -p slive-ffi --example rust_consumer -- 6`
//! 演示：房间详情 → 清晰度 → 取流 → 10 秒弹幕 → 去重/合并计数。

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use slive_core::danmaku::mask::DanmakuMask;
use slive_core::platforms::{create_site, Platform};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let room = std::env::args().nth(1).unwrap_or_else(|| "6".to_string());
    let site = create_site(Platform::Bilibili);
    println!("站点：{} ({})", site.name(), site.id());

    let detail = site.get_room_detail(room.clone()).await?;
    println!("房间：{} / {}", detail.title, detail.user_name);
    println!("状态：{}", if detail.status { "直播中" } else { "未开播" });

    if !detail.status {
        println!("未开播，跳过取流与弹幕。");
        return Ok(());
    }

    let qualities = site.get_play_qualities(detail.clone()).await?;
    println!("清晰度：{} 项", qualities.len());
    if let Some(q) = qualities.first() {
        let play = site.get_play_urls(detail.clone(), q.clone()).await?;
        println!("「{}」地址 {} 条", q.quality, play.urls.len());
    }

    // 弹幕 10 秒 + 合并计数演示
    let args_json = detail.danmaku_data.clone().ok_or("缺少 danmaku_data")?;
    let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);

    let mask = Arc::new(std::sync::Mutex::new(DanmakuMask::new(10_000, 10, true, false, 3)));
    let seen = Arc::new(AtomicUsize::new(0));
    let seen_cb = Arc::clone(&seen);
    let mask_cb = Arc::clone(&mask);

    let start_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as u64;

    // 注意：spawn 内部自建 runtime 线程，故这里用 tokio 主 runtime 等待即可。
    slive_host::danmaku::spawn(Platform::Bilibili, &args_json, stop_rx, move |item| {
        if let slive_core::models::LiveDanmuItem::Message(m) = item {
            if !m.message.is_empty() {
                seen_cb.fetch_add(1, Ordering::SeqCst);
                let now = start_ms + 1;
                let groups = mask_cb
                    .lock()
                    .expect("mask")
                    .merge_list_batch(vec![m.message.clone()], now);
                if let Some(g) = groups.first() {
                    if g.is_new {
                        println!("[弹幕] {} (x{})", g.key, g.window_count);
                    }
                }
            }
        }
    })?;

    tokio::time::sleep(Duration::from_secs(10)).await;
    let _ = stop_tx.send(true);
    tokio::time::sleep(Duration::from_millis(200)).await;

    println!("10 秒内收到 {} 条文本弹幕", seen.load(Ordering::SeqCst));
    Ok(())
}
