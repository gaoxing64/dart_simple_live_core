//! 弹幕 C ABI：create / start / stop / free。事件经 dispatcher 线程投递。

use std::os::raw::c_int;

use slive_core::models::{LiveDanmuControlEvent, LiveDanmuItem};
use slive_core::platforms::Platform;
use tokio::sync::watch;

use crate::abi::*;
use crate::dispatcher::{self, Event};
use crate::envelope;
use crate::registry::{self, DanmakuHandle, Entry};

pub const DANMAKU_READY: i32 = 0;
pub const DANMAKU_RECONNECTING: i32 = 1;
pub const DANMAKU_CLOSED: i32 = 2;
pub const DANMAKU_MESSAGE: i32 = 3;

#[no_mangle]
pub extern "C" fn slive_danmaku_create(platform: i32, out: *mut SliveHandle) -> c_int {
    guard(|| {
        if out.is_null() {
            return SLIVE_ERR_INVALID_ARG;
        }
        let Some(platform) = Platform::from_i32(platform) else {
            return SLIVE_ERR_INVALID_ARG;
        };
        let (stop_tx, _stop_rx) = watch::channel(false);
        let id = registry::insert(Entry::Danmaku(DanmakuHandle { stop_tx, platform }));
        unsafe { *out = id };
        SLIVE_OK
    })
}

#[no_mangle]
pub extern "C" fn slive_danmaku_start(handle: SliveHandle, args: SliveStr) -> c_int {
    guard(|| {
        let Ok(args) = (unsafe { from_slive_str(args) }) else {
            return SLIVE_ERR_INVALID_UTF8;
        };
        // 锁内只做快照：平台 + stop 接收端
        let Some((platform, stop_rx)) = registry::with_entry(handle, |e| match e {
            Entry::Danmaku(d) => Some((d.platform, d.stop_tx.subscribe())),
            _ => None,
        })
        .flatten() else {
            return SLIVE_ERR_INVALID_HANDLE;
        };
        let conn = handle;
        let result = slive_host::danmaku::spawn(platform, &args, stop_rx, move |item| {
            let (kind, payload) = match item {
                LiveDanmuItem::Message(m) => (
                    DANMAKU_MESSAGE,
                    serde_json::to_string(&*m).unwrap_or_else(|_| "{}".to_string()),
                ),
                LiveDanmuItem::Control(c) => match c {
                    LiveDanmuControlEvent::Ready => (DANMAKU_READY, "{}".to_string()),
                    LiveDanmuControlEvent::Reconnecting { attempt } => (
                        DANMAKU_RECONNECTING,
                        serde_json::json!({ "attempt": attempt }).to_string(),
                    ),
                    LiveDanmuControlEvent::Closed(reason) => {
                        (DANMAKU_CLOSED, serde_json::json!({ "reason": reason }).to_string())
                    }
                },
            };
            dispatcher::send(Event::Danmaku { conn, kind, payload });
        });
        match result {
            Ok(()) => SLIVE_OK,
            Err(e) => {
                let (k, m, c) = envelope::classify(&e);
                dispatcher::send(Event::Completion {
                    request_id: 0,
                    status: SLIVE_ERR_CORE,
                    payload: envelope::err(k, &m, c),
                });
                SLIVE_ERR_CORE
            }
        }
    })
}

#[no_mangle]
pub extern "C" fn slive_danmaku_stop(handle: SliveHandle) -> c_int {
    guard(|| {
        let sent = registry::with_entry(handle, |e| match e {
            Entry::Danmaku(d) => Some(d.stop_tx.send(true).is_ok()),
            _ => None,
        });
        match sent {
            Some(Some(_)) => SLIVE_OK,
            Some(None) => SLIVE_OK, // 已无接收端（会话已结束）
            None => SLIVE_ERR_INVALID_HANDLE,
        }
    })
}

#[no_mangle]
pub extern "C" fn slive_danmaku_free(handle: SliveHandle) -> c_int {
    guard(|| {
        // 先确认类型再摘除：remove + 回插会让句柄换号
        let is_danmaku =
            registry::with_entry(handle, |e| matches!(e, Entry::Danmaku(_))).unwrap_or(false);
        if !is_danmaku {
            return SLIVE_ERR_INVALID_HANDLE;
        }
        if let Some(Entry::Danmaku(d)) = registry::remove(handle) {
            let _ = d.stop_tx.send(true);
        }
        SLIVE_OK
    })
}
