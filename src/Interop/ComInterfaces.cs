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
    int SetMasterVolumeLevel(float levelDB, IntPtr eventContext);

    [PreserveSig]
    int SetMasterVolumeLevelScalar(float level, IntPtr eventContext);

    [PreserveSig]
    int GetMasterVolumeLevel(out float levelDB);

    [PreserveSig]
    int GetMasterVolumeLevelScalar(out float level);

    [PreserveSig]
    int SetChannelVolumeLevel(uint channel, float levelDB, IntPtr eventContext);

    [PreserveSig]
    int GetChannelVolumeLevel(uint channel, out float levelDB);

    [PreserveSig]
    int SetChannelVolumeLevelScalar(uint channel, float level, IntPtr eventContext);

    [PreserveSig]
    int GetChannelVolumeLevelScalar(uint channel, out float level);


    [PreserveSig]
    int GetMute(out int mute);

    [PreserveSig]
    int GetVolumeStepInfo(out uint step, out uint stepCount);

    [PreserveSig]
    int SetMute(int mute, IntPtr eventContext);

    [PreserveSig]
    int VolumeStepUp(IntPtr eventContext);

    [PreserveSig]
    int VolumeStepDown(IntPtr eventContext);

    [PreserveSig]
    int QueryHardwareSupport(out uint hardwareSupportMask);

    [PreserveSig]
    int GetVolumeRange(out float volumeMinDB, out float volumeMaxDB, out float volumeStepDB);
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
