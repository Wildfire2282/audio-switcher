# Audio Switcher

Quick audio output device switcher for the Windows system tray.

Right-click the tray icon to switch the default playback device directly — no need to open Sound settings.

- **Device switching** — enumerates all active render endpoints, one click sets default
- **Three-role switch** — sets console / multimedia / communications ERoles simultaneously
- **Wheel volume** — scroll over the tray icon to adjust volume, instant tooltip shows device name + percentage
- **Auto-start** — toggle Windows startup from the menu
- **About** — menu link to the GitHub repo
- **Theme-aware** — auto-detects system theme (light/dark), picks matching tray icon
- **Single instance** — global mutex ensures only one instance runs
- **AOT single-file** — published as a single native executable, no runtime required

## Tech stack

- C# 13 / .NET 10
- Native Win32 P/Invoke (CsWin32 source generator)
- Core Audio API (`MMDeviceEnumerator`, `IPolicyConfig`, `IAudioEndpointVolume`)
- Native tray icon & context menu (`Shell_NotifyIcon` / `TrackPopupMenu`)
- AOT + single-file publish

> Note: Windows Forms is incompatible with .NET 10 AOT trimming, so the UI layer uses raw Win32.

## Usage

1. Download the latest `audio-switcher.exe` from [Releases](https://github.com/Wildfire2282/audio-switcher/releases)
2. Double-click to run (no admin required)

Right-click the tray icon to pick a default audio device.

## Build

```shell
dotnet build
```

## AOT single-file publish

```shell
dotnet publish -c Release -r win-x64 --self-contained true
```

Output: `bin/Release/net10.0-windows/win-x64/publish/audio-switcher.exe` (~2.7 MB).

### Prerequisites

- .NET 10 SDK
- Windows 10/11 (x64)

## Project structure

```
src/
├── Program.cs              # Entry: COM init, single-instance guard, message loop
├── TrayApp.cs              # Native tray icon, context menu, wheel hook
├── AudioManager.cs         # Device enumeration + default switching
├── TrayMenuBuilder.cs      # Context menu item builder
├── ThemeDetector.cs        # Windows theme detection (registry)
├── StartupManager.cs       # Auto-start manager (Run registry key)
└── Interop/
    ├── ComInterfaces.cs    # Core Audio COM interfaces (GeneratedComInterface)
    ├── Win32Defs.cs        # Win32 enums, structs, HRESULT helpers
    └── NativeMethods.cs    # AOT-safe CoCreateInstance + DPI awareness
tests/
└── Smoke/
    └── Program.cs          # End-to-end audio smoke test
```

## Internals

### Audio switching

Uses `MMDeviceEnumerator` to enumerate active render endpoints, switches the default via `IPolicyConfig` (`PolicyConfigClient` COM class). Based on [`com-policy-config`](https://github.com/sidit77/com-policy-config).

Switching covers all three ERoles:

- `eConsole` — console
- `eMultimedia` — multimedia
- `eCommunications` — communications

### Tray icon

Two PNGs (light/dark) are embedded as resources. The matching icon is selected at startup based on the registry value `AppsUseLightTheme`.

### Auto-start

Managed via `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, per-user only.

### Wheel volume

A `WH_MOUSE_LL` low-level mouse hook captures global scroll events. Hover detection uses `Shell_NotifyIconGetRect` to check whether the cursor is inside the tray icon's bounding rectangle at the time of the event — no timers, no lag. Volume is adjusted in 1% steps via `IAudioEndpointVolume`, with an instant tooltip showing device name + percentage.

### DPI awareness

`SetProcessDpiAwarenessContext` with `DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2` is called at startup, so the native context menu renders at native resolution on high-DPI (4K) displays.

### AOT COM interop

All Core Audio COM interfaces use `[GeneratedComInterface]` (source-generated COM) instead of `[ComImport]` (runtime-based, fails under AOT). RCW creation goes through `StrategyBasedComWrappers`, and all string/boolean marshalling is done manually via `IntPtr` to avoid AOT-unsupported `MarshalAs` attributes.

## Debug tools

```shell
# Audio smoke test (temporarily switches default device and restores)
dotnet run --project tests/Smoke/Smoke.csproj -c Release
```

## License

MIT
