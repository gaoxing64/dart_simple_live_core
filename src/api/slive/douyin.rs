//! 抖音 FRB 桥：opaque 句柄 `LiveDouyinSite` + 弹幕 Stream。
//!
//! 纪律（AC-39 / 裁决 J-6 / R-13）：本文件**只做类型翻译 + 转发**，零协议逻辑 ——
//! URL 拼接 / 签名 / 解析 / 重连全部在 `slive-core`。异常由 `SliveError` -> `CoreError`
//! 映射后抛出（两路径收敛到同一异常类型，ADR-006）。
//!
//! ⚠️ **真机坑（Spec §13 修复记录，FRB FFI 线程无 tokio runtime）**：
//! `DouyinDanmakuConnection::start` 在 FRB 的 FFI 调用线程上执行，那里**没有**
//! tokio runtime，直接 `tokio::spawn` 会 panic（"there is no reactor running"，
//! 且 panic 被吞掉、表现为「直播开始后没有任何弹幕」）。故弹幕会话走**独立线程 +
//! 自建 current_thread runtime**（单会话 IO，current_thread 足够）。

use flutter_rust_bridge::frb;
use tokio::sync::watch;

use slive_core::platforms::douyin::DouyinSite;

use crate::api::slive::error::CoreError;
use crate::api::slive::models::{
    LiveCategory, LiveCategoryResult, LiveDanmuItem, LivePlayQuality, LivePlayUrl, LiveRoomDetail,
    LiveSearchAnchorResult, LiveSearchRoomResult, LiveSubCategory, LiveSuperChatMessage,
};
use crate::frb_generated::StreamSink;

#[frb(opaque)]
pub struct LiveDouyinSite {
    inner: DouyinSite,
}

impl Default for LiveDouyinSite {
    fn default() -> Self {
        Self::new()
    }
}

impl LiveDouyinSite {
    /// 构造站点句柄（App 每平台单例持有）。
    #[frb(sync)]
    pub fn new() -> Self {
        Self {
            inner: DouyinSite::new(),
        }
    }

    /// 站点 ID（`"douyin"`）。
    #[frb(sync)]
    pub fn id(&self) -> String {
        self.inner.id()
    }

    /// 站点名（`"抖音直播"`）。
    #[frb(sync)]
    pub fn name(&self) -> String {
        self.inner.name()
    }

    /// 注入凭证 / 配置（`{"cookie": "...", "hlsFirst": false}`）。
    #[frb(sync)]
    pub fn set_site_attrs(&self, data: String) -> Result<(), CoreError> {
        self.inner.set_site_attrs(&data).map_err(CoreError::from)
    }

    /// 分类 A 列表。
    pub async fn get_categories(&self) -> Result<Vec<LiveCategory>, CoreError> {
        self.inner.get_categories().await.map_err(CoreError::from)
    }

    /// 分类下房间（`page_size` 服务端不认，保留形参对齐 Dart 签名）。
    pub async fn get_category_rooms(
        &self,
        category: LiveSubCategory,
        page: i32,
        page_size: Option<i32>,
    ) -> Result<LiveCategoryResult, CoreError> {
        self.inner
            .get_category_rooms(category, page, page_size)
            .await
            .map_err(CoreError::from)
    }

    /// 推荐房间。
    pub async fn get_recommend_rooms(
        &self,
        page: i32,
        page_size: Option<i32>,
    ) -> Result<LiveCategoryResult, CoreError> {
        self.inner
            .get_recommend_rooms(page, page_size)
            .await
            .map_err(CoreError::from)
    }

    /// 搜索直播间。
    pub async fn search_rooms(&self, keyword: String, page: i32) -> Result<LiveSearchRoomResult, CoreError> {
        self.inner.search_rooms(keyword, page).await.map_err(CoreError::from)
    }

    /// 搜索主播（抖音不支持，恒报错）。
    pub async fn search_anchors(&self, keyword: String, page: i32) -> Result<LiveSearchAnchorResult, CoreError> {
        self.inner.search_anchors(keyword, page).await.map_err(CoreError::from)
    }

    /// 房间详情（三段降级）。
    pub async fn get_room_detail(&self, room_id: String) -> Result<LiveRoomDetail, CoreError> {
        self.inner.get_room_detail(room_id).await.map_err(CoreError::from)
    }

    /// 清晰度列表（本地计算，无网络请求）。
    pub async fn get_play_qualities(&self, detail: LiveRoomDetail) -> Result<Vec<LivePlayQuality>, CoreError> {
        self.inner.get_play_qualities(detail).await.map_err(CoreError::from)
    }

    /// 播放地址（本地计算，无网络请求）。
    pub async fn get_play_urls(
        &self,
        detail: LiveRoomDetail,
        quality: LivePlayQuality,
    ) -> Result<LivePlayUrl, CoreError> {
        self.inner.get_play_urls(detail, quality).await.map_err(CoreError::from)
    }

    /// 直播状态。
    pub async fn get_live_status(&self, room_id: String) -> Result<bool, CoreError> {
        self.inner.get_live_status(room_id).await.map_err(CoreError::from)
    }

    /// 房间 SC 列表（抖音无此能力，恒空）。
    pub async fn get_super_chat_messages(&self, room_id: String) -> Result<Vec<LiveSuperChatMessage>, CoreError> {
        self.inner
            .get_super_chat_messages(room_id)
            .await
            .map_err(CoreError::from)
    }

    /// 登录二维码（抖音不支持扫码登录）。
    pub async fn get_qr_code(&self) -> Result<crate::api::slive::models::QrCode, CoreError> {
        self.inner.get_qr_code().await.map_err(CoreError::from)
    }

    /// 扫码轮询（抖音不支持扫码登录）。
    pub async fn get_qr_poll_result(
        &self,
        qrcode_key: String,
    ) -> Result<crate::api::slive::models::QrPollResult, CoreError> {
        self.inner.get_qr_poll_result(qrcode_key).await.map_err(CoreError::from)
    }

    /// 账号信息。
    pub async fn get_account_info(
        &self,
        credential: String,
    ) -> Result<crate::api::slive::models::AccountInfo, CoreError> {
        self.inner.get_account_info(credential).await.map_err(CoreError::from)
    }
}

/// 抖音弹幕连接句柄（FRB opaque）：**每实例一份**，持有本会话的停止信号。
///
/// 2026-10-18 修复多开异常：旧实现用全局单例 `active_stop` 登记「当前会话」，
/// 启动第二路会先把第一路的 stop 置 `true` —— 多开时只有最后启动的那路有弹幕，
/// 其余路聊天区显示「已手动停止」。改为句柄自持 stop 后各停各的、互不影响；
/// 句柄被释放（Dart 侧丢弃且未显式 stop）时发送端 drop，会话随句柄一起结束。
#[frb(opaque)]
pub struct DouyinDanmakuConnection {
    stop_tx: watch::Sender<bool>,
}

impl DouyinDanmakuConnection {
    /// 构造连接句柄（尚未连接；`start` 之后才持有会话）。
    #[frb(sync)]
    pub fn new() -> Self {
        let (stop_tx, _stop_rx) = watch::channel(false);
        Self { stop_tx }
    }

    /// 启动抖音弹幕会话：事件推入 stream（FRB 侧表现为 `Stream<LiveDanmuItem>`）。
    ///
    /// `args_json` 是 [`DouyinDanmakuArgs`] 的 JSON 串（snake_case 契约）；解析失败或
    /// 握手签名失败 -> `CoreError`，对应 Dart 侧 `FormatException`（J-10）。
    /// 多路播放各自持句柄调用，会话之间互不停止。
    pub fn start(&self, args_json: String, sink: StreamSink<LiveDanmuItem>) -> Result<(), CoreError> {
        slive_host::logging::ensure_stderr_logger();
        slive_host::danmaku::spawn(
            slive_core::platforms::Platform::Douyin,
            &args_json,
            self.stop_tx.subscribe(),
            move |item| {
                let _ = sink.add(item);
            },
        )
        .map_err(CoreError::from)
    }

    /// 手动停止本会话（对齐 Dart `LiveDanmaku.stop()`：发出 `Closed`，不再重连）。
    #[frb(sync)]
    pub fn stop(&self) {
        let _ = self.stop_tx.send(true);
    }
}
