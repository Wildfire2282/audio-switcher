namespace AudioSwitcher.Interop;

using System.Runtime.InteropServices;
using System.Runtime.InteropServices.Marshalling;

// AOT-safe COM interfaces using [GeneratedComInterface] (source-generated COM)
// instead of [ComImport] (runtime-based, fails under AOT).

[GeneratedComInterface]
[Guid("A95664D2-9614-4F35-A746-DE8DB63617E6")]
internal partial interface IMMDeviceEnumerator
{
    [PreserveSig]
    int EnumAudioEndpoints(EDataFlow dataFlow, DeviceState stateMask, out IntPtr devicesPtr);

    [PreserveSig]
    int GetDefaultAudioEndpoint(EDataFlow dataFlow, ERole role, out IntPtr devicePtr);

    [PreserveSig]
    int GetDevice(IntPtr idPtr, out IntPtr devicePtr);

    [PreserveSig]
    int RegisterEndpointNotificationCallback(IntPtr client);

    [PreserveSig]
    int UnregisterEndpointNotificationCallback(IntPtr client);
}

[GeneratedComInterface]
[Guid("0BD7A1BE-7A1A-44DB-8397-CC5392387B5E")]
internal partial interface IMMDeviceCollection
{
    [PreserveSig]
    int GetCount(out uint count);

    [PreserveSig]
    int Item(uint index, out IntPtr devicePtr);
}

[GeneratedComInterface]
[Guid("D666063F-1587-4E43-81F1-B948E807363F")]
internal partial interface IMMDevice
{
    [PreserveSig]
    int Activate(ref Guid iid, uint clsCtx, IntPtr activationParams, out IntPtr interfacePtr);

    [PreserveSig]
    int OpenPropertyStore(uint stgmAccess, out IntPtr propsPtr);

    [PreserveSig]
    int GetId(out IntPtr deviceIdPtr);

    [PreserveSig]
    int GetState(out DeviceState state);
}

[GeneratedComInterface]
[Guid("886D8EEB-8CF2-4446-8D02-CDBA1DBDCF99")]
internal partial interface IPropertyStore
{
    [PreserveSig]
    int GetCount(out uint count);

    [PreserveSig]
    int GetAt(uint index, out PropertyKey key);

    [PreserveSig]
    int GetValue(ref PropertyKey key, out PropVariant value);

    [PreserveSig]
    int SetValue(ref PropertyKey key, ref PropVariant value);

    [PreserveSig]
    int Commit();
}

[GeneratedComInterface]
[Guid("F8679F50-850A-41CF-9C72-430F290290C8")]
internal partial interface IPolicyConfig
{
    [PreserveSig]
    int GetMixFormat();

    [PreserveSig]
    int GetDeviceFormat();

    [PreserveSig]
    int ResetDeviceFormat();

    [PreserveSig]
    int SetDeviceFormat();

    [PreserveSig]
    int GetProcessingPeriod();

    [PreserveSig]
    int SetProcessingPeriod();

    [PreserveSig]
    int GetShareMode();

    [PreserveSig]
    int SetShareMode();

    [PreserveSig]
    int GetPropertyValue();

    [PreserveSig]
    int SetPropertyValue();

    [PreserveSig]
    int SetDefaultEndpoint(IntPtr deviceIdPtr, ERole role);

    [PreserveSig]
    int SetEndpointVisibility();
}

[GeneratedComInterface]
[Guid("5CDF2C82-841E-4546-9722-0CF74078229A")]
internal partial interface IAudioEndpointVolume
{
    [PreserveSig]
    int RegisterControlChangeNotify(IntPtr notify);

    [PreserveSig]
    int UnregisterControlChangeNotify(IntPtr notify);

    [PreserveSig]
    int GetChannelCount(out uint count);

    [PreserveSig]
    unsafe int SetMasterVolumeLevel(float levelDB, Guid* eventContext);

    [PreserveSig]
    unsafe int SetMasterVolumeLevelScalar(float level, Guid* eventContext);

    [PreserveSig]
    int GetMasterVolumeLevel(out float levelDB);

    [PreserveSig]
    int GetMasterVolumeLevelScalar(out float level);

    [PreserveSig]
    unsafe int SetChannelVolumeLevel(uint channel, float levelDB, Guid* eventContext);

    [PreserveSig]
    int GetChannelVolumeLevel(uint channel, out float levelDB);

    [PreserveSig]
    unsafe int SetChannelVolumeLevelScalar(uint channel, float level, Guid* eventContext);

    [PreserveSig]
    int GetChannelVolumeLevelScalar(uint channel, out float level);

    [PreserveSig]
    unsafe int SetMute(int mute, Guid* eventContext);

    [PreserveSig]
    int GetMute(out int mute);

    [PreserveSig]
    int GetVolumeStepInfo(out uint step, out uint stepCount);

    [PreserveSig]
    unsafe int VolumeStepUp(Guid* eventContext);

    [PreserveSig]
    unsafe int VolumeStepDown(Guid* eventContext);

    [PreserveSig]
    int QueryHardwareSupport(out uint hardwareSupportMask);

    [PreserveSig]
    int GetVolumeRange(out float volumeMinDB, out float volumeMaxDB, out float volumeStepDB);
}

// ── Per-session audio interfaces ───────────────────────────

[GeneratedComInterface]
[Guid("77AA99A0-1BD6-484F-8BC7-2C654C9A9B6F")]
internal partial interface IAudioSessionManager2
{
    [PreserveSig]
    int GetAudioSessionControl(IntPtr audioSessionGuid, uint streamFlags, out IntPtr sessionControl);

    [PreserveSig]
    int GetSimpleAudioVolume(IntPtr audioSessionGuid, uint streamFlags, out IntPtr audioVolume);

    [PreserveSig]
    int GetSessionEnumerator(out IntPtr sessionEnum);

    [PreserveSig]
    int RegisterSessionNotification(IntPtr sessionNotification);

    [PreserveSig]
    int UnregisterSessionNotification(IntPtr sessionNotification);
}

[GeneratedComInterface]
[Guid("E2F5BB11-0570-40CA-ACDD-3AA01277DEE8")]
internal partial interface IAudioSessionEnumerator
{
    [PreserveSig]
    int GetCount(out int sessionCount);

    [PreserveSig]
    int GetSession(int sessionCount, out IntPtr session);
}

[GeneratedComInterface]
[Guid("BFB7FF88-7239-4FC9-8FA2-07C950BE9C6D")]
internal partial interface IAudioSessionControl2
{
    [PreserveSig]
    int GetState(out int state);

    [PreserveSig]
    int GetDisplayName(out IntPtr displayName);

    [PreserveSig]
    int SetDisplayName(IntPtr displayName, IntPtr eventContext);

    [PreserveSig]
    int GetIconPath(out IntPtr iconPath);

    [PreserveSig]
    int SetIconPath(IntPtr iconPath, IntPtr eventContext);

    [PreserveSig]
    int GetGroupingParam(out Guid groupingParam);

    [PreserveSig]
    int SetGroupingParam(Guid groupingId, IntPtr eventContext);

    [PreserveSig]
    int AudioVolumeNotificationRegistered(IntPtr notificationObject);

    [PreserveSig]
    int UnregisterAudioVolumeNotification(IntPtr notificationObject);

    // IAudioSessionControl2 specific
    [PreserveSig]
    int GetSessionIdentifier(out IntPtr sessionIdentifier);

    [PreserveSig]
    int GetSessionInstanceIdentifier(out IntPtr sessionInstanceIdentifier);

    [PreserveSig]
    int GetProcessId(out uint processId);

    [PreserveSig]
    int IsSystemSoundsSession();

    [PreserveSig]
    int SetDuckingPreference([MarshalAs(UnmanagedType.Bool)] bool optOut);
}

[GeneratedComInterface]
[Guid("87CE5498-68D6-44E5-9215-6DA47EF883D8")]
internal partial interface ISimpleAudioVolume
{
    [PreserveSig]
    unsafe int SetMasterVolume(float level, Guid* eventContext);

    [PreserveSig]
    int GetMasterVolume(out float level);

    [PreserveSig]
    unsafe int SetMute(int mute, Guid* eventContext);

    [PreserveSig]
    int GetMute(out int mute);
}

[GeneratedComInterface]
[Guid("2BE0978D-8A09-4704-89A1-3D81BF418F0C")]
internal partial interface IAudioSessionNotification
{
    [PreserveSig]
    int OnSessionCreated(IntPtr newSession);
}

// ── Device change notification callback ────────────────────

internal delegate void DeviceChangeCallback(string? defaultDeviceId);

[GeneratedComInterface]
[Guid("7991EEC9-7E89-4D85-8390-6C703CEC60C0")]
internal partial interface IMMNotificationClient
{
    [PreserveSig]
    int OnDeviceStateChanged(IntPtr pwstrDeviceId, DeviceState dwNewState);

    [PreserveSig]
    int OnDeviceAdded(IntPtr pwstrDeviceId);

    [PreserveSig]
    int OnDeviceRemoved(IntPtr pwstrDeviceId);

    [PreserveSig]
    int OnDefaultDeviceChanged(EDataFlow flow, ERole role, IntPtr pwstrDefaultDeviceId);

    [PreserveSig]
    int OnPropertyValueChanged(IntPtr pwstrDeviceId, PropertyKey key);
}

[GeneratedComClass]
internal sealed partial class DeviceNotificationClient : IMMNotificationClient
{
    private readonly DeviceChangeCallback _callback;

    public DeviceNotificationClient(DeviceChangeCallback callback)
    {
        _callback = callback;
    }

    public int OnDeviceStateChanged(IntPtr pwstrDeviceId, DeviceState dwNewState)
    {
        _callback(null);
        return 0;
    }

    public int OnDeviceAdded(IntPtr pwstrDeviceId)
    {
        _callback(null);
        return 0;
    }

    public int OnDeviceRemoved(IntPtr pwstrDeviceId)
    {
        _callback(null);
        return 0;
    }

    public int OnDefaultDeviceChanged(EDataFlow flow, ERole role, IntPtr pwstrDefaultDeviceId)
    {
        if (flow == EDataFlow.eRender && role == ERole.eConsole)
        {
            var id = pwstrDefaultDeviceId != IntPtr.Zero
                ? Marshal.PtrToStringUni(pwstrDefaultDeviceId)
                : null;
            _callback(id);
        }
        return 0;
    }

    public int OnPropertyValueChanged(IntPtr pwstrDeviceId, PropertyKey key)
    {
        return 0;
    }
}
