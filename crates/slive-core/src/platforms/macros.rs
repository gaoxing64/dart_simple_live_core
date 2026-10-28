/// 为具体站点生成 `LiveSite` 的 16 个共享方法转发（仅共享方法；平台特有方法不进 trait）。
macro_rules! impl_live_site {
    ($site:ty) => {
        #[async_trait::async_trait]
        impl $crate::platforms::live_site::LiveSite for $site {
            fn id(&self) -> String {
                <$site>::id(self)
            }
            fn name(&self) -> String {
                <$site>::name(self)
            }
            fn set_site_attrs(&self, data: &str) -> Result<(), $crate::error::SliveError> {
                <$site>::set_site_attrs(self, data)
            }
            async fn get_categories(&self) -> Result<Vec<$crate::models::LiveCategory>, $crate::error::SliveError> {
                <$site>::get_categories(self).await
            }
            async fn get_category_rooms(
                &self,
                category: $crate::models::LiveSubCategory,
                page: i32,
                page_size: Option<i32>,
            ) -> Result<$crate::models::LiveCategoryResult, $crate::error::SliveError> {
                <$site>::get_category_rooms(self, category, page, page_size).await
            }
            async fn get_recommend_rooms(
                &self,
                page: i32,
                page_size: Option<i32>,
            ) -> Result<$crate::models::LiveCategoryResult, $crate::error::SliveError> {
                <$site>::get_recommend_rooms(self, page, page_size).await
            }
            async fn search_rooms(
                &self,
                keyword: String,
                page: i32,
            ) -> Result<$crate::models::LiveSearchRoomResult, $crate::error::SliveError> {
                <$site>::search_rooms(self, keyword, page).await
            }
            async fn search_anchors(
                &self,
                keyword: String,
                page: i32,
            ) -> Result<$crate::models::LiveSearchAnchorResult, $crate::error::SliveError> {
                <$site>::search_anchors(self, keyword, page).await
            }
            async fn get_room_detail(
                &self,
                room_id: String,
            ) -> Result<$crate::models::LiveRoomDetail, $crate::error::SliveError> {
                <$site>::get_room_detail(self, room_id).await
            }
            async fn get_play_qualities(
                &self,
                detail: $crate::models::LiveRoomDetail,
            ) -> Result<Vec<$crate::models::LivePlayQuality>, $crate::error::SliveError> {
                <$site>::get_play_qualities(self, detail).await
            }
            async fn get_play_urls(
                &self,
                detail: $crate::models::LiveRoomDetail,
                quality: $crate::models::LivePlayQuality,
            ) -> Result<$crate::models::LivePlayUrl, $crate::error::SliveError> {
                <$site>::get_play_urls(self, detail, quality).await
            }
            async fn get_live_status(&self, room_id: String) -> Result<bool, $crate::error::SliveError> {
                <$site>::get_live_status(self, room_id).await
            }
            async fn get_super_chat_messages(
                &self,
                room_id: String,
            ) -> Result<Vec<$crate::models::LiveSuperChatMessage>, $crate::error::SliveError> {
                <$site>::get_super_chat_messages(self, room_id).await
            }
            async fn get_qr_code(&self) -> Result<$crate::models::QrCode, $crate::error::SliveError> {
                <$site>::get_qr_code(self).await
            }
            async fn get_qr_poll_result(
                &self,
                qrcode_key: String,
            ) -> Result<$crate::models::QrPollResult, $crate::error::SliveError> {
                <$site>::get_qr_poll_result(self, qrcode_key).await
            }
            async fn get_account_info(
                &self,
                credential: String,
            ) -> Result<$crate::models::AccountInfo, $crate::error::SliveError> {
                <$site>::get_account_info(self, credential).await
            }
        }
    };
}

pub(crate) use impl_live_site;
