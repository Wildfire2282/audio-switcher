namespace AudioSwitcher.Interop;

using System.Runtime.InteropServices;
using System.Runtime.InteropServices.Marshalling;
internal enum EDataFlow : uint
{
    eRender = 0,
    eCapture = 1,
    eAll = 2,
}

internal enum ERole : uint
{
    eConsole = 0,
    eMultimedia = 1,
    eCommunications = 2,
 }

internal enum AudioSessionState : int
{
    Active = 0,
    Inactive = 1,
    Expired = 2,
}

 [Flags]
internal enum DeviceState : uint
{
    Active = 0x00000001,
    Disabled = 0x00000002,
    NotPresent = 0x00000004,
    Unplugged = 0x00000008,
    All = 0x0000000F,
}

internal static class Clsids
{
    public static readonly Guid MMDeviceEnumerator = new("BCDE0395-E52F-467C-8E3D-C4579291692E");
    public static readonly Guid PolicyConfigClient = new("870af99c-171d-4f9e-af0d-e63df40c2bc9");
}

internal static class Iids
{
    public static readonly Guid IMMDeviceEnumerator = new("A95664D2-9614-4F35-A746-DE8DB63617E6");
}

internal static class Stgms
{
    public const uint STGM_READ = 0x00000000;
}

internal static class Clsctx
{
    public const uint CLSCTX_ALL = 0x00000001 | 0x00000002 | 0x00000004 | 0x00000010;
}

[StructLayout(LayoutKind.Sequential, Pack = 4)]
internal struct PropertyKey
{
    public Guid fmtid;
    public uint pid;
}

[StructLayout(LayoutKind.Explicit)]
internal struct PropVariant : IDisposable
{
    [FieldOffset(0)] public ushort vt;
    [FieldOffset(2)] public ushort wReserved1;
    [FieldOffset(4)] public ushort wReserved2;
    [FieldOffset(6)] public ushort wReserved3;
    [FieldOffset(8)] public IntPtr pointerValue;

    public void Dispose()
    {
        if (pointerValue != IntPtr.Zero)
        {
            // VT_LPWSTR -> free the underlying string and clear the variant.
            if (vt == 31)
            {
                Marshal.FreeCoTaskMem(pointerValue);
            }
            pointerValue = IntPtr.Zero;
            vt = 0;
        }
    }
}

internal static class PropertyKeys
{
    public static readonly PropertyKey PKEY_Device_FriendlyName = new()
    {
        fmtid = new Guid("A45C254E-DF1C-4EFD-8020-67D146A850E0"),
        pid = 14,
    };
}

internal static class HResult
{
    public const int S_OK = 0;
    public const int ERROR_NOT_FOUND = unchecked((int)0x80070490);

    public static bool Succeeded(int hr) => hr >= 0;

    public static void ThrowIfFailed(int hr)
    {
        if (hr < 0)
            Marshal.ThrowExceptionForHR(hr);
    }
}

internal static class ComHelpers
{
    public static readonly StrategyBasedComWrappers Wrappers = new();
}
