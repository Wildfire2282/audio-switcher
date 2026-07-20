using System.Runtime.InteropServices;
using AudioSwitcher;
using Windows.Win32;
unsafe
{
    PInvoke.CoInitializeEx(null, Windows.Win32.System.Com.COINIT.COINIT_MULTITHREADED);
}

try
{
    var devices = AudioManager.EnumerateRenderEndpoints();
    Console.WriteLine($"[smoke] enumerated {devices.Count} render device(s):");
    foreach (var d in devices)
        Console.WriteLine($"  - {d.Name}  ({d.Id})");

    if (devices.Count == 0)
    {
        Console.Error.WriteLine("[smoke] FAIL: no render endpoints");
        Environment.Exit(1);
    }

    var originalDefault = AudioManager.GetCurrentDefaultId();
    if (originalDefault == null)
    {
        Console.Error.WriteLine("[smoke] FAIL: current_default_id returned null");
        Environment.Exit(1);
    }
    Console.WriteLine($"[smoke] current default id = {originalDefault}");

    if (!devices.Any(d => d.Id == originalDefault))
    {
        Console.Error.WriteLine("[smoke] FAIL: current default id is not in the enumerated set");
        Environment.Exit(1);
    }

    var switched = 0;
    foreach (var d in devices)
    {
        if (d.Id == originalDefault)
            continue;

        AudioManager.SetDefault(d.Id);
        var now = AudioManager.GetCurrentDefaultId();
        if (now != d.Id)
        {
            Console.Error.WriteLine($"[smoke] FAIL: switch to '{d.Id}' did not stick (got '{now}')");
            Environment.Exit(1);
        }
        switched++;
        Console.WriteLine($"[smoke] switch ok -> {d.Name}");
    }

    if (switched == 0)
        Console.WriteLine("[smoke] WARN: only one device, nothing to switch to");
    else
        Console.WriteLine($"[smoke] all {switched} non-default switches round-tripped");

    AudioManager.SetDefault(originalDefault);
    var restored = AudioManager.GetCurrentDefaultId();
    if (restored != originalDefault)
    {
        Console.Error.WriteLine("[smoke] FAIL: could not restore original default");
        Environment.Exit(1);
    }
    Console.WriteLine("[smoke] restored original default ok");

    Console.WriteLine("[smoke] PASS");
}
finally
{
    PInvoke.CoUninitialize();
}
