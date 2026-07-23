namespace AudioSwitcher;

using System.Diagnostics;
using Microsoft.Win32;

internal static class StartupManager
{
    private const string SubKey = @"Software\Microsoft\Windows\CurrentVersion\Run";
    private const string ValueName = "AudioSwitcher";

    public static bool IsEnabled()
    {
        try
        {
            using var key = Registry.CurrentUser.OpenSubKey(SubKey, false);
            return key?.GetValue(ValueName) is string value && !string.IsNullOrEmpty(value);
        }
        catch
        {
            Debug.WriteLine("[AudioSwitcher] StartupManager.IsEnabled: registry error");
            return false;
        }
    }

    public static bool SetEnabled(bool enabled)
    {
        try
        {
            using var key = Registry.CurrentUser.OpenSubKey(SubKey, true)
                ?? Registry.CurrentUser.CreateSubKey(SubKey);

            if (enabled)
            {
                var exePath = Environment.ProcessPath;
                if (string.IsNullOrEmpty(exePath))
                    return false;
                key.SetValue(ValueName, $"\"{exePath}\"");
            }
            else
            {
                key.DeleteValue(ValueName, false);
            }

            return true;
        }
        catch
        {
            Debug.WriteLine("[AudioSwitcher] StartupManager.SetEnabled: registry error");
            return false;
        }
    }
}
