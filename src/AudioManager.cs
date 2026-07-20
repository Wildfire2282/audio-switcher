namespace AudioSwitcher;

using System.Runtime.InteropServices;
using System.Runtime.InteropServices.Marshalling;
using AudioSwitcher.Interop;

internal sealed record AudioDevice(string Id, string Name);

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
        var devices = EnumerateRenderEndpoints();
        if (!devices.Any(d => d.Id == deviceId))
            Marshal.ThrowExceptionForHR(HResult.ERROR_NOT_FOUND);

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
