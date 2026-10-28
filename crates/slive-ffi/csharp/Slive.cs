// Slive.cs — Simple Live C ABI 的 C# P/Invoke 封装（net8.0 / WinUI 3）
//
// 用法：
//   using var rt = new Slive.Runtime(onComplete: (id, status, json) => { ... });
//   using var site = Slive.Site.Create(Slive.Platform.Bilibili);
//   var json = await site.CallAsync("get_room_detail", "{\"room_id\":\"6\"}");
//
// 线程：库内回调在专用线程串行触发。WinUI 3 中请在 onComplete 内
//   DispatcherQueue.TryEnqueue(() => { ... }) 切回 UI 线程。

using System;
using System.Collections.Concurrent;
using System.Runtime.InteropServices;
using System.Text;
using System.Threading;
using System.Threading.Tasks;

namespace Slive;

public enum Platform
{
    Bilibili = 0,
    Douyu = 1,
    Huya = 2,
    Douyin = 3,
    Twitch = 4,
}

public static class Status
{
    public const int Ok = 0;
    public const int Core = 1;
    public const int InvalidHandle = -1;
    public const int InvalidArg = -2;
    public const int InvalidUtf8 = -3;
    public const int NotInit = -4;
    public const int Busy = -5;
    public const int Panic = -6;
    public const int NotFound = -7;
    public const int Cancelled = -8;
    public const int Unsupported = -9;
}

[StructLayout(LayoutKind.Sequential)]
public struct SliveStr
{
    public IntPtr ptr;
    public UIntPtr len;

    public string ToManaged()
    {
        if (ptr == IntPtr.Zero || len == UIntPtr.Zero) return string.Empty;
        return Marshal.PtrToStringUTF8(ptr, checked((int)len.ToUInt32())) ?? string.Empty;
    }
}

[UnmanagedFunctionPointer(CallingConvention.Cdecl)]
public delegate void CompletionFn(ulong requestId, int status, SliveStr payload, IntPtr userData);

[UnmanagedFunctionPointer(CallingConvention.Cdecl)]
public delegate void EventFn(ulong conn, int eventKind, SliveStr payload, IntPtr userData);

[UnmanagedFunctionPointer(CallingConvention.Cdecl)]
public delegate void LogFn(int level, SliveStr target, SliveStr message, IntPtr userData);

[StructLayout(LayoutKind.Sequential)]
public struct SliveConfig
{
    public uint struct_size;
    public uint abi_version;
    public uint worker_threads;
    public uint reserved;
    public IntPtr on_complete;
    public IntPtr on_event;
    public IntPtr on_log;
    public IntPtr user_data;
}

internal static class Native
{
    private const string Lib = "slive_ffi";

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern uint slive_abi_version();

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int slive_init(IntPtr cfg);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int slive_shutdown(uint timeoutMs);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern ulong slive_request_id_new();

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int slive_site_create(int platform, out ulong handle);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int slive_site_free(ulong handle);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int slive_site_id(ulong handle, out SliveStr str);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int slive_site_name(ulong handle, out SliveStr str);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int slive_site_set_attrs(ulong handle, SliveStr json);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int slive_site_call(ulong handle, SliveStr method, SliveStr args, ulong requestId);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int slive_danmaku_create(int platform, out ulong handle);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int slive_danmaku_start(ulong handle, SliveStr args);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int slive_danmaku_stop(ulong handle);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int slive_danmaku_free(ulong handle);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int slive_mask_create(uint windowMs, ushort buckets, byte normalize,
        byte freqControl, ushort maxFreq, out ulong handle);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int slive_mask_allow(ulong handle, SliveStr textsJson, ulong nowMs, out SliveStr outJson);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int slive_mask_merge(ulong handle, SliveStr textsJson, ulong nowMs, out SliveStr outJson);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int slive_mask_reset(ulong handle);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern int slive_mask_free(ulong handle);

    [DllImport(Lib, CallingConvention = CallingConvention.Cdecl)]
    public static extern void slive_str_free(SliveStr str);
}

internal static class Utf8
{
    public static SliveStr ToNative(string s)
    {
        var bytes = Encoding.UTF8.GetBytes(s);
        var ptr = Marshal.AllocHGlobal(bytes.Length);
        Marshal.Copy(bytes, 0, ptr, bytes.Length);
        return new SliveStr { ptr = ptr, len = (UIntPtr)bytes.Length };
    }

    public static void Free(SliveStr s)
    {
        if (s.ptr != IntPtr.Zero) Marshal.FreeHGlobal(s.ptr);
    }

    public static string Take(SliveStr s)
    {
        var managed = s.ToManaged();
        Native.slive_str_free(s);
        return managed;
    }
}

/// 全局运行时（幂等 init）。使用后 Dispose 触发 shutdown。
public sealed class Runtime : IDisposable
{
    private static readonly ConcurrentDictionary<ulong, TaskCompletionSource<string>> Pending = new();
    private static volatile bool _initialized;

    private readonly CompletionFn _onComplete;
    private readonly EventFn? _onEvent;
    private readonly LogFn? _onLog;
    private readonly GCHandle _selfHandle;

    public Runtime(Action<ulong, int, string>? onComplete = null,
                   Action<ulong, int, string>? onEvent = null,
                   Action<int, string, string>? onLog = null,
                   uint workerThreads = 0)
    {
        _onComplete = (id, status, payload, _) =>
        {
            var json = Utf8.Take(payload);
            if (Pending.TryRemove(id, out var tcs)) tcs.TrySetResult(json);
            onComplete?.Invoke(id, status, json);
        };
        _onEvent = onEvent is null ? null : (conn, kind, payload, _) => onEvent(conn, kind, Utf8.Take(payload));
        _onLog = onLog is null ? null : (lv, tgt, msg, _) => onLog(lv, Utf8.Take(tgt), Utf8.Take(msg));

        _selfHandle = GCHandle.Alloc(this);
        var cfg = new SliveConfig
        {
            struct_size = (uint)Marshal.SizeOf<SliveConfig>(),
            abi_version = Native.slive_abi_version(),
            worker_threads = workerThreads,
            on_complete = Marshal.GetFunctionPointerForDelegate(_onComplete),
            on_event = _onEvent is null ? IntPtr.Zero : Marshal.GetFunctionPointerForDelegate(_onEvent),
            on_log = _onLog is null ? IntPtr.Zero : Marshal.GetFunctionPointerForDelegate(_onLog),
            user_data = GCHandle.ToIntPtr(_selfHandle),
        };
        var cfgPtr = Marshal.AllocHGlobal(Marshal.SizeOf<SliveConfig>());
        try
        {
            Marshal.StructureToPtr(cfg, cfgPtr, false);
            var rc = Native.slive_init(cfgPtr);
            if (rc != Status.Ok) throw new InvalidOperationException($"slive_init failed: {rc}");
            _initialized = true;
        }
        finally
        {
            Marshal.FreeHGlobal(cfgPtr);
        }
    }

    internal static bool IsInitialized => _initialized;

    internal static void Register(ulong id, TaskCompletionSource<string> tcs) => Pending[id] = tcs;
    internal static void Unregister(ulong id) => Pending.TryRemove(id, out _);

    public void Dispose()
    {
        Native.slive_shutdown(2000);
        if (_selfHandle.IsAllocated) _selfHandle.Free();
        GC.KeepAlive(_onComplete);
        GC.KeepAlive(_onEvent);
        GC.KeepAlive(_onLog);
    }
}

/// 站点句柄。
public sealed class Site : IDisposable
{
    private ulong _handle;

    private Site(ulong handle) => _handle = handle;

    public static Site Create(Platform platform)
    {
        var rc = Native.slive_site_create((int)platform, out var h);
        if (rc != Status.Ok) throw new InvalidOperationException($"slive_site_create failed: {rc}");
        return new Site(h);
    }

    public string Id => ReadString(Native.slive_site_id);
    public string Name => ReadString(Native.slive_site_name);

    public void SetAttrs(string json)
    {
        var s = Utf8.ToNative(json);
        try
        {
            var rc = Native.slive_site_set_attrs(_handle, s);
            if (rc != Status.Ok) throw new InvalidOperationException($"set_attrs failed: {rc}");
        }
        finally { Utf8.Free(s); }
    }

    public async Task<string> CallAsync(string method, string argsJson, CancellationToken ct = default)
    {
        if (!Runtime.IsInitialized)
            throw new InvalidOperationException("先构造 Slive.Runtime，再调用 CallAsync");

        var id = Native.slive_request_id_new();
        var tcs = new TaskCompletionSource<string>(TaskCreationOptions.RunContinuationsAsynchronously);
        Runtime.Register(id, tcs);

        var m = Utf8.ToNative(method);
        var a = Utf8.ToNative(argsJson);
        try
        {
            var rc = Native.slive_site_call(_handle, m, a, id);
            if (rc < 0)
            {
                Runtime.Unregister(id);
                throw new InvalidOperationException($"slive_site_call rejected: {rc}");
            }
        }
        finally
        {
            Utf8.Free(m);
            Utf8.Free(a);
        }

        using var reg = ct.Register(() => tcs.TrySetCanceled(ct));
        return await tcs.Task.ConfigureAwait(false);
    }

    private delegate int SiteStrFn(ulong handle, out SliveStr str);

    private string ReadString(SiteStrFn fn)
    {
        var rc = fn(_handle, out var s);
        return rc == Status.Ok ? Utf8.Take(s) : string.Empty;
    }

    public void Dispose()
    {
        if (_handle != 0)
        {
            Native.slive_site_free(_handle);
            _handle = 0;
        }
    }
}

/// 弹幕去重/合并。
public sealed class Mask : IDisposable
{
    private ulong _handle;

    public Mask(uint windowMs, ushort buckets, bool normalize, bool freqControl, ushort maxFreq)
    {
        var rc = Native.slive_mask_create(windowMs, buckets, (byte)(normalize ? 1 : 0),
            (byte)(freqControl ? 1 : 0), maxFreq, out _handle);
        if (rc != Status.Ok) throw new InvalidOperationException($"slive_mask_create failed: {rc}");
    }

    public string Allow(string textsJson, ulong nowMs) => Batch(Native.slive_mask_allow, textsJson, nowMs);
    public string Merge(string textsJson, ulong nowMs) => Batch(Native.slive_mask_merge, textsJson, nowMs);

    private delegate int BatchFn(ulong h, SliveStr texts, ulong now, out SliveStr output);

    private string Batch(BatchFn fn, string texts, ulong now)
    {
        var s = Utf8.ToNative(texts);
        try
        {
            var rc = fn(_handle, s, now, out var outJson);
            return rc == Status.Ok ? Utf8.Take(outJson) : string.Empty;
        }
        finally { Utf8.Free(s); }
    }

    public void Dispose()
    {
        if (_handle != 0)
        {
            Native.slive_mask_free(_handle);
            _handle = 0;
        }
    }
}
