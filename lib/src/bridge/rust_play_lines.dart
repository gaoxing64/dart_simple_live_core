//! 播放线路优选门面。
//!
//! 纪律（同其余 bridge）：只做类型翻译与转发 —— 容器判定、mcdn/scdn 降级、
//! 探测与排序策略全在 `slive_core::play`（Rust），本层不得出现任何规则判断。

import 'package:simple_live_core/src/common/core_log.dart';
import 'package:simple_live_core/src/rust/api/slive/models/play.dart' as rust_models;
import 'package:simple_live_core/src/rust/api/slive/play.dart' as rust_play;

/// 线路容器类型。
enum LiveLineKind {
  /// HTTP-FLV（起播快、延迟低）。
  flv,

  /// HLS（m3u8）。
  hls,

  /// 无法判定（排在两类之后）。
  other,
}

/// 线路平台标记。
enum LiveLineHint {
  /// 平台主推线路。
  primary,

  /// B站 mcdn（易失败，默认降级）。
  mcdn,

  /// 斗鱼 scdn（易失败，默认降级）。
  scdn,
}

/// 一条结构化播放线路（Dart 视图）。
class LivePlayLine {
  const LivePlayLine({
    required this.url,
    required this.host,
    required this.kind,
    required this.hint,
    required this.originIndex,
  });

  /// 直链（原样，不做任何改写）。
  final String url;

  /// 主机（小写，已去端口）。
  final String host;

  /// 容器类型。
  final LiveLineKind kind;

  /// 平台标记。
  final LiveLineHint hint;

  /// 平台原始顺序下标（0 基）。
  final int originIndex;

  /// 平台标记为降级的线路。
  bool get isDemoted => hint != LiveLineHint.primary;

  /// 列表展示用的短标签。
  String get kindLabel => switch (kind) {
        LiveLineKind.flv => "FLV",
        LiveLineKind.hls => "HLS",
        LiveLineKind.other => "未知",
      };

  @override
  String toString() => "$host ${kind.name} ${hint.name} #$originIndex";
}

/// 线路优选入口。
class PlayLineRanker {
  const PlayLineRanker._();

  /// 静态优选：零网络、零延迟。
  ///
  /// 规则（在 Rust 侧）：FLV 优先、mcdn/scdn 降级、只重排不丢弃。
  /// 起播路径默认走这个 —— 不引入任何额外时延。
  static List<LivePlayLine> rank({
    required String platformId,
    required List<String> urls,
    bool preferHls = false,
  }) {
    if (urls.isEmpty) {
      return const [];
    }
    return rust_play
        .rankPlayLines(
          platformId: platformId,
          urls: urls,
          preferHls: preferHls,
        )
        .map(_toDart)
        .toList(growable: false);
  }

  /// 实测优选：并发探测各线路首字节时延后排序（失败项由 Rust 侧复核一次）。
  ///
  /// [budgetMs] 是**单条线路**预算，全部线路并发。注意 Rust 侧对失败线路还会跑
  /// 一轮复核，所以最坏耗时约 `budgetMs + min(budgetMs, 600)`；实际取值会被夹在
  /// `[200, 3000]`（低于 200 会按 200 执行）。
  /// 探测整体失败时退回静态序 —— 优选不能成为起播失败的原因。
  static Future<List<LivePlayLine>> probe({
    required String platformId,
    required List<String> urls,
    Map<String, String>? headers,
    int budgetMs = 600,
    bool preferHls = false,
  }) async {
    if (urls.isEmpty) {
      return const [];
    }
    // budgetMs <= 0 是「关闭探测」的开关：直接走零延迟静态序。
    // 只有一条线路时没有可比较的对象，探测纯属浪费起播时间。
    if (budgetMs <= 0 || urls.length == 1) {
      return rank(platformId: platformId, urls: urls, preferHls: preferHls);
    }
    try {
      final lines = await rust_play.probePlayLines(
        platformId: platformId,
        urls: urls,
        headers: headers ?? const {},
        budgetMs: budgetMs,
        preferHls: preferHls,
      );
      if (lines.isEmpty) {
        return rank(platformId: platformId, urls: urls, preferHls: preferHls);
      }
      return lines.map(_toDart).toList(growable: false);
    } catch (e) {
      // 只有 FFI 层故障才会走到这里（网络失败在 Rust 侧已降级成静态序）。
      // 兜底本身也是 FFI 调用，同样可能失败，所以再包一层 —— 最坏返回空表，
      // 由调用方（App）退回平台原始顺序，绝不让优选成为起播失败的原因。
      CoreLog.error("线路探测失败，退回静态优选: $e");
      try {
        return rank(platformId: platformId, urls: urls, preferHls: preferHls);
      } catch (e2) {
        CoreLog.error("静态优选也失败，退回平台原始顺序: $e2");
        return const [];
      }
    }
  }

  /// 从优选结果里取主机顺序（续流时回传给 [reorder]）。
  ///
  /// 不要手搓：`reorder` 只认 [LivePlayLine.host] 这种形态（小写、无端口），
  /// 传 `host:port` 或整条 URL 会全部匹配不上、静默退化成原序。
  static List<String> hostOrderOf(List<LivePlayLine> lines) =>
      lines.map((line) => line.host).toList(growable: false);

  /// 按主机顺序重排刷新后的直链（续流用）。
  ///
  /// 直链刷新后字符串全变、主机稳定，靠它保证「第 N 条线路」仍指向同一条 CDN。
  /// [hostOrder] 必须是 [LivePlayLine.host] 形态（用 [hostOrderOf] 生成）；
  /// 未命中的主机保持原有相对顺序排到最后，不会丢弃任何线路。
  static List<String> reorder({
    required List<String> urls,
    required List<String> hostOrder,
  }) {
    if (urls.isEmpty || hostOrder.isEmpty) {
      return urls;
    }
    return rust_play.reorderPlayUrls(urls: urls, hostOrder: hostOrder);
  }

  static LivePlayLine _toDart(rust_models.PlayLine line) => LivePlayLine(
        url: line.url,
        host: line.host,
        kind: switch (line.kind) {
          rust_models.LineKind.flv => LiveLineKind.flv,
          rust_models.LineKind.hls => LiveLineKind.hls,
          rust_models.LineKind.other => LiveLineKind.other,
        },
        hint: switch (line.hint) {
          rust_models.LineHint.primary => LiveLineHint.primary,
          rust_models.LineHint.mcdn => LiveLineHint.mcdn,
          rust_models.LineHint.scdn => LiveLineHint.scdn,
        },
        originIndex: line.originIndex.toInt(),
      );
}
