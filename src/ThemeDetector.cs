namespace AudioSwitcher;

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
            // Ignore registry errors.
        }
        return Theme.Unknown;
    }
}
