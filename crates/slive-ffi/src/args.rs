//! 每个方法的命名参数（`deny_unknown_fields`：拼错的键必须报错，不静默忽略）。

use serde::Deserialize;
use slive_core::models::{LivePlayQuality, LiveRoomDetail, LiveSubCategory};

fn one() -> i32 {
    1
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoomIdArgs {
    pub room_id: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeywordArgs {
    pub keyword: String,
    #[serde(default = "one")]
    pub page: i32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageArgs {
    #[serde(default = "one")]
    pub page: i32,
    pub page_size: Option<i32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CategoryRoomsArgs {
    pub category: LiveSubCategory,
    #[serde(default = "one")]
    pub page: i32,
    pub page_size: Option<i32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetailArgs {
    pub detail: LiveRoomDetail,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlayUrlsArgs {
    pub detail: LiveRoomDetail,
    pub quality: LivePlayQuality,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialArgs {
    pub credential: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QrKeyArgs {
    pub qrcode_key: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextsArgs {
    pub texts: Vec<String>,
}
