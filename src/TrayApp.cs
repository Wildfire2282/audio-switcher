namespace AudioSwitcher;

using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using AudioSwitcher.Interop;
using Windows.Win32;
using Windows.Win32.Foundation;
using Windows.Win32.Graphics.Gdi;
using Windows.Win32.UI.Shell;
using Windows.Win32.UI.WindowsAndMessaging;

internal sealed class TrayApp : IDisposable
{
    private const uint TrayMessageId = 0x8000;
    private const uint IconId = 1;
    private const int WheelDelta = 120;
    private const int FirstDeviceId = 1000;

    private static TrayApp? _instance;
    private static HOOKPROC? _mouseHookProc;

    private readonly string _className;
    private HWND _hwnd;
    private HHOOK _hookHandle;
    private HICON _icon;
    public TrayApp()
    {
        _instance = this;
        _className = $"AudioSwitcherTray_{Guid.NewGuid():N}";

        RegisterWindowClass();
        CreateMessageWindow();
        _icon = LoadIcon(ThemeDetector.Detect());
        AddTrayIcon();
        InstallWheelHook();
    }

    public void RunMessageLoop()
    {
        while (PInvoke.GetMessage(out var msg, HWND.Null, 0, 0))
        {
            _ = PInvoke.TranslateMessage(in msg);
            PInvoke.DispatchMessage(in msg);
        }
    }

    public void Dispose()
    {
        RemoveTrayIcon();


        if (!_hookHandle.IsNull)
        {
            PInvoke.UnhookWindowsHookEx(_hookHandle);
            _hookHandle = default;
        }
        _mouseHookProc = null;

        if (!_icon.IsNull)
        {
            PInvoke.DestroyIcon(_icon);
            _icon = default;
        }

        if (!_hwnd.IsNull)
        {
            PInvoke.DestroyWindow(_hwnd);
            _hwnd = default;
        }

        unsafe
        {
            fixed (char* pClassName = _className)
            {
                _ = PInvoke.UnregisterClass(pClassName, PInvoke.GetModuleHandle((char*)null));
            }
        }

        _instance = null;
    }

    private void RegisterWindowClass()
    {
        unsafe
        {
            fixed (char* pClassName = _className)
            {
                var wcex = new WNDCLASSEXW
                {
                    cbSize = (uint)Marshal.SizeOf<WNDCLASSEXW>(),
                    lpfnWndProc = StaticWindowProc,
                    hInstance = PInvoke.GetModuleHandle((char*)null),
                    lpszClassName = pClassName,
                    hCursor = PInvoke.LoadCursor(HINSTANCE.Null, PInvoke.IDC_ARROW),
                };

                var atom = PInvoke.RegisterClassEx(in wcex);
                if (atom == 0)
                    Marshal.ThrowExceptionForHR(Marshal.GetHRForLastWin32Error());
            }
        }
    }

    private void CreateMessageWindow()
    {
        unsafe
        {
            var hInstance = PInvoke.GetModuleHandle((char*)null);
            using var hInstanceSafe = new Microsoft.Win32.SafeHandles.SafeFileHandle((nint)hInstance.Value, ownsHandle: false);

            _hwnd = PInvoke.CreateWindowEx(
                WINDOW_EX_STYLE.WS_EX_NOACTIVATE,
                _className,
                "AudioSwitcher",
                WINDOW_STYLE.WS_OVERLAPPED,
                0, 0, 0, 0,
                new HWND(-3),
                null,
                hInstanceSafe,
                null);
        }

        if (_hwnd.IsNull)
            Marshal.ThrowExceptionForHR(Marshal.GetHRForLastWin32Error());
    }

    private void AddTrayIcon()
    {
        var data = new NOTIFYICONDATAW
        {
            cbSize = (uint)Marshal.SizeOf<NOTIFYICONDATAW>(),
            hWnd = _hwnd,
            uID = IconId,
            uFlags = NOTIFY_ICON_DATA_FLAGS.NIF_MESSAGE | NOTIFY_ICON_DATA_FLAGS.NIF_ICON | NOTIFY_ICON_DATA_FLAGS.NIF_TIP | NOTIFY_ICON_DATA_FLAGS.NIF_SHOWTIP,
            uCallbackMessage = TrayMessageId,
            hIcon = _icon,
        };
        CopyToTip(ref data, Locale.T("音频输出设备切换", "Audio device switcher"));

        if (!PInvoke.Shell_NotifyIcon(NOTIFY_ICON_MESSAGE.NIM_ADD, in data))
            Marshal.ThrowExceptionForHR(Marshal.GetHRForLastWin32Error());
    }

    private void RemoveTrayIcon()
    {
        unsafe
        {
            var data = new NOTIFYICONDATAW
            {
                cbSize = (uint)Marshal.SizeOf<NOTIFYICONDATAW>(),
                hWnd = _hwnd,
                uID = IconId,
            };
            _ = PInvoke.Shell_NotifyIcon(NOTIFY_ICON_MESSAGE.NIM_DELETE, in data);
        }
    }

    private void InstallWheelHook()
    {
        _mouseHookProc = MouseHookProc;
        unsafe
        {
            var module = PInvoke.GetModuleHandle((char*)null);
            _hookHandle = PInvoke.SetWindowsHookEx(
                WINDOWS_HOOK_ID.WH_MOUSE_LL,
                _mouseHookProc,
                module,
                0);
        }

        if (_hookHandle.IsNull)
            Marshal.ThrowExceptionForHR(Marshal.GetHRForLastWin32Error());
    }

    private LRESULT MouseHookProc(int nCode, WPARAM wParam, LPARAM lParam)
    {
        if (nCode >= 0)
        {
            var msg = (uint)wParam.Value;
            if (msg == PInvoke.WM_MOUSEWHEEL)
            {
                unsafe
                {
                    var info = (MSLLHOOKSTRUCT*)lParam.Value;
                    if (IsCursorOverIcon(info->pt))
                    {
                        var delta = (short)((info->mouseData >> 16) & 0xFFFF);
                        var notches = delta / WheelDelta;
                        if (notches != 0)
                        {
                            try
                            {
                                AudioManager.AdjustVolume(notches);
                                if (AudioManager.GetVolumeScalar() is float volume)
                                {
                                    var pct = (int)Math.Round(volume * 100);
                                    UpdateTooltip(BuildVolumeTooltip(pct));
                                }
                            }
                            catch { /* ignore */ }
                        }
                    }
                }
            }
        }

        return PInvoke.CallNextHookEx(null, nCode, wParam, lParam);
    }

    private static string BuildVolumeTooltip(int pct)
    {
        var name = Locale.T("音频输出", "Audio output");
        var defaultId = AudioManager.GetCurrentDefaultId();
        if (defaultId != null)
        {
            var device = AudioManager.EnumerateRenderEndpoints()
                .FirstOrDefault(d => d.Id == defaultId);
            if (device != null)
                name = device.Name;
        }
        return $"{name} - {pct}%";
    }

    private void UpdateTooltip(string text)
    {
        var data = new NOTIFYICONDATAW
        {
            cbSize = (uint)Marshal.SizeOf<NOTIFYICONDATAW>(),
            hWnd = _hwnd,
            uID = IconId,
            uFlags = NOTIFY_ICON_DATA_FLAGS.NIF_TIP | NOTIFY_ICON_DATA_FLAGS.NIF_SHOWTIP,
        };
        CopyToTip(ref data, text);

        _ = PInvoke.Shell_NotifyIcon(NOTIFY_ICON_MESSAGE.NIM_MODIFY, in data);
    }

    private static void CopyToTip(ref NOTIFYICONDATAW data, string text)
    {
        var tip = text[..Math.Min(text.Length, 127)];
        for (var i = 0; i < tip.Length; i++)
            data.szTip[i] = tip[i];
    }

    private unsafe void ShowContextMenu()
    {
        var hMenu = PInvoke.CreatePopupMenu();
        if (hMenu.IsNull)
            return;

        try
        {
            var items = TrayMenuBuilder.Build();
            var deviceMap = BuildDeviceMap();

            foreach (var item in items)
            {
                if (item.Separator)
                {
                    _ = PInvoke.AppendMenu(hMenu, MENU_ITEM_FLAGS.MF_SEPARATOR, 0, null);
                }
                else
                {
                    var flags = MENU_ITEM_FLAGS.MF_STRING;
                    if (!item.Enabled)
                        flags |= MENU_ITEM_FLAGS.MF_GRAYED;
                    if (item.Checked)
                        flags |= MENU_ITEM_FLAGS.MF_CHECKED;

                    unsafe
                    {
                        fixed (char* pText = item.Text)
                        {
                            _ = PInvoke.AppendMenu(hMenu, flags, (nuint)(uint)item.Id, pText);
                        }
                    }
                }
            }

            var pt = default(Point);
            _ = PInvoke.GetCursorPos(out pt);

            _ = PInvoke.SetForegroundWindow(_hwnd);

            var cmd = PInvoke.TrackPopupMenu(
                hMenu,
                TRACK_POPUP_MENU_FLAGS.TPM_LEFTALIGN | TRACK_POPUP_MENU_FLAGS.TPM_RIGHTBUTTON | TRACK_POPUP_MENU_FLAGS.TPM_RETURNCMD | TRACK_POPUP_MENU_FLAGS.TPM_NONOTIFY,
                pt.X,
                pt.Y,
                0,
                _hwnd,
                (RECT*)null);

            if (cmd.Value != 0)
            {
                ExecuteMenuCommand(cmd.Value, deviceMap);
            }
        }
        finally
        {
            PInvoke.DestroyMenu(hMenu);
        }
    }

    private static Dictionary<int, string> BuildDeviceMap()
    {
        var map = new Dictionary<int, string>();
        var devices = AudioManager.EnumerateRenderEndpoints();
        for (var i = 0; i < devices.Count; i++)
            map[FirstDeviceId + i] = devices[i].Id;
        return map;
    }

    private void ExecuteMenuCommand(int cmd, Dictionary<int, string> deviceMap)
    {
        if (deviceMap.TryGetValue(cmd, out var deviceId))
        {
            try { AudioManager.SetDefault(deviceId); }
            catch { /* transient COM hiccup */ }
            return;
        }

        switch ((TrayMenuCommand)cmd)
        {
            case TrayMenuCommand.Refresh:
                break;
            case TrayMenuCommand.ToggleAutostart:
                StartupManager.SetEnabled(!StartupManager.IsEnabled());
                break;
            case TrayMenuCommand.About:
                OpenUrl("https://github.com/Wildfire2282/audio-switcher");
                break;
            case TrayMenuCommand.Exit:
                PInvoke.PostQuitMessage(0);
                break;
        }
    }

    private void OnTrayMessage(uint message)
    {
        switch (message)
        {
            case PInvoke.WM_RBUTTONUP:
            case PInvoke.WM_CONTEXTMENU:
                ShowContextMenu();
                break;
        }
    }


    private bool IsCursorOverIcon(Point cursorScreen)
    {
        var identifier = new NOTIFYICONIDENTIFIER
        {
            cbSize = (uint)Marshal.SizeOf<NOTIFYICONIDENTIFIER>(),
            hWnd = _hwnd,
            uID = IconId,
        };

        RECT rect;
        if (PInvoke.Shell_NotifyIconGetRect(in identifier, out rect).Value < 0)
            return false;

        return cursorScreen.X >= rect.left && cursorScreen.X <= rect.right
            && cursorScreen.Y >= rect.top && cursorScreen.Y <= rect.bottom;
    }
    private LRESULT WindowProc(HWND hWnd, uint msg, WPARAM wParam, LPARAM lParam)
    {
        switch (msg)
        {
            case TrayMessageId:
                OnTrayMessage((uint)lParam.Value);
                return (LRESULT)0;

            case PInvoke.WM_COMMAND:
                // Handled synchronously by TrackPopupMenu with TPM_RETURNCMD.
                return (LRESULT)0;

            case PInvoke.WM_DESTROY:
                PInvoke.PostQuitMessage(0);
                return (LRESULT)0;
        }

        return PInvoke.DefWindowProc(hWnd, msg, wParam, lParam);
    }

    private static LRESULT StaticWindowProc(HWND hWnd, uint msg, WPARAM wParam, LPARAM lParam)
    {
        if (_instance != null)
            return _instance.WindowProc(hWnd, msg, wParam, lParam);

        return PInvoke.DefWindowProc(hWnd, msg, wParam, lParam);
    }

    private static HICON LoadIcon(Theme theme)
    {
        var isDark = theme == Theme.Dark;
        var resourceName = isDark ? "AudioSwitcher.assets.icon_dark.png" : "AudioSwitcher.assets.icon_light.png";
        using var stream = typeof(Program).Assembly.GetManifestResourceStream(resourceName);
        if (stream == null)
            return CreateFallbackIcon(isDark);

        using var bitmap = new Bitmap(stream);
        return new HICON(bitmap.GetHicon());
    }

    private static HICON CreateFallbackIcon(bool isDark)
    {
        var color = isDark
            ? Color.FromArgb(0xE6, 0xE6, 0xE6)
            : Color.FromArgb(0x36, 0x36, 0x36);

        using var bitmap = new Bitmap(32, 32, PixelFormat.Format32bppArgb);
        using (var g = Graphics.FromImage(bitmap))
        {
            g.Clear(color);
        }

        return new HICON(bitmap.GetHicon());
    }

    private static void OpenUrl(string url)
    {
        try
        {
            System.Diagnostics.Process.Start(new System.Diagnostics.ProcessStartInfo(url) { UseShellExecute = true });
        }
        catch { /* ignore */ }
    }
}
