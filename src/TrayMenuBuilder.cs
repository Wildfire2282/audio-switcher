namespace AudioSwitcher;

internal enum TrayMenuCommand
{
    ToggleAutostart,
    ToggleGlobalMute,
    OpenVolumeMixer,
    About,
    Exit,
}

internal sealed record TrayMenuItem(string Text, int Id, bool Checked = false, bool Enabled = true, bool Separator = false, List<TrayMenuItem>? Children = null);

internal static class TrayMenuBuilder
{
    public const int FirstDeviceId = 1000;
    public const int FirstSessionMuteId = 2000;

    /// <summary>
    /// Left-click menu: switch default device + global mute + per-app mute.
    /// </summary>
    public static List<TrayMenuItem> BuildLeftMenu(IReadOnlyList<AudioDevice> devices, string? currentId, IReadOnlyList<AudioSessionInfo> sessions)
    {
        var items = new List<TrayMenuItem>();
        items.AddRange(BuildDeviceSection(devices, currentId));

        var globalMuted = AudioManager.GetMute() ?? false;
        items.Add(new TrayMenuItem(string.Empty, -1, Separator: true));
        items.Add(new TrayMenuItem(
            Locale.T("全局静音", "Global mute"),
            (int)TrayMenuCommand.ToggleGlobalMute,
            Checked: globalMuted));

        if (sessions.Count > 0)
        {
            items.Add(new TrayMenuItem(string.Empty, -1, Separator: true));
            items.Add(new TrayMenuItem(Locale.T("应用静音", "App mute"), -1, Enabled: false));

            for (var i = 0; i < sessions.Count; i++)
            {
                var session = sessions[i];
                items.Add(new TrayMenuItem(
                    session.DisplayName,
                    FirstSessionMuteId + i,
                    Checked: session.IsMuted));
            }
        }

        items.Add(new TrayMenuItem(string.Empty, -1, Separator: true));
        items.Add(new TrayMenuItem(Locale.T("打开音量合成器", "Open volume mixer"), (int)TrayMenuCommand.OpenVolumeMixer));

        return items;
    }

    /// <summary>
    /// Right-click menu: settings and exit.
    /// </summary>
    public static List<TrayMenuItem> BuildRightMenu()
    {
        var autostart = StartupManager.IsEnabled();
        var version = System.Reflection.Assembly.GetExecutingAssembly().GetName().Version;
        var versionStr = version != null ? $"v{version.Major}.{version.Minor}.{version.Build}" : "";

        return
        [
            new TrayMenuItem(Locale.T("开机自动启动", "Start with Windows"), (int)TrayMenuCommand.ToggleAutostart, Checked: autostart),
            new TrayMenuItem($"{Locale.T("关于", "About")} {versionStr}", (int)TrayMenuCommand.About),
            new TrayMenuItem(Locale.T("退出", "Exit"), (int)TrayMenuCommand.Exit),
        ];
    }

    private static List<TrayMenuItem> BuildDeviceSection(IReadOnlyList<AudioDevice> devices, string? currentId)
    {
        var items = new List<TrayMenuItem>();
        items.Add(new TrayMenuItem(Locale.T("输出设备", "Output device"), -1, Enabled: false));

        if (devices.Count == 0)
        {
            items.Add(new TrayMenuItem(Locale.T("(无可用音频设备)", "(No audio devices available)"), -1, Enabled: false));
        }
        else
        {
            for (var i = 0; i < devices.Count; i++)
            {
                var device = devices[i];
                var id = FirstDeviceId + i;
                items.Add(new TrayMenuItem(device.Name, id, Checked: device.Id == currentId));
            }
        }

        return items;
    }
}
