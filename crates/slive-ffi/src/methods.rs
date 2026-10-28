//! 方法名 → 具体调用。新增方法只需加一个分支，非破坏性。

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use serde_json::{json, Value};
use slive_core::error::SliveError;
use slive_core::platforms::LiveSite;

use crate::args::*;

pub const METHOD_GET_CATEGORIES: &str = "get_categories";
pub const METHOD_GET_CATEGORY_ROOMS: &str = "get_category_rooms";
pub const METHOD_GET_RECOMMEND_ROOMS: &str = "get_recommend_rooms";
pub const METHOD_SEARCH_ROOMS: &str = "search_rooms";
pub const METHOD_SEARCH_ANCHORS: &str = "search_anchors";
pub const METHOD_GET_ROOM_DETAIL: &str = "get_room_detail";
pub const METHOD_GET_PLAY_QUALITIES: &str = "get_play_qualities";
pub const METHOD_GET_PLAY_URLS: &str = "get_play_urls";
pub const METHOD_GET_LIVE_STATUS: &str = "get_live_status";
pub const METHOD_GET_SUPER_CHAT_MESSAGES: &str = "get_super_chat_messages";
pub const METHOD_GET_QR_CODE: &str = "get_qr_code";
pub const METHOD_GET_QR_POLL_RESULT: &str = "get_qr_poll_result";
pub const METHOD_GET_ACCOUNT_INFO: &str = "get_account_info";

pub const ALL_METHODS: &[&str] = &[
    METHOD_GET_CATEGORIES,
    METHOD_GET_CATEGORY_ROOMS,
    METHOD_GET_RECOMMEND_ROOMS,
    METHOD_SEARCH_ROOMS,
    METHOD_SEARCH_ANCHORS,
    METHOD_GET_ROOM_DETAIL,
    METHOD_GET_PLAY_QUALITIES,
    METHOD_GET_PLAY_URLS,
    METHOD_GET_LIVE_STATUS,
    METHOD_GET_SUPER_CHAT_MESSAGES,
    METHOD_GET_QR_CODE,
    METHOD_GET_QR_POLL_RESULT,
    METHOD_GET_ACCOUNT_INFO,
];

pub type CallFuture = Pin<Box<dyn Future<Output = Result<Value, SliveError>> + Send>>;

fn parse<T: serde::de::DeserializeOwned>(args: &str) -> Result<T, SliveError> {
    serde_json::from_str(args).map_err(|e| SliveError::invalid_input(format!("参数非法: {e}")))
}

fn err_fut(e: SliveError) -> CallFuture {
    Box::pin(async move { Err(e) })
}

fn to_json<T: serde::Serialize>(v: T) -> Result<Value, SliveError> {
    serde_json::to_value(v).map_err(|e| SliveError::Decode(e.to_string()))
}

/// 站点方法分派。`Arc` 移入 async 块，保证 future 为 `'static + Send`。
pub fn dispatch(site: Arc<dyn LiveSite>, method: &str, args: &str) -> Option<CallFuture> {
    Some(match method {
        METHOD_GET_CATEGORIES => Box::pin(async move {
            let v = site.get_categories().await?;
            to_json(v)
        }),
        METHOD_GET_CATEGORY_ROOMS => {
            let a: CategoryRoomsArgs = match parse(args) {
                Ok(v) => v,
                Err(e) => return Some(err_fut(e)),
            };
            Box::pin(async move {
                let v = site.get_category_rooms(a.category, a.page, a.page_size).await?;
                to_json(v)
            })
        }
        METHOD_GET_RECOMMEND_ROOMS => {
            let a: PageArgs = match parse(args) {
                Ok(v) => v,
                Err(e) => return Some(err_fut(e)),
            };
            Box::pin(async move {
                let v = site.get_recommend_rooms(a.page, a.page_size).await?;
                to_json(v)
            })
        }
        METHOD_SEARCH_ROOMS => {
            let a: KeywordArgs = match parse(args) {
                Ok(v) => v,
                Err(e) => return Some(err_fut(e)),
            };
            Box::pin(async move {
                let v = site.search_rooms(a.keyword, a.page).await?;
                to_json(v)
            })
        }
        METHOD_SEARCH_ANCHORS => {
            let a: KeywordArgs = match parse(args) {
                Ok(v) => v,
                Err(e) => return Some(err_fut(e)),
            };
            Box::pin(async move {
                let v = site.search_anchors(a.keyword, a.page).await?;
                to_json(v)
            })
        }
        METHOD_GET_ROOM_DETAIL => {
            let a: RoomIdArgs = match parse(args) {
                Ok(v) => v,
                Err(e) => return Some(err_fut(e)),
            };
            Box::pin(async move {
                let v = site.get_room_detail(a.room_id).await?;
                to_json(v)
            })
        }
        METHOD_GET_PLAY_QUALITIES => {
            let a: DetailArgs = match parse(args) {
                Ok(v) => v,
                Err(e) => return Some(err_fut(e)),
            };
            Box::pin(async move {
                let v = site.get_play_qualities(a.detail).await?;
                to_json(v)
            })
        }
        METHOD_GET_PLAY_URLS => {
            let a: PlayUrlsArgs = match parse(args) {
                Ok(v) => v,
                Err(e) => return Some(err_fut(e)),
            };
            Box::pin(async move {
                let v = site.get_play_urls(a.detail, a.quality).await?;
                to_json(v)
            })
        }
        METHOD_GET_LIVE_STATUS => {
            let a: RoomIdArgs = match parse(args) {
                Ok(v) => v,
                Err(e) => return Some(err_fut(e)),
            };
            Box::pin(async move {
                let v = site.get_live_status(a.room_id).await?;
                Ok(json!(v))
            })
        }
        METHOD_GET_SUPER_CHAT_MESSAGES => {
            let a: RoomIdArgs = match parse(args) {
                Ok(v) => v,
                Err(e) => return Some(err_fut(e)),
            };
            Box::pin(async move {
                let v = site.get_super_chat_messages(a.room_id).await?;
                to_json(v)
            })
        }
        METHOD_GET_QR_CODE => Box::pin(async move {
            let v = site.get_qr_code().await?;
            to_json(v)
        }),
        METHOD_GET_QR_POLL_RESULT => {
            let a: QrKeyArgs = match parse(args) {
                Ok(v) => v,
                Err(e) => return Some(err_fut(e)),
            };
            Box::pin(async move {
                let v = site.get_qr_poll_result(a.qrcode_key).await?;
                to_json(v)
            })
        }
        METHOD_GET_ACCOUNT_INFO => {
            let a: CredentialArgs = match parse(args) {
                Ok(v) => v,
                Err(e) => return Some(err_fut(e)),
            };
            Box::pin(async move {
                let v = site.get_account_info(a.credential).await?;
                to_json(v)
            })
        }
        _ => return None,
    })
}
