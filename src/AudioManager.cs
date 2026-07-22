namespace AudioSwitcher;

using System.Runtime.InteropServices;
using System.Runtime.InteropServices.Marshalling;
using AudioSwitcher.Interop;

 internal sealed record AudioDevice(string Id, string Name);
internal sealed record AudioSessionInfo(uint ProcessId, string DisplayName);

internal static class AudioManager
{
    private const float VolumeStep = 0.01f;

    public static List<AudioDevice> EnumerateRenderEndpoints()
    {
        var devices = new List<AudioDevice>();
        try
        {
            var enumerator = CreateEnumerator();
            try
            {
                var hr = enumerator.EnumAudioEndpoints(EDataFlow.eRender, DeviceState.Active, out var collPtr);
                if (!HResult.Succeeded(hr)) return devices;
                var collection = Cast<IMMDeviceCollection>(collPtr);

                try
                {
                    hr = collection.GetCount(out var count);
                    if (!HResult.Succeeded(hr)) return devices;

                    for (uint i = 0; i < count; i++)
                    {
                        try
                        {
                            hr = collection.Item(i, out var devPtr);
                            if (!HResult.Succeeded(hr)) continue;
                            var device = Cast<IMMDevice>(devPtr);

                            try
                            {
                                var name = GetDeviceFriendlyName(device);
                                var id = GetDeviceId(device);
                                if (!string.IsNullOrEmpty(name) && !string.IsNullOrEmpty(id))
                                    devices.Add(new AudioDevice(id, name));
                            }
                            finally { Release(device); }
                        }
                        catch { /* skip */ }
                    }
                }
                finally { Release(collection); }
            }
            finally { Release(enumerator); }
        }
        catch { /* COM hiccup */ }
        return devices;
    }

    public static string? GetCurrentDefaultId()
    {
        try
        {
            var enumerator = CreateEnumerator();
            try
            {
                var hr = enumerator.GetDefaultAudioEndpoint(EDataFlow.eRender, ERole.eConsole, out var devPtr);
                if (!HResult.Succeeded(hr)) return null;
                var device = Cast<IMMDevice>(devPtr);
                try { return GetDeviceId(device); }
                finally { Release(device); }
            }
            finally { Release(enumerator); }
        }
        catch { return null; }
    }

    public static void SetDefault(string deviceId)
    {
        var ptr = Marshal.StringToCoTaskMemUni(deviceId);
        try
        {
            var policy = CreatePolicyConfig();
            try
            {
                var hr = policy.SetDefaultEndpoint(ptr, ERole.eConsole); HResult.ThrowIfFailed(hr);
                hr = policy.SetDefaultEndpoint(ptr, ERole.eMultimedia); HResult.ThrowIfFailed(hr);
                hr = policy.SetDefaultEndpoint(ptr, ERole.eCommunications); HResult.ThrowIfFailed(hr);
            }
            finally { Release(policy); }
        }
        finally { Marshal.FreeCoTaskMem(ptr); }
    }

    public static void AdjustVolume(int notches)
    {
        if (notches == 0) return;
        var volume = GetEndpointVolume();
        if (volume == null) return;
        try
        {
            var hr = volume.GetMasterVolumeLevelScalar(out var current); HResult.ThrowIfFailed(hr);
            var newLevel = Math.Clamp(current + notches * VolumeStep, 0.0f, 1.0f);
            hr = volume.SetMasterVolumeLevelScalar(newLevel, IntPtr.Zero); HResult.ThrowIfFailed(hr);
        }
        finally { Release(volume); }
    }

    public static float? GetVolumeScalar()
    {
        var volume = GetEndpointVolume();
        if (volume == null) return null;
        try
        {
            var hr = volume.GetMasterVolumeLevelScalar(out var level);
            return HResult.Succeeded(hr) ? level : null;
        }
        finally { Release(volume); }
    }

    // ── per-app audio session methods ──────────────────────────────

    public static List<AudioSessionInfo> EnumerateSessions()
    {
        var sessions = new List<AudioSessionInfo>();
        try
        {
            var enumerator = CreateEnumerator();
            try
            {
                var hr = enumerator.GetDefaultAudioEndpoint(EDataFlow.eRender, ERole.eConsole, out var devPtr);
                if (!HResult.Succeeded(hr)) return sessions;
                var device = Cast<IMMDevice>(devPtr);

                try
                {
                    var iid = typeof(IAudioSessionManager2).GUID;
                    hr = device.Activate(ref iid, Clsctx.CLSCTX_ALL, IntPtr.Zero, out var mgrPtr);
                    if (!HResult.Succeeded(hr)) return sessions;
                    var mgr = Cast<IAudioSessionManager2>(mgrPtr);

                    try
                    {
                        hr = mgr.GetSessionEnumerator(out var enumPtr);
                        if (!HResult.Succeeded(hr)) return sessions;
                        var sessionEnum = Cast<IAudioSessionEnumerator>(enumPtr);

                        try
                        {
                            hr = sessionEnum.GetCount(out var count);
                            if (!HResult.Succeeded(hr)) return sessions;

                            for (int i = 0; i < count; i++)
                            {
                                hr = sessionEnum.GetSession(i, out var sessionPtr);
                                if (!HResult.Succeeded(hr)) continue;

                                try
                                {
                                    hr = Cast<IAudioSessionControl2>(sessionPtr).GetState(out var state);
                                    if (!HResult.Succeeded(hr) || state != 0) continue;

                                    hr = Cast<IAudioSessionControl2>(sessionPtr).GetProcessId(out var pid);
                                    if (!HResult.Succeeded(hr)) continue;

                                    hr = Cast<IAudioSessionControl2>(sessionPtr).GetDisplayName(out var namePtr);
                                    string name = string.Empty;
                                    if (HResult.Succeeded(hr) && namePtr != IntPtr.Zero)
                                    {
                                        try { name = Marshal.PtrToStringUni(namePtr) ?? string.Empty; }
                                        finally { NativeMethods.CoTaskMemFree(namePtr); }
                                    }

                                    if (string.IsNullOrWhiteSpace(name))
                                        name = GetProcessNameSafe(pid);

                                    sessions.Add(new AudioSessionInfo(pid, name));
                                }
                                catch { /* skip */ }
                            }
                        }
                        finally { Release(sessionEnum); }
                    }
                    finally { Release(mgr); }
                }
                finally { Release(device); }
            }
            finally { Release(enumerator); }
        }
        catch { /* COM hiccup */ }
        return sessions;
    }

    public static float? GetSessionVolume(uint processId)
    {
        return IterateSession(processId, (sessionPtr) =>
        {
            var volume = Cast<ISimpleAudioVolume>(sessionPtr);
            var hr = volume.GetMasterVolume(out var level);
            return HResult.Succeeded(hr) ? level : (float?)null;
        });
    }

    public static void SetSessionVolume(uint processId, float level)
    {
        IterateSession(processId, (sessionPtr) =>
        {
            var volume = Cast<ISimpleAudioVolume>(sessionPtr);
            var hr = volume.SetMasterVolume(Math.Clamp(level, 0f, 1f), IntPtr.Zero);
            HResult.ThrowIfFailed(hr);
            return true;
        });
    }

    private static TRes? IterateSession<TRes>(uint processId, Func<IntPtr, TRes?> action)
    {
        TRes? result = default;
        try
        {
            var enumerator = CreateEnumerator();
            try
            {
                var hr = enumerator.GetDefaultAudioEndpoint(EDataFlow.eRender, ERole.eConsole, out var devPtr);
                if (!HResult.Succeeded(hr)) return result;
                var device = Cast<IMMDevice>(devPtr);

                try
                {
                    var iid = typeof(IAudioSessionManager2).GUID;
                    hr = device.Activate(ref iid, Clsctx.CLSCTX_ALL, IntPtr.Zero, out var mgrPtr);
                    if (!HResult.Succeeded(hr)) return result;
                    var mgr = Cast<IAudioSessionManager2>(mgrPtr);

                    try
                    {
                        hr = mgr.GetSessionEnumerator(out var enumPtr);
                        if (!HResult.Succeeded(hr)) return result;
                        var sessionEnum = Cast<IAudioSessionEnumerator>(enumPtr);

                        try
                        {
                            hr = sessionEnum.GetCount(out var count);
                            if (!HResult.Succeeded(hr)) return result;

                            for (int i = 0; i < count; i++)
                            {
                                hr = sessionEnum.GetSession(i, out var sessionPtr);
                                if (!HResult.Succeeded(hr)) continue;

                                try
                                {
                                    var control = Cast<IAudioSessionControl2>(sessionPtr);
                                    try
                                    {
                                        hr = control.GetProcessId(out var pid);
                                        if (!HResult.Succeeded(hr) || pid != processId) continue;

                                        result = action(sessionPtr);
                                        return result;
                                    }
                                    finally { Release(control); }
                                }
                                catch { /* skip */ }
                            }
                        }
                        finally { Release(sessionEnum); }
                    }
                    finally { Release(mgr); }
                }
                finally { Release(device); }
            }
            finally { Release(enumerator); }
        }
        catch { return result; }
        return result;
    }

    private static string GetProcessNameSafe(uint processId)
    {
        try
        {
            using var process = System.Diagnostics.Process.GetProcessById((int)processId);
            return process.ProcessName;
        }
        catch { return $"PID {processId}"; }
    }

    // ── helpers ────────────────────────────────────────────────

    private static IAudioEndpointVolume? GetEndpointVolume()
    {
        try
        {
            var enumerator = CreateEnumerator();
            try
            {
                var hr = enumerator.GetDefaultAudioEndpoint(EDataFlow.eRender, ERole.eConsole, out var devPtr);
                if (!HResult.Succeeded(hr)) return null;
                var device = Cast<IMMDevice>(devPtr);
                try
                {
                    var iid = typeof(IAudioEndpointVolume).GUID;
                    hr = device.Activate(ref iid, Clsctx.CLSCTX_ALL, IntPtr.Zero, out var volPtr);
                    return HResult.Succeeded(hr) && volPtr != IntPtr.Zero
                        ? Cast<IAudioEndpointVolume>(volPtr)
                        : null;
                }
                finally { Release(device); }
            }
            finally { Release(enumerator); }
        }
        catch { return null; }
    }

    private static IMMDeviceEnumerator CreateEnumerator()
    {
        var hr = NativeMethods.CoCreateInstance(Clsids.MMDeviceEnumerator, IntPtr.Zero,
            Clsctx.CLSCTX_ALL, typeof(IMMDeviceEnumerator).GUID, out var ptr);
        HResult.ThrowIfFailed(hr);
        return Cast<IMMDeviceEnumerator>(ptr)!;
    }

    private static IPolicyConfig CreatePolicyConfig()
    {
        var hr = NativeMethods.CoCreateInstance(Clsids.PolicyConfigClient, IntPtr.Zero,
            Clsctx.CLSCTX_ALL, typeof(IPolicyConfig).GUID, out var ptr);
        HResult.ThrowIfFailed(hr);
        return Cast<IPolicyConfig>(ptr)!;
    }

    private static string GetDeviceFriendlyName(IMMDevice device)
    {
        var hr = device.OpenPropertyStore(Stgms.STGM_READ, out var storePtr);
        HResult.ThrowIfFailed(hr);
        var store = Cast<IPropertyStore>(storePtr);
        try
        {
            var key = PropertyKeys.PKEY_Device_FriendlyName;
            hr = store.GetValue(ref key, out var value);
            HResult.ThrowIfFailed(hr);
            try
            {
                return (value.vt == 31 && value.pointerValue != IntPtr.Zero)
                    ? Marshal.PtrToStringUni(value.pointerValue) ?? string.Empty
                    : string.Empty;
            }
            finally { value.Dispose(); }
        }
        finally { Release(store); }
    }

    private static string GetDeviceId(IMMDevice device)
    {
        var hr = device.GetId(out var ptr);
        if (!HResult.Succeeded(hr) || ptr == IntPtr.Zero) return string.Empty;
        var id = Marshal.PtrToStringUni(ptr) ?? string.Empty;
        NativeMethods.CoTaskMemFree(ptr);
        return id;
    }
    private static T Cast<T>(IntPtr ptr) where T : class
    {
        var obj = ComHelpers.Wrappers.GetOrCreateObjectForComInstance(ptr, CreateObjectFlags.None);
        return (T)obj;
    }

    private static void Release(object? comObject)
    {
        if (comObject is IDisposable d)
            d.Dispose();
    }
}
