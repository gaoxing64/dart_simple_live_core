//! 弹幕文本处理：滑动窗口去重 / 频控 / 重复合并计数。
//!
//! 原实现位于 FRB 桥层（`src/api/danmaku_mask.rs`），此处下沉到纯核心，
//! 使非 Dart 宿主（Rust / C# / C++）也能复用同一套算法。

pub mod mask;
