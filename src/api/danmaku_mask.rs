//! 弹幕去重/合并的 FRB 门面：算法本体在 `slive_core::danmaku::mask`（ADR-016）。
//!
//! 本文件只做类型翻译（newtype 包装 + 结果映射），零算法逻辑。
//! 保持 `#[frb(opaque)]` 与全部 `pub` 签名不变 —— 生成物无需重跑 codegen。

use slive_core::danmaku::mask as core_mask;

/// DanmakuMask: 滑动窗口 + 分桶 + 频控。
#[flutter_rust_bridge::frb(opaque)]
pub struct DanmakuMask {
    inner: core_mask::DanmakuMask,
}

#[flutter_rust_bridge::frb(sync)]
impl DanmakuMask {
    pub fn new(
        base_window_ms: u32,
        bucket_count: u16,
        use_normalization: bool,
        use_frequency_control: bool,
        max_frequency: u16,
    ) -> Self {
        Self {
            inner: core_mask::DanmakuMask::new(
                base_window_ms,
                bucket_count,
                use_normalization,
                use_frequency_control,
                max_frequency,
            ),
        }
    }

    /// 重置状态。
    pub fn reset(&mut self) {
        self.inner.reset();
    }

    pub fn dispose(self) {
        self.inner.dispose();
    }
}

/// 合并模式的一组重复弹幕（`merge_list_batch` 的返回项）。
///
/// 与屏蔽模式（`allow_list_batch`）互斥：合并模式不丢弹幕，而是把窗口内的
/// 重复文本聚成一条带计数的展示项，保留直播间的热闹氛围。
pub struct DanmakuMergeGroup {
    /// 归一化文本：窗口内同一 key 的重复都会归到同一条展示项
    pub key: String,
    /// 本批中该组**首次出现**的下标（取该条消息的用户名/颜色/表情做展示）
    pub first_index: u32,
    /// 计入本批后，该 key 在滑动窗口内的累计出现次数
    pub window_count: u32,
    /// 本批之前窗口内没有该 key：调用方应新建展示项；false 表示更新已有展示项
    pub is_new: bool,
}

impl From<core_mask::DanmakuMergeGroup> for DanmakuMergeGroup {
    fn from(g: core_mask::DanmakuMergeGroup) -> Self {
        Self {
            key: g.key,
            first_index: g.first_index,
            window_count: g.window_count,
            is_new: g.is_new,
        }
    }
}

#[flutter_rust_bridge::frb]
impl DanmakuMask {
    /// 批量判断是否允许。返回 `Vec<u8>`：1 = 允许，0 = 屏蔽。
    pub fn allow_list_batch(&mut self, texts: Vec<String>, now_ms: u64) -> Vec<u8> {
        self.inner.allow_list_batch(texts, now_ms)
    }

    /// 合并模式的批量分组：按归一化文本把本批弹幕聚组，并给出窗口累计计数。
    pub fn merge_list_batch(&mut self, texts: Vec<String>, now_ms: u64) -> Vec<DanmakuMergeGroup> {
        self.inner
            .merge_list_batch(texts, now_ms)
            .into_iter()
            .map(DanmakuMergeGroup::from)
            .collect()
    }
}
