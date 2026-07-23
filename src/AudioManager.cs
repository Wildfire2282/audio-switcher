namespace AudioSwitcher;

using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Runtime.InteropServices.Marshalling;
using AudioSwitcher.Interop;

 internal sealed record AudioDevice(string Id, string Name);

internal static class AudioManager
{
    private const float VolumeStep = 0.01f;

    // ── Device enumeration cache ────────────────────────────────
    private static List<AudioDevice>? _cachedDevices;
    private static DateTime _devicesCacheTime;
    private static string? _cachedDefaultId;
    private static DateTime _defaultIdCacheTime;
    private static readonly TimeSpan CacheTtl = TimeSpan.FromMilliseconds(800);

    public static void InvalidateDeviceCache()
    {
        _cachedDevices = null;
        _cachedDefaultId = null;
    }

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
                        catch { Debug.WriteLine($"[AudioSwitcher] EnumerateRenderEndpoints: skip device"); }
                    }
                }
                finally { Release(collection); }
            }
            finally { Release(enumerator); }
        }
        catch (Exception ex) { Debug.WriteLine($"[AudioSwitcher] EnumerateRenderEndpoints: {ex.Message}"); }

        if (devices.Count > 0)
        {
            _cachedDevices = devices;
            _devicesCacheTime = DateTime.UtcNow;
        }
        return devices;
    }

    public static List<AudioDevice> EnumerateRenderEndpointsCached()
    {
        if (_cachedDevices != null && DateTime.UtcNow - _devicesCacheTime < CacheTtl)
            return _cachedDevices;
        return EnumerateRenderEndpoints();
    }

    public static string? GetCurrentDefaultId()
    {
        if (_cachedDefaultId != null && DateTime.UtcNow - _defaultIdCacheTime < CacheTtl)
            return _cachedDefaultId;
        return RefreshDefaultId();
    }

    private static string? RefreshDefaultId()
    {
        return GetDefaultId(ERole.eConsole);
    }

    private static string? GetDefaultId(ERole role)
    {
        try
        {
            var enumerator = CreateEnumerator();
            try
            {
                var hr = enumerator.GetDefaultAudioEndpoint(EDataFlow.eRender, role, out var devPtr);
                if (!HResult.Succeeded(hr)) return null;
                var device = Cast<IMMDevice>(devPtr);
                try
                {
                    var id = GetDeviceId(device);
                    if (role == ERole.eConsole && !string.IsNullOrEmpty(id))
                    {
                        _cachedDefaultId = id;
                        _defaultIdCacheTime = DateTime.UtcNow;
                    }
                    return id;
                }
                finally { Release(device); }
            }
            finally { Release(enumerator); }
        }
        catch (Exception ex) { Debug.WriteLine($"[AudioSwitcher] GetDefaultId: {ex.Message}"); return null; }
    }

    public static void SetDefault(string deviceId)
    {
        var oldConsole = GetDefaultId(ERole.eConsole);
        var oldMultimedia = GetDefaultId(ERole.eMultimedia);
        var oldCommunications = GetDefaultId(ERole.eCommunications);
        if (oldConsole == null || oldMultimedia == null || oldCommunications == null)
            throw new InvalidOperationException("Unable to capture current default audio endpoints");

        var ptr = Marshal.StringToCoTaskMemUni(deviceId);
        try
        {
            var policy = CreatePolicyConfig();
            try
            {
                try
                {
                    var hr = policy.SetDefaultEndpoint(ptr, ERole.eConsole); HResult.ThrowIfFailed(hr);
                    hr = policy.SetDefaultEndpoint(ptr, ERole.eMultimedia); HResult.ThrowIfFailed(hr);
                    hr = policy.SetDefaultEndpoint(ptr, ERole.eCommunications); HResult.ThrowIfFailed(hr);
                }
                catch
                {
                    RestoreDefault(policy, oldConsole, ERole.eConsole);
                    RestoreDefault(policy, oldMultimedia, ERole.eMultimedia);
                    RestoreDefault(policy, oldCommunications, ERole.eCommunications);
                    throw;
                }
            }
            finally
            {
                Release(policy);
                InvalidateDeviceCache();
            }
        }
        finally { Marshal.FreeCoTaskMem(ptr); }
    }

    private static void RestoreDefault(IPolicyConfig policy, string deviceId, ERole role)
    {
        var restorePtr = Marshal.StringToCoTaskMemUni(deviceId);
        try
        {
            var hr = policy.SetDefaultEndpoint(restorePtr, role);
            if (!HResult.Succeeded(hr))
                Debug.WriteLine($"[AudioSwitcher] RestoreDefault failed for {role}: 0x{hr:X8}");
        }
        catch (Exception ex) { Debug.WriteLine($"[AudioSwitcher] RestoreDefault failed for {role}: {ex.Message}"); }
        finally { Marshal.FreeCoTaskMem(restorePtr); }
    }

    public static void RegisterDeviceNotifications(DeviceChangeCallback callback)
    {
        try
        {
            if (_notificationClient != null) return;

            var notificationClient = new DeviceNotificationClient(callback);
            var enumerator = CreateEnumerator();
            var comPtr = ComHelpers.Wrappers.GetOrCreateComInterfaceForObject(
                notificationClient, CreateComInterfaceFlags.None);
            try
            {
                var hr = enumerator.RegisterEndpointNotificationCallback(comPtr);
                HResult.ThrowIfFailed(hr);
                _notificationClient = notificationClient;
                _enumerator = enumerator;
            }
            finally
            {
                _ = Marshal.Release(comPtr);
                if (_enumerator == null)
                    Release(enumerator);
            }
        }
        catch (Exception ex) { Debug.WriteLine($"[AudioSwitcher] RegisterDeviceNotifications: {ex.Message}"); }
    }

    public static void UnregisterDeviceNotifications()
    {
        if (_enumerator == null || _notificationClient == null) return;
        try
        {
            var comPtr = ComHelpers.Wrappers.GetOrCreateComInterfaceForObject(
                _notificationClient, CreateComInterfaceFlags.None);
            try
            {
                _ = _enumerator.UnregisterEndpointNotificationCallback(comPtr);
            }
            finally
            {
                _ = Marshal.Release(comPtr);
            }
        }
        catch (Exception ex) { Debug.WriteLine($"[AudioSwitcher] UnregisterDeviceNotifications: {ex.Message}"); }
        finally
        {
            Release(_enumerator);
            _enumerator = null;
            _notificationClient = null;
        }
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

    public static bool? GetMute()
    {
        var volume = GetEndpointVolume();
        if (volume == null) return null;
        try
        {
            var hr = volume.GetMute(out var mute);
            return HResult.Succeeded(hr) ? mute != 0 : null;
        }
        finally { Release(volume); }
    }

    public static void ToggleMute()
    {
        var volume = GetEndpointVolume();
        if (volume == null) return;
        try
        {
            var hr = volume.GetMute(out var mute); HResult.ThrowIfFailed(hr);
            hr = volume.SetMute(mute != 0 ? 0 : 1, IntPtr.Zero); HResult.ThrowIfFailed(hr);
        }
        finally { Release(volume); }
    }

    // ── helpers ────────────────────────────────────────────────

    private static IMMDeviceEnumerator? _enumerator;
    private static DeviceNotificationClient? _notificationClient;


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
        catch (Exception ex) { Debug.WriteLine($"[AudioSwitcher] GetEndpointVolume: {ex.Message}"); return null; }
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
