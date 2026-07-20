namespace AudioSwitcher.Interop;

using System.Runtime.InteropServices;

internal static partial class NativeMethods
{
    /// <summary>CoCreateInstance without COM marshalling — AOT-safe.</summary>
    [LibraryImport("ole32.dll")]
    internal static partial int CoCreateInstance(
        in Guid rclsid,
        IntPtr pUnkOuter,
        uint dwClsContext,
        in Guid riid,
        out IntPtr ppv);

    /// <summary>Free a task-allocated COM string.</summary>
    [LibraryImport("ole32.dll")]
    internal static partial void CoTaskMemFree(IntPtr ptr);

    /// <summary>Per-monitor DPI v2 awareness (crisp text on 4K).</summary>
    [LibraryImport("user32.dll")]
    internal static partial int SetProcessDpiAwarenessContext(IntPtr dpiContext);
}
