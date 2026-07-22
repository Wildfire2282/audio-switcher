namespace AudioSwitcher;

using System.Diagnostics;
using Microsoft.Win32;

internal enum Theme
{
    Light,
    Dark,
    Unknown,
}

internal static class ThemeDetector
{
    private const string SubKey = @"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";
    private const string ValueName = "AppsUseLightTheme";

    public static Theme Detect()
    {
        try
        {
            using var key = Registry.CurrentUser.OpenSubKey(SubKey, false);
            if (key?.GetValue(ValueName) is int value)
            {
                return value == 0 ? Theme.Dark : Theme.Light;
            }
        }
        catch
        {
            Debug.WriteLine("[AudioSwitcher] ThemeDetector.Detect: registry error");
        }
        return Theme.Unknown;
    }
}
