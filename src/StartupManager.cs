namespace AudioSwitcher;

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
            return false;
        }
    }

    public static void SetEnabled(bool enabled)
    {
        try
        {
            using var key = Registry.CurrentUser.OpenSubKey(SubKey, true)
                ?? Registry.CurrentUser.CreateSubKey(SubKey);

            if (enabled)
            {
                var exePath = Environment.ProcessPath;
                if (!string.IsNullOrEmpty(exePath))
                    key.SetValue(ValueName, $"\"{exePath}\"");
            }
            else
            {
                key.DeleteValue(ValueName, false);
            }
        }
        catch
        {
            // Ignore registry errors.
        }
    }
}
