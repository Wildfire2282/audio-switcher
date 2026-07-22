namespace AudioSwitcher;

using System.ComponentModel;
using System.Runtime.InteropServices;
using AudioSwitcher.Interop;
using Windows.Win32;
using Windows.Win32.Foundation;
using Windows.Win32.System.Com;

internal static class Program
{
    private const string MutexName = "Global\\AudioSwitcherTray";
    private static readonly IntPtr DpiContextPerMonitorAwareV2 = new(-4);

    [STAThread]
    static void Main()
    {
        // Per-monitor DPI v2 for crisp text on high-DPI displays.
        _ = NativeMethods.SetProcessDpiAwarenessContext(DpiContextPerMonitorAwareV2);

        // STA is required for COM message pumping; Core Audio works fine in STA.
        unsafe
        {
            var hr = PInvoke.CoInitializeEx(null, COINIT.COINIT_APARTMENTTHREADED);
            if (!HResult.Succeeded(hr))
                Environment.Exit(1);
        }

        var mutexHandle = CreateSingleInstanceMutex();
        if (mutexHandle == null)
        {
            PInvoke.CoUninitialize();
            return;
        }

        try
        {
            using var app = new TrayApp();
            app.RunMessageLoop();
        }
        finally
        {
            mutexHandle.Dispose();
            PInvoke.CoUninitialize();
        }
    }

    private static SafeHandle? CreateSingleInstanceMutex()
    {
        try
        {
            var handle = PInvoke.CreateMutex(null, true, MutexName);
            if (handle.IsInvalid)
                return null;

            if (Marshal.GetLastWin32Error() == (int)WIN32_ERROR.ERROR_ALREADY_EXISTS)
            {
                handle.Dispose();
                return null;
            }

            return handle;
        }
        catch (Win32Exception)
        {
            return null;
        }
    }
}
