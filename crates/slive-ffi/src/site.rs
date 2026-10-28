//! 站点 C ABI：create / free / id / name / set_attrs / call（异步，回调投递）。
//!
//! 纪律：所有 `extern "C"` 函数体包 `guard`；所有字符串用 `SliveStr`；
//! 回调一律经 dispatcher 线程投递（绝不在 tokio worker 上直接调宿主回调）。

use std::os::raw::c_int;
use std::sync::Arc;

use slive_core::platforms::{create_site, LiveSite, Platform};

use crate::abi::*;
use crate::dispatcher::{self, Event};
use crate::envelope;
use crate::registry::{self, Entry};
use crate::runtime;

#[no_mangle]
pub extern "C" fn slive_site_create(platform: i32, out: *mut SliveHandle) -> c_int {
    guard(|| {
        if out.is_null() {
            return SLIVE_ERR_INVALID_ARG;
        }
        let Some(p) = Platform::from_i32(platform) else {
            return SLIVE_ERR_INVALID_ARG;
        };
        let site: Arc<dyn LiveSite> = Arc::from(create_site(p));
        let id = registry::insert(Entry::Site(site));
        unsafe { *out = id };
        SLIVE_OK
    })
}

#[no_mangle]
pub extern "C" fn slive_site_free(handle: SliveHandle) -> c_int {
    guard(|| {
        // 先确认类型再摘除：直接 remove + 回插会让句柄换号，调用方后续持有的是旧号。
        let is_site = registry::with_entry(handle, |e| matches!(e, Entry::Site(_))).unwrap_or(false);
        if !is_site {
            return SLIVE_ERR_INVALID_HANDLE;
        }
        registry::remove(handle);
        SLIVE_OK
    })
}

fn site_string(handle: SliveHandle, out: *mut SliveStr, f: fn(&dyn LiveSite) -> String) -> c_int {
    if out.is_null() {
        return SLIVE_ERR_INVALID_ARG;
    }
    let Some(site) = registry::clone_site(handle) else {
        return SLIVE_ERR_INVALID_HANDLE;
    };
    let s = f(&*site);
    unsafe { *out = to_slive_str(&s) };
    SLIVE_OK
}

#[no_mangle]
pub extern "C" fn slive_site_id(handle: SliveHandle, out: *mut SliveStr) -> c_int {
    guard(|| site_string(handle, out, |s| s.id()))
}

#[no_mangle]
pub extern "C" fn slive_site_name(handle: SliveHandle, out: *mut SliveStr) -> c_int {
    guard(|| site_string(handle, out, |s| s.name()))
}

#[no_mangle]
pub extern "C" fn slive_site_set_attrs(handle: SliveHandle, json: SliveStr) -> c_int {
    guard(|| {
        let Ok(data) = (unsafe { from_slive_str(json) }) else {
            return SLIVE_ERR_INVALID_UTF8;
        };
        let Some(site) = registry::clone_site(handle) else {
            return SLIVE_ERR_INVALID_HANDLE;
        };
        // 锁外调用（clone_site 已释放注册表锁）
        match site.set_site_attrs(&data) {
            Ok(()) => SLIVE_OK,
            Err(e) => {
                let (kind, msg, code) = envelope::classify(&e);
                dispatcher::send(Event::Completion {
                    request_id: 0,
                    status: SLIVE_ERR_CORE,
                    payload: envelope::err(kind, &msg, code),
                });
                SLIVE_ERR_CORE
            }
        }
    })
}

/// 异步站点调用：`0` = 已受理（保证恰好一次 completion）；`<0` = 拒绝（永无回调）。
#[no_mangle]
pub extern "C" fn slive_site_call(
    handle: SliveHandle,
    method: SliveStr,
    args_json: SliveStr,
    request_id: u64,
) -> c_int {
    guard(|| {
        let Ok(method) = (unsafe { from_slive_str(method) }) else {
            return SLIVE_ERR_INVALID_UTF8;
        };
        let Ok(args) = (unsafe { from_slive_str(args_json) }) else {
            return SLIVE_ERR_INVALID_UTF8;
        };
        let Some(site) = registry::clone_site(handle) else {
            return SLIVE_ERR_INVALID_HANDLE;
        };
        let Some(fut) = crate::methods::dispatch(site, &method, &args) else {
            return SLIVE_ERR_UNSUPPORTED;
        };
        let Some(rt) = runtime::handle() else {
            return SLIVE_ERR_NOT_INIT;
        };
        // 登记在途请求：重复 ID → BUSY；失败路径不登记，故永不回调
        let Some(mut cancel_rx) = crate::requests::register(request_id) else {
            return SLIVE_ERR_BUSY;
        };
        // 任务体也包 catch_unwind：panic 会被 JoinHandle 吞，导致宿主 promise 永不完成
        rt.spawn(async move {
            use futures_util::FutureExt;
            let payload = tokio::select! {
                biased;
                r = std::panic::AssertUnwindSafe(fut).catch_unwind() => match r {
                    Ok(Ok(v)) => envelope::ok(v),
                    Ok(Err(e)) => {
                        let (k, m, c) = envelope::classify(&e);
                        envelope::err(k, &m, c)
                    }
                    Err(_) => envelope::err("internal", "任务 panic", 0),
                },
                _ = cancel_rx.changed() => {
                    envelope::err("cancelled", "请求已取消", SLIVE_ERR_CANCELLED)
                }
            };
            crate::requests::finish(request_id);
            dispatcher::send(Event::Completion {
                request_id,
                status: SLIVE_OK,
                payload,
            });
        });
        SLIVE_OK
    })
}

#[no_mangle]
pub extern "C" fn slive_str_free(s: SliveStr) {
    unsafe { free_slive_str(s) }
}
