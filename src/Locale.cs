namespace AudioSwitcher;

using System.Globalization;

internal static class Locale
{
    public static bool IsZh { get; }

    static Locale()
    {
        var lang = CultureInfo.InstalledUICulture.TwoLetterISOLanguageName;
        IsZh = lang == "zh";
    }

    public static string T(string zh, string en) => IsZh ? zh : en;
}
