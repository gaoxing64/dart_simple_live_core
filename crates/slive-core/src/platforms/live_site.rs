//! 统一站点门面：五个平台的 `*Site` 有同构方法集，这里收敛为单 trait + 工厂，
//! 供 CLI / FRB 桥 / C ABI 复用（ADR-008：只转发，零协议逻辑）。

use async_trait::async_trait;

use crate::error::SliveError;
use crate::models::{
    AccountInfo, LiveCategory, LiveCategoryResult, LivePlayQuality, LivePlayUrl, LiveRoomDetail,
    LiveSearchAnchorResult, LiveSearchRoomResult, LiveSubCategory, LiveSuperChatMessage, QrCode, QrPollResult,
};

/// 平台标识（`repr(i32)` 稳定值，C ABI 亦依赖该数值）。
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Bilibili = 0,
    Douyu = 1,
    Huya = 2,
    Douyin = 3,
    Twitch = 4,
}

impl Platform {
    /// 平台显示名（与各 `*Site::name()` 一致）。
    pub fn name(self) -> &'static str {
        match self {
            Platform::Bilibili => "哔哩哔哩直播",
            Platform::Douyu => "斗鱼直播",
            Platform::Huya => "虎牙直播",
            Platform::Douyin => "抖音直播",
            Platform::Twitch => "Twitch",
        }
    }

    /// 由 C ABI 的整型值还原（非法值返回 `None`）。
    pub fn from_i32(value: i32) -> Option<Self> {
        match value {
            0 => Some(Platform::Bilibili),
            1 => Some(Platform::Douyu),
            2 => Some(Platform::Huya),
            3 => Some(Platform::Douyin),
            4 => Some(Platform::Twitch),
            _ => None,
        }
    }
}

/// 五平台站点的统一视图。
///
/// 只做转发，零协议逻辑（ADR-008）。平台特有方法（虎牙子分类、斗鱼 Cookie 刷新）
/// 仍留在各具体类型上，不进本 trait —— 保证对象安全与工厂返回类型稳定。
#[async_trait]
pub trait LiveSite: Send + Sync {
    fn id(&self) -> String;
    fn name(&self) -> String;
    fn set_site_attrs(&self, data: &str) -> Result<(), SliveError>;

    async fn get_categories(&self) -> Result<Vec<LiveCategory>, SliveError>;
    async fn get_category_rooms(
        &self,
        category: LiveSubCategory,
        page: i32,
        page_size: Option<i32>,
    ) -> Result<LiveCategoryResult, SliveError>;
    async fn get_recommend_rooms(&self, page: i32, page_size: Option<i32>) -> Result<LiveCategoryResult, SliveError>;
    async fn search_rooms(&self, keyword: String, page: i32) -> Result<LiveSearchRoomResult, SliveError>;
    async fn search_anchors(&self, keyword: String, page: i32) -> Result<LiveSearchAnchorResult, SliveError>;
    async fn get_room_detail(&self, room_id: String) -> Result<LiveRoomDetail, SliveError>;
    async fn get_play_qualities(&self, detail: LiveRoomDetail) -> Result<Vec<LivePlayQuality>, SliveError>;
    async fn get_play_urls(&self, detail: LiveRoomDetail, quality: LivePlayQuality) -> Result<LivePlayUrl, SliveError>;

    /// 把 [`get_play_urls`] 的裸 URL 列表还原成结构化线路（容器 / 主机 / 平台标记）。
    ///
    /// 同步、无 I/O：只做纯解析，**不重发请求**，因此可以安全地放在起播路径上。
    /// 默认实现按 `id()` 走通用分类；平台若能在取流阶段拿到更准的语义（例如斗鱼
    /// 的 cdn 名），可覆盖本方法。排序不在这里做 —— 见 [`crate::play::rank`]。
    fn play_lines(&self, play: &LivePlayUrl) -> Vec<crate::play::PlayLine> {
        crate::play::classify(&self.id(), &play.urls)
    }

    async fn get_live_status(&self, room_id: String) -> Result<bool, SliveError>;
    async fn get_super_chat_messages(&self, room_id: String) -> Result<Vec<LiveSuperChatMessage>, SliveError>;
    async fn get_qr_code(&self) -> Result<QrCode, SliveError>;
    async fn get_qr_poll_result(&self, qrcode_key: String) -> Result<QrPollResult, SliveError>;
    async fn get_account_info(&self, credential: String) -> Result<AccountInfo, SliveError>;
}

/// 按平台构造统一站点视图。
pub fn create_site(platform: Platform) -> Box<dyn LiveSite> {
    match platform {
        Platform::Bilibili => Box::new(crate::platforms::bilibili::BilibiliSite::new()),
        Platform::Douyu => Box::new(crate::platforms::douyu::DouyuSite::new()),
        Platform::Huya => Box::new(crate::platforms::huya::HuyaSite::new()),
        Platform::Douyin => Box::new(crate::platforms::douyin::DouyinSite::new()),
        Platform::Twitch => Box::new(crate::platforms::twitch::TwitchSite::new()),
    }
}

/// 全部支持的平台（顺序与 `Platform` 判别值一致）。
pub fn all_platforms() -> &'static [Platform] {
    &[
        Platform::Bilibili,
        Platform::Douyu,
        Platform::Huya,
        Platform::Douyin,
        Platform::Twitch,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_send_sync<T: Send + Sync + ?Sized>() {}

    #[test]
    fn trait_object_is_send_sync() {
        assert_send_sync::<dyn LiveSite>();
    }

    #[test]
    fn platform_roundtrips_through_i32() {
        for &p in all_platforms() {
            assert_eq!(Platform::from_i32(p as i32), Some(p));
        }
        assert_eq!(Platform::from_i32(5), None);
        assert_eq!(Platform::from_i32(-1), None);
    }

    #[test]
    fn factory_builds_all_five_sites() {
        for &p in all_platforms() {
            let site = create_site(p);
            assert!(!site.id().is_empty(), "{p:?} 的 id 不得为空");
            assert_eq!(site.name(), p.name(), "工厂产物的 name 应与 Platform::name 一致");
        }
    }
}
