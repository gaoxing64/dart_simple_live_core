//! 清晰度 / 播放地址模型镜像。

pub use slive_core::models::{LivePlayQuality, LivePlayUrl};
pub use slive_core::play::{LineHint, LineKind, PlayLine};

#[flutter_rust_bridge::frb(mirror(LivePlayQuality))]
pub struct _LivePlayQuality {
    pub quality: String,
    pub data: String,
    pub sort: i32,
}

#[flutter_rust_bridge::frb(mirror(LivePlayUrl))]
pub struct _LivePlayUrl {
    pub urls: Vec<String>,
    pub headers: Option<std::collections::HashMap<String, String>>,
}

#[flutter_rust_bridge::frb(mirror(LineKind))]
pub enum _LineKind {
    Flv,
    Hls,
    Other,
}

#[flutter_rust_bridge::frb(mirror(LineHint))]
pub enum _LineHint {
    Primary,
    Mcdn,
    Scdn,
}

#[flutter_rust_bridge::frb(mirror(PlayLine))]
pub struct _PlayLine {
    pub url: String,
    pub kind: LineKind,
    pub host: String,
    pub hint: LineHint,
    pub origin_index: usize,
}
