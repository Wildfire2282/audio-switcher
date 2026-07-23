using System.Runtime.InteropServices;
using AudioSwitcher;
using Windows.Win32;
unsafe
{
    PInvoke.CoInitializeEx(null, Windows.Win32.System.Com.COINIT.COINIT_MULTITHREADED);
}

var exitCode = 0;
string? originalDefault = null;

try
{
    var devices = AudioManager.EnumerateRenderEndpoints();
    Console.WriteLine($"[smoke] enumerated {devices.Count} render device(s):");
    foreach (var d in devices)
        Console.WriteLine($"  - {d.Name}  ({d.Id})");

    if (devices.Count == 0)
        throw new InvalidOperationException("no render endpoints");

    originalDefault = AudioManager.GetCurrentDefaultId();
    if (originalDefault == null)
        throw new InvalidOperationException("current_default_id returned null");
    Console.WriteLine($"[smoke] current default id = {originalDefault}");

    if (!devices.Any(d => d.Id == originalDefault))
        throw new InvalidOperationException("current default id is not in the enumerated set");

    var switched = 0;
    foreach (var d in devices)
    {
        if (d.Id == originalDefault)
            continue;

        AudioManager.SetDefault(d.Id);
        var now = AudioManager.GetCurrentDefaultId();
        if (now != d.Id)
            throw new InvalidOperationException($"switch to '{d.Id}' did not stick (got '{now}')");
        switched++;
        Console.WriteLine($"[smoke] switch ok -> {d.Name}");
    }

    if (switched == 0)
        Console.WriteLine("[smoke] WARN: only one device, nothing to switch to");
    else
        Console.WriteLine($"[smoke] all {switched} non-default switches round-tripped");

    Console.WriteLine("[smoke] PASS");
}
catch (Exception ex)
{
    Console.Error.WriteLine($"[smoke] FAIL: {ex.Message}");
    exitCode = 1;
}
finally
{
    if (originalDefault != null)
    {
        try
        {
            AudioManager.SetDefault(originalDefault);
            var restored = AudioManager.GetCurrentDefaultId();
            if (restored != originalDefault)
            {
                Console.Error.WriteLine("[smoke] FAIL: could not restore original default");
                exitCode = 1;
            }
            else
            {
                Console.WriteLine("[smoke] restored original default ok");
            }
        }
        catch (Exception ex)
        {
            Console.Error.WriteLine($"[smoke] FAIL: restore failed: {ex.Message}");
            exitCode = 1;
        }
    }

    PInvoke.CoUninitialize();
}

return exitCode;
