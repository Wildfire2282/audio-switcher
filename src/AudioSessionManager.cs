namespace AudioSwitcher;

using System.Diagnostics;
using System.Runtime.InteropServices;
using System.Runtime.InteropServices.Marshalling;
using AudioSwitcher.Interop;

internal sealed record AudioSessionInfo(
    string SessionId,
    string DisplayName,
    string ProcessName,
    uint ProcessId,
    float Volume,
    bool IsMuted,
    bool IsSystemSounds);

internal static class AudioSessionManager
{
    private static readonly object _cacheLock = new();
    private static List<AudioSessionInfo>? _cachedSessions;
    private static DateTime _sessionsCacheTime;
    private static readonly TimeSpan CacheTtl = TimeSpan.FromMilliseconds(500);

    private static readonly object _disposeLock = new();
    private static IMMDeviceEnumerator? _enumerator;
    private static IAudioSessionManager2? _sessionManager;
    private static SessionNotificationClient? _notificationClient;
    private static bool _disposed;
    private static bool _wantsSessionNotifications;

    public static void InvalidateCache()
    {
        lock (_cacheLock)
        {
            _cachedSessions = null;
        }
    }

    /// <summary>
    /// Called when the default audio render device changes so the cached
    /// IAudioSessionManager2 (bound to a specific IMMDevice) is released
    /// and will be re-created on next use. The session notification client
    /// is unregistered from the old manager and re-registered on the new
    /// one so that session notifications keep firing after a device switch.
    /// </summary>
    public static void OnDefaultDeviceChanged()
    {
        IAudioSessionManager2? oldMgr;
        SessionNotificationClient? oldClient;
        IMMDeviceEnumerator? oldEnum;
        bool wantsNotifications;

        lock (_disposeLock)
        {
            if (_disposed) return;
            oldMgr = _sessionManager;
            oldClient = _notificationClient;
            oldEnum = _enumerator;
            _sessionManager = null;
            _notificationClient = null;
            _enumerator = null;
            wantsNotifications = _wantsSessionNotifications;
        }

        // Unregister the old notification client from the old session
        // manager so the COM side stops holding a reference to it.
        if (oldMgr != null && oldClient != null)
        {
            try
            {
                var comPtr = ComHelpers.Wrappers.GetOrCreateComInterfaceForObject(
                    oldClient, CreateComInterfaceFlags.None);
                try { _ = oldMgr.UnregisterSessionNotification(comPtr); }
                finally { _ = Marshal.Release(comPtr); }
            }
            catch (Exception ex) { Debug.WriteLine($"[AudioSwitcher] OnDefaultDeviceChanged unregister: {ex.Message}"); }
        }

        if (oldMgr != null) { try { Release(oldMgr); } catch { } }
        if (oldEnum != null) { try { Release(oldEnum); } catch { } }

        // Re-create the session manager for the new device and re-register
        // notifications if they were previously requested.
        if (wantsNotifications)
            RegisterSessionNotifications();
    }

    public static List<AudioSessionInfo> EnumerateSessions()
    {
        lock (_cacheLock)
        {
            if (_cachedSessions != null && DateTime.UtcNow - _sessionsCacheTime < CacheTtl)
                return _cachedSessions;
        }

        var sessions = new List<AudioSessionInfo>();
        try
        {
            var mgr = GetSessionManager();
            if (mgr == null) return sessions;

            var hr = mgr.GetSessionEnumerator(out var enumPtr);
            if (!HResult.Succeeded(hr)) return sessions;

            try
            {
                var sessionEnum = Cast<IAudioSessionEnumerator>(enumPtr);
                try
                {
                    hr = sessionEnum.GetCount(out var count);
                    if (!HResult.Succeeded(hr)) return sessions;

                    for (var i = 0; i < count; i++)
                    {
                        hr = sessionEnum.GetSession(i, out var sessionPtr);
                        if (!HResult.Succeeded(hr)) continue;

                        try
                        {
                            var session = ProcessSession(sessionPtr);
                            if (session != null)
                                sessions.Add(session);
                        }
                        catch (Exception ex)
                        {
                            Debug.WriteLine($"[AudioSwitcher] EnumerateSessions: skip session {i}: {ex.Message}");
                        }
                        finally
                        {
                            Marshal.Release(sessionPtr);
                        }
                    }
                }
                finally
                {
                    Release(sessionEnum);
                }
            }
            finally
            {
                Marshal.Release(enumPtr);
            }
        }
        catch (Exception ex)
        {
            Debug.WriteLine($"[AudioSwitcher] EnumerateSessions: {ex.Message}");
        }

        sessions = sessions
            .OrderByDescending(s => s.IsSystemSounds)
            .ThenBy(s => s.DisplayName, StringComparer.OrdinalIgnoreCase)
            .ToList();

        lock (_cacheLock)
        {
            _cachedSessions = sessions;
            _sessionsCacheTime = DateTime.UtcNow;
        }

        return sessions;
    }

    private static AudioSessionInfo? ProcessSession(IntPtr sessionPtr)
    {
        var guid2 = typeof(IAudioSessionControl2).GUID;
        var hr = Marshal.QueryInterface(sessionPtr, in guid2, out var ctrl2Ptr);
        if (!HResult.Succeeded(hr)) return null;

        try
        {
            var ctrl2 = Cast<IAudioSessionControl2>(ctrl2Ptr);
            try
            {
                hr = ctrl2.GetProcessId(out var pid);
                if (!HResult.Succeeded(hr)) return null;

                string displayName;
                hr = ctrl2.GetDisplayName(out var namePtr);
                if (HResult.Succeeded(hr) && namePtr != IntPtr.Zero)
                {
                    displayName = Marshal.PtrToStringUni(namePtr) ?? string.Empty;
                    Marshal.FreeCoTaskMem(namePtr);
                }
                else
                {
                    displayName = string.Empty;
                }

                var isSystemSounds = ctrl2.IsSystemSoundsSession() == 0;

                hr = ctrl2.GetSessionIdentifier(out var idPtr);
                string sessionId;
                if (HResult.Succeeded(hr) && idPtr != IntPtr.Zero)
                {
                    sessionId = Marshal.PtrToStringUni(idPtr) ?? string.Empty;
                    Marshal.FreeCoTaskMem(idPtr);
                }
                else
                {
                    sessionId = string.Empty;
                }

                string processName = string.Empty;
                if (pid > 0)
                {
                    try
                    {
                        using var process = Process.GetProcessById((int)pid);
                        processName = process.ProcessName;
                    }
                    catch
                    {
                        processName = $"PID:{pid}";
                    }
                }

                if (isSystemSounds)
                {
                    displayName = Locale.T("系统声音", "System Sounds");
                }
                else if (string.IsNullOrWhiteSpace(displayName))
                {
                    if (!string.IsNullOrEmpty(processName))
                        displayName = processName;
                    else
                        displayName = Locale.T("未知应用", "Unknown");
                }

                var guidVol = typeof(ISimpleAudioVolume).GUID;
                hr = Marshal.QueryInterface(sessionPtr, in guidVol, out var volPtr);
                if (!HResult.Succeeded(hr)) return null;

                try
                {
                    var simpleVol = Cast<ISimpleAudioVolume>(volPtr);
                    try
                    {
                        hr = simpleVol.GetMasterVolume(out var volume);
                        if (!HResult.Succeeded(hr)) volume = 0;

                        hr = simpleVol.GetMute(out var muted);
                        var isMuted = HResult.Succeeded(hr) && muted != 0;

                        return new AudioSessionInfo(
                            SessionId: sessionId,
                            DisplayName: displayName,
                            ProcessName: processName,
                            ProcessId: pid,
                            Volume: volume,
                            IsMuted: isMuted,
                            IsSystemSounds: isSystemSounds);
                    }
                    finally
                    {
                        Release(simpleVol);
                    }
                }
                finally
                {
                    Marshal.Release(volPtr);
                }
            }
            finally
            {
                Release(ctrl2);
            }
        }
        finally
        {
            Marshal.Release(ctrl2Ptr);
        }
    }

    public static void SetSessionMute(string sessionId, bool mute)
    {
        var mgr = GetSessionManager();
        if (mgr == null) return;

        try
        {
            var hr = mgr.GetSessionEnumerator(out var enumPtr);
            if (!HResult.Succeeded(hr)) return;

            try
            {
                var sessionEnum = Cast<IAudioSessionEnumerator>(enumPtr);
                try
                {
                    hr = sessionEnum.GetCount(out var count);
                    if (!HResult.Succeeded(hr)) return;

                    for (var i = 0; i < count; i++)
                    {
                        hr = sessionEnum.GetSession(i, out var sessionPtr);
                        if (!HResult.Succeeded(hr)) continue;

                        try
                        {
                            var guidCtrl2 = typeof(IAudioSessionControl2).GUID;
                            var hr2 = Marshal.QueryInterface(sessionPtr, in guidCtrl2, out var ctrl2Ptr);
                            if (!HResult.Succeeded(hr2)) continue;

                            try
                            {
                                var ctrl2 = Cast<IAudioSessionControl2>(ctrl2Ptr);
                                try
                                {
                                    hr2 = ctrl2.GetSessionIdentifier(out var idPtr);
                                    if (!HResult.Succeeded(hr2)) continue;

                                    var id = Marshal.PtrToStringUni(idPtr) ?? string.Empty;
                                    Marshal.FreeCoTaskMem(idPtr);

                                    if (id == sessionId)
                                    {
                                        var guidVol2 = typeof(ISimpleAudioVolume).GUID;
                                        var hr3 = Marshal.QueryInterface(sessionPtr, in guidVol2, out var volPtr);
                                        if (!HResult.Succeeded(hr3)) continue;

                                        try
                                        {
                                            var simpleVol = Cast<ISimpleAudioVolume>(volPtr);
                                            try
                                            {
                                                unsafe { simpleVol.SetMute(mute ? 1 : 0, null); }
                                            }
                                            finally
                                            {
                                                Release(simpleVol);
                                            }
                                        }
                                        finally
                                        {
                                            Marshal.Release(volPtr);
                                        }

                                        InvalidateCache();
                                        return;
                                    }
                                }
                                finally
                                {
                                    Release(ctrl2);
                                }
                            }
                            finally
                            {
                                Marshal.Release(ctrl2Ptr);
                            }
                        }
                        finally
                        {
                            Marshal.Release(sessionPtr);
                        }
                    }
                }
                finally
                {
                    Release(sessionEnum);
                }
            }
            finally
            {
                Marshal.Release(enumPtr);
            }
        }
        catch (Exception ex)
        {
            Debug.WriteLine($"[AudioSwitcher] SetSessionMute: {ex.Message}");
        }
    }

    private static IAudioSessionManager2? GetSessionManager()
    {
        lock (_disposeLock)
        {
            if (_disposed) return null;
            if (_sessionManager != null) return _sessionManager;
        }

        try
        {
            var enumerator = CreateEnumerator();
            try
            {
                var hr = enumerator.GetDefaultAudioEndpoint(EDataFlow.eRender, ERole.eConsole, out var devPtr);
                if (!HResult.Succeeded(hr))
                {
                    Release(enumerator);
                    return null;
                }

                try
                {
                    var device = Cast<IMMDevice>(devPtr);
                    try
                    {
                        var guidMgr = typeof(IAudioSessionManager2).GUID;
                        hr = device.Activate(ref guidMgr, Clsctx.CLSCTX_ALL, IntPtr.Zero, out var mgrPtr);
                        if (!HResult.Succeeded(hr))
                        {
                            Release(enumerator);
                            return null;
                        }

                        var mgr = Cast<IAudioSessionManager2>(mgrPtr);

                        lock (_disposeLock)
                        {
                            if (_disposed)
                            {
                                Marshal.Release(mgrPtr);
                                Release(enumerator);
                                return null;
                            }
                            _sessionManager = mgr;
                            _enumerator = enumerator;
                        }

                        return mgr;
                    }
                    finally
                    {
                        Release(device);
                    }
                }
                finally
                {
                    Marshal.Release(devPtr);
                }
            }
            catch
            {
                Release(enumerator);
                return null;
            }
        }
        catch (Exception ex)
        {
            Debug.WriteLine($"[AudioSwitcher] GetSessionManager: {ex.Message}");
            return null;
        }
    }

    private static void Release(object? comObject)
    {
        if (comObject is IDisposable d)
            d.Dispose();
    }

    public static void RegisterSessionNotifications()
    {
        lock (_disposeLock)
        {
            if (_disposed) return;
            _wantsSessionNotifications = true;
            if (_notificationClient != null) return;

            var mgr = GetSessionManager();
            if (mgr == null) return;

            try
            {
                var notificationClient = new SessionNotificationClient(() =>
                {
                    InvalidateCache();
                });

                var comPtr = ComHelpers.Wrappers.GetOrCreateComInterfaceForObject(
                    notificationClient, CreateComInterfaceFlags.None);

                try
                {
                    var hr = mgr.RegisterSessionNotification(comPtr);
                    if (HResult.Succeeded(hr))
                    {
                        _notificationClient = notificationClient;
                    }
                }
                finally
                {
                    _ = Marshal.Release(comPtr);
                }
            }
            catch (Exception ex)
            {
                Debug.WriteLine($"[AudioSwitcher] RegisterSessionNotifications: {ex.Message}");
            }
        }
    }

    public static void UnregisterSessionNotifications()
    {
        IAudioSessionManager2? mgr;
        IMMDeviceEnumerator? enum_;
        SessionNotificationClient? client;
        IntPtr comPtr = IntPtr.Zero;

        lock (_disposeLock)
        {
            if (_disposed) return;
            if (_sessionManager == null || _notificationClient == null) return;

            _disposed = true;
            mgr = _sessionManager;
            enum_ = _enumerator;
            client = _notificationClient;
            _sessionManager = null;
            _enumerator = null;
            _notificationClient = null;
            _wantsSessionNotifications = false;

            comPtr = ComHelpers.Wrappers.GetOrCreateComInterfaceForObject(
                client, CreateComInterfaceFlags.None);
        }

        try
        {
            _ = mgr!.UnregisterSessionNotification(comPtr);
        }
        catch (Exception ex)
        {
            Debug.WriteLine($"[AudioSwitcher] UnregisterSessionNotifications: {ex.Message}");
        }
        finally
        {
            _ = Marshal.Release(comPtr);
            if (mgr != null) { try { Release(mgr); } catch { } }
            if (enum_ != null) { try { Release(enum_); } catch { } }
        }
    }

    private static IMMDeviceEnumerator CreateEnumerator()
    {
        var hr = NativeMethods.CoCreateInstance(Clsids.MMDeviceEnumerator, IntPtr.Zero,
            Clsctx.CLSCTX_ALL, typeof(IMMDeviceEnumerator).GUID, out var ptr);
        HResult.ThrowIfFailed(hr);
        return Cast<IMMDeviceEnumerator>(ptr)!;
    }

    private static T Cast<T>(IntPtr ptr) where T : class
    {
        var obj = ComHelpers.Wrappers.GetOrCreateObjectForComInstance(ptr, CreateObjectFlags.None);
        return (T)obj;
    }
}

[GeneratedComClass]
internal sealed partial class SessionNotificationClient : IAudioSessionNotification
{
    private readonly Action _callback;

    public SessionNotificationClient(Action callback)
    {
        _callback = callback;
    }

    public int OnSessionCreated(IntPtr newSession)
    {
        _callback();
        return 0;
    }
}
