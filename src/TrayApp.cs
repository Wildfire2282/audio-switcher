namespace AudioSwitcher;

using System.Diagnostics;
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

    private static TrayApp? _instance;
    private static HOOKPROC? _mouseHookProc;

    private readonly string _className;
    private HWND _hwnd;
    private HHOOK _hookHandle;
    private HICON _icon;
    private bool _trayIconAdded;
    private bool _initFailed;
    private bool _sessionNotificationsRegistered;

    public TrayApp()
    {
        _instance = this;
        _className = $"AudioSwitcherTray_{Guid.NewGuid():N}";

        try { RegisterWindowClass(); }
        catch (Exception ex) { Debug.WriteLine($"[AudioSwitcher] RegisterWindowClass failed: {ex.Message}"); _initFailed = true; return; }

        try { CreateMessageWindow(); }
        catch (Exception ex) { Debug.WriteLine($"[AudioSwitcher] CreateMessageWindow failed: {ex.Message}"); _initFailed = true; return; }

        try { _icon = LoadIcon(); }
        catch (Exception ex) { Debug.WriteLine($"[AudioSwitcher] LoadIcon failed: {ex.Message}"); }

        try { AddTrayIcon(); _trayIconAdded = true; }
        catch (Exception ex) { Debug.WriteLine($"[AudioSwitcher] AddTrayIcon failed: {ex.Message}"); }

        try { InstallWheelHook(); }
        catch (Exception ex) { Debug.WriteLine($"[AudioSwitcher] InstallWheelHook failed: {ex.Message}"); }

        try { RegisterDeviceHotplug(); }
        catch (Exception ex) { Debug.WriteLine($"[AudioSwitcher] RegisterDeviceHotplug failed: {ex.Message}"); }
    }

    public bool IsInitialized() => !_initFailed && _trayIconAdded && !_hwnd.IsNull;

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
        if (_initFailed)
        {
            _instance = null;
            return;
        }

        if (_trayIconAdded)
            RemoveTrayIcon();
        if (!_hookHandle.IsNull)
        {
            PInvoke.UnhookWindowsHookEx(_hookHandle);
            _hookHandle = default;
        }
        _mouseHookProc = null;
        AudioManager.UnregisterDeviceNotifications();
        AudioSessionManager.UnregisterSessionNotifications();

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
                HWND.HWND_MESSAGE,
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

        // NOTIFYICON_VERSION_4 is required for reliable middle/right mouse
        // message delivery on modern Windows.
        data.Anonymous.uVersion = 4;
        _ = PInvoke.Shell_NotifyIcon(NOTIFY_ICON_MESSAGE.NIM_SETVERSION, in data);
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

    private void RegisterDeviceHotplug()
    {
        AudioManager.RegisterDeviceNotifications(defaultId =>
        {
            AudioManager.InvalidateDeviceCache();
            // Refresh the tooltip to show current device/volume.
            if (AudioManager.GetVolumeScalar() is float volume)
            {
                var pct = (int)Math.Round(volume * 100);
                UpdateTooltip(BuildVolumeTooltip(pct));
            }
            else
            {
                UpdateTooltip(BuildVolumeTooltip(0));
            }
        });
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
                            catch (Exception ex) { Debug.WriteLine($"[AudioSwitcher] MouseHookProc: {ex.Message}"); }
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
            var device = AudioManager.EnumerateRenderEndpointsCached()
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
        // The struct is always freshly created (zero-initialized) before
        // this method is called, so we only need to write the text and
        // ensure it is properly null-terminated.
        var len = Math.Min(text.Length, 127);
        for (var i = 0; i < len; i++)
            data.szTip[i] = text[i];
        data.szTip[len] = '\0';
    }

    private static void ShowBalloonTip(string text)
    {
        var instance = _instance;
        if (instance == null || instance._hwnd.IsNull)
            return;

        var data = new NOTIFYICONDATAW
        {
            cbSize = (uint)Marshal.SizeOf<NOTIFYICONDATAW>(),
            hWnd = instance._hwnd,
            uID = IconId,
            uFlags = NOTIFY_ICON_DATA_FLAGS.NIF_INFO | NOTIFY_ICON_DATA_FLAGS.NIF_SHOWTIP,
        };
        data.Anonymous.uTimeout = 2000;
        var len = Math.Min(text.Length, 255);
        for (var i = 0; i < len; i++)
            data.szInfo[i] = text[i];
        data.dwInfoFlags = NOTIFY_ICON_INFOTIP_FLAGS.NIIF_NONE;

        _ = PInvoke.Shell_NotifyIcon(NOTIFY_ICON_MESSAGE.NIM_MODIFY, in data);
    }

    private unsafe void ShowLeftMenu()
    {
        if (!_sessionNotificationsRegistered)
        {
            try
            {
                AudioSessionManager.RegisterSessionNotifications();
                _sessionNotificationsRegistered = true;
            }
            catch (Exception ex) { Debug.WriteLine($"[AudioSwitcher] RegisterSessionNotifications failed: {ex.Message}"); }
        }

        var devices = AudioManager.EnumerateRenderEndpoints();
        var currentId = AudioManager.GetCurrentDefaultId();
        var sessions = AudioSessionManager.EnumerateSessions();
        var items = TrayMenuBuilder.BuildLeftMenu(devices, currentId, sessions);
        ShowContextMenu(items, devices, sessions);
    }

    private unsafe void ShowRightMenu()
    {
        var items = TrayMenuBuilder.BuildRightMenu();
        ShowContextMenu(items, [], []);
    }

    private unsafe void ShowContextMenu(List<TrayMenuItem> items, IReadOnlyList<AudioDevice> devices, IReadOnlyList<AudioSessionInfo> sessions)
    {
        var hMenu = PInvoke.CreatePopupMenu();
        if (hMenu.IsNull)
            return;

        var deviceMap = new Dictionary<int, string>();
        var sessionMap = new Dictionary<int, AudioSessionInfo>();

        try
        {
            AppendMenuItems(hMenu, items, devices, sessions, deviceMap, sessionMap);

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
                ExecuteMenuCommand(cmd.Value, deviceMap, sessionMap);
        }
        finally
        {
            PInvoke.DestroyMenu(hMenu);
        }
    }

    private static unsafe void AppendMenuItems(HMENU hMenu, List<TrayMenuItem> items, IReadOnlyList<AudioDevice> devices, IReadOnlyList<AudioSessionInfo> sessions, Dictionary<int, string> deviceMap, Dictionary<int, AudioSessionInfo> sessionMap)
    {
        foreach (var item in items)
        {
            if (item.Separator)
            {
                _ = PInvoke.AppendMenu(hMenu, MENU_ITEM_FLAGS.MF_SEPARATOR, 0, null);
                continue;
            }

            if (item.Children is { Count: > 0 })
            {
                var hSubMenu = PInvoke.CreatePopupMenu();
                if (!hSubMenu.IsNull)
                {
                    AppendMenuItems(hSubMenu, item.Children, devices, sessions, deviceMap, sessionMap);
                    fixed (char* pText = item.Text)
                    {
                        _ = PInvoke.AppendMenu(hMenu, MENU_ITEM_FLAGS.MF_STRING | MENU_ITEM_FLAGS.MF_POPUP, (nuint)hSubMenu.Value, pText);
                    }
                }
                continue;
            }

            var flags = MENU_ITEM_FLAGS.MF_STRING;
            if (!item.Enabled)
                flags |= MENU_ITEM_FLAGS.MF_GRAYED;
            if (item.Checked)
                flags |= MENU_ITEM_FLAGS.MF_CHECKED;

            // Build lookup maps for commands that need runtime data.
            if (item.Id >= TrayMenuBuilder.FirstDeviceId && item.Id < TrayMenuBuilder.FirstSessionMuteId)
            {
                var idx = item.Id - TrayMenuBuilder.FirstDeviceId;
                if (idx >= 0 && idx < devices.Count)
                    deviceMap[item.Id] = devices[idx].Id;
            }
            else if (item.Id >= TrayMenuBuilder.FirstSessionMuteId)
            {
                var idx = MapSessionIndex(item.Id);
                if (idx >= 0 && idx < sessions.Count)
                    sessionMap[item.Id] = sessions[idx];
            }

            fixed (char* pText = item.Text)
            {
                _ = PInvoke.AppendMenu(hMenu, flags, (nuint)(uint)item.Id, pText);
            }
        }
    }

    private static int MapSessionIndex(int id)
    {
        return id - TrayMenuBuilder.FirstSessionMuteId;
    }

    private void ExecuteMenuCommand(int cmd, Dictionary<int, string> deviceMap, Dictionary<int, AudioSessionInfo> sessionMap)
    {
        if (deviceMap.TryGetValue(cmd, out var deviceId))
        {
            try
            {
                AudioManager.SetDefault(deviceId);
                var deviceName = AudioManager.EnumerateRenderEndpointsCached()
                    .FirstOrDefault(d => d.Id == deviceId)?.Name ?? deviceId;
                ShowBalloonTip(string.Format(Locale.IsZh
                    ? "已切换到: {0}"
                    : "Switched to: {0}", deviceName));
            }
            catch (Exception ex) { Debug.WriteLine($"[AudioSwitcher] SetDefault: {ex.Message}"); }
            return;
        }

        if (sessionMap.TryGetValue(cmd, out var session))
        {
            try
            {
                AudioSessionManager.SetSessionMute(session.SessionId, !session.IsMuted);
            }
            catch (Exception ex) { Debug.WriteLine($"[AudioSwitcher] Session command: {ex.Message}"); }
            return;
        }

        switch ((TrayMenuCommand)cmd)
        {
            case TrayMenuCommand.ToggleAutostart:
            {
                var target = !StartupManager.IsEnabled();
                var success = StartupManager.SetEnabled(target);
                ShowBalloonTip(success
                    ? (target
                        ? Locale.T("开机自动启动已开启", "Start with Windows: ON")
                        : Locale.T("开机自动启动已关闭", "Start with Windows: OFF"))
                    : Locale.T("设置开机自启失败", "Failed to set autostart"));
                break;
            }
            case TrayMenuCommand.ToggleGlobalMute:
            {
                try
                {
                    AudioManager.ToggleMute();
                    var isMuted = AudioManager.GetMute();
                    var volume = AudioManager.GetVolumeScalar();
                    var pct = (int)Math.Round((volume ?? 0) * 100);
                    UpdateTooltip(BuildVolumeTooltip(pct));
                    ShowBalloonTip(isMuted == true
                        ? Locale.T("已全局静音", "Global mute: ON")
                        : Locale.T("已取消全局静音", "Global mute: OFF"));
                }
                catch (Exception ex)
                {
                    Debug.WriteLine($"[AudioSwitcher] ToggleGlobalMute: {ex.Message}");
                    ShowBalloonTip(Locale.T("全局静音切换失败", "Failed to toggle global mute"));
                }
                break;
            }
            case TrayMenuCommand.OpenVolumeMixer:
                OpenVolumeMixer();
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
        try
        {
            LogTrayMessage(message);
            switch (message)
            {
                case PInvoke.WM_LBUTTONUP:
                    ShowLeftMenu();
                    break;
                case PInvoke.WM_RBUTTONUP:
                case PInvoke.WM_CONTEXTMENU:
                    ShowRightMenu();
                    break;
            }
        }
        catch (Exception ex)
        {
            Debug.WriteLine($"[AudioSwitcher] Tray message 0x{message:X8} failed: {ex.Message}");
            LogTrayMessage(message, ex.Message);
        }
    }

    private static void LogTrayMessage(uint message, string? note = null)
    {
        try
        {
            var path = Path.Combine(Path.GetTempPath(), "audio-switcher-tray.log");
            var line = $"{DateTime.Now:HH:mm:ss.fff} msg=0x{message:X4} ({message}) {note}";
            File.AppendAllText(path, line + Environment.NewLine);
        }
        catch { }
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
                // NOTIFYICON_VERSION_4 packs the icon ID in HIWORD and the
                // actual mouse message in LOWORD of lParam.
                OnTrayMessage((uint)(lParam.Value & 0xFFFF));
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

    private static HICON LoadIcon()
    {
        const string resourceName = "AudioSwitcher.assets.icon.ico";
        using var stream = typeof(Program).Assembly.GetManifestResourceStream(resourceName);
        if (stream == null)
            return CreateFallbackIcon();

        // Load the 16x16 entry directly — avoids Bitmap.GetHicon() alpha
        // issues and the MemoryStream round-trip used for the PNG resource.
        // Do NOT dispose the Icon wrapper: on .NET the wrapper owns the HICON
        // and disposing it destroys the handle. TrayApp.Dispose calls
        // DestroyIcon to release it.
        var icon = new System.Drawing.Icon(stream, 16, 16);
        var handle = icon.Handle;
        GC.SuppressFinalize(icon);

        return new HICON(handle);
    }

    private static HICON CreateFallbackIcon()
    {
        var foreColor = Color.FromArgb(0x36, 0x36, 0x36);

        using var bitmap = new Bitmap(32, 32, PixelFormat.Format32bppArgb);
        using (var g = Graphics.FromImage(bitmap))
        {
            g.Clear(Color.Transparent);
            using var brush = new SolidBrush(foreColor);
            using var pen = new Pen(foreColor, 2);
            // Draw a simple speaker icon
            g.FillRectangle(brush, 6, 12, 8, 8);
            // Speaker cone (triangle pointing right)
            var cone = new[] { new Point(14, 10), new Point(22, 6), new Point(22, 26) };
            g.FillPolygon(brush, cone);
            // Sound waves
            g.DrawArc(pen, 20, 6, 8, 20, -60, 120);
            g.DrawArc(pen, 24, 2, 10, 28, -60, 120);
        }

        return new HICON(bitmap.GetHicon());
    }

    private static void OpenUrl(string url)
    {
        try
        {
            System.Diagnostics.Process.Start(new System.Diagnostics.ProcessStartInfo(url) { UseShellExecute = true });
        }
        catch (Exception ex) { Debug.WriteLine($"[AudioSwitcher] OpenUrl: {ex.Message}"); }
    }

    private static void OpenVolumeMixer()
    {
        try
        {
            var path = System.IO.Path.Combine(Environment.SystemDirectory, "sndvol.exe");
            System.Diagnostics.Process.Start(new System.Diagnostics.ProcessStartInfo(path) { UseShellExecute = true });
        }
        catch (Exception ex) { Debug.WriteLine($"[AudioSwitcher] OpenVolumeMixer: {ex.Message}"); }
    }
}
