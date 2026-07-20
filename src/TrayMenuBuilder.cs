namespace AudioSwitcher;

internal enum TrayMenuCommand
{
    Refresh = 1,
    ToggleAutostart,
    About,
    Exit,
}

internal sealed record TrayMenuItem(string Text, int Id, bool Checked = false, bool Enabled = true, bool Separator = false);

internal static class TrayMenuBuilder
{
    private const int FirstDeviceId = 1000;

    public static List<TrayMenuItem> Build()
    {
        var items = new List<TrayMenuItem>();
        var devices = AudioManager.EnumerateRenderEndpoints();
        var currentId = AudioManager.GetCurrentDefaultId();
        var autostart = StartupManager.IsEnabled();

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

        items.Add(new TrayMenuItem(string.Empty, -1, Separator: true));
        items.Add(new TrayMenuItem(Locale.T("开机自动启动", "Start with Windows"), (int)TrayMenuCommand.ToggleAutostart, Checked: autostart));
        items.Add(new TrayMenuItem(Locale.T("刷新设备列表", "Refresh device list"), (int)TrayMenuCommand.Refresh));
        items.Add(new TrayMenuItem(Locale.T("关于", "About"), (int)TrayMenuCommand.About));
        items.Add(new TrayMenuItem(Locale.T("退出", "Exit"), (int)TrayMenuCommand.Exit));

        return items;
    }

    public static string? FindDeviceIdByMenuId(int menuId)
    {
        var index = menuId - FirstDeviceId;
        if (index < 0)
            return null;

        var devices = AudioManager.EnumerateRenderEndpoints();
        if (index >= devices.Count)
            return null;

        return devices[index].Id;
    }
}
