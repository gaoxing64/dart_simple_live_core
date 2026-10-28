//! 会话线程：FRB 的 FFI 调用线程没有 tokio reactor，`tokio::spawn` 会 panic
//! （"there is no reactor running"，真机日志抓到过）。故每个会话独立线程 + current_thread runtime。

/// 在独立线程中运行一个会话 future（立即返回）。
pub fn spawn_session<F>(name: &'static str, fut: F)
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    std::thread::Builder::new()
        .name(name.to_string())
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("danmaku runtime");
            runtime.block_on(fut);
        })
        .expect("spawn danmaku thread");
}
