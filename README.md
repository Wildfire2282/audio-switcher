# Audio Switcher

**Language / 语言**：`English` | [中文](#中文)

<details open>
<summary><b>English</b></summary>

Quick audio output device switcher for the Windows system tray.

Right-click the tray icon to switch the default playback device directly — no need to open Sound settings.

- **Device switching** — enumerates all active render endpoints, one click sets default
- **Three-role switch** — sets console / multimedia / communications ERoles simultaneously
- **Wheel volume** — scroll over the tray icon to adjust volume, instant tooltip shows device name + percentage
- **Auto-start** — toggle Windows startup from the menu
- **About** — menu link to the GitHub repo
- **Custom tray icon** — waveform icon embedded as resource, same across light/dark themes
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
├── Locale.cs               # zh/en system language detection
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

A single waveform icon (`assets/icon.png`) is embedded as a resource. It is converted to a proper `.ico` at runtime via `Icon.Save` to preserve the alpha channel, ensuring sharp rendering on any theme.

### Auto-start

Managed via `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, per-user only.

### Wheel volume

A `WH_MOUSE_LL` low-level mouse hook captures global scroll events. Hover detection uses `Shell_NotifyIconGetRect` to check whether the cursor is inside the tray icon's bounding rectangle at the time of the event — no timers, no lag. Volume is adjusted in 1% steps via `IAudioEndpointVolume`, with an instant tooltip showing device name + percentage.

### DPI awareness

`SetProcessDpiAwarenessContext` with `DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2` is called at startup, so the native context menu renders at native resolution on high-DPI (4K) displays.

### AOT COM interop

All Core Audio COM interfaces use `[GeneratedComInterface]` (source-generated COM) instead of `[ComImport]` (runtime-based, fails under AOT). RCW creation goes through `StrategyBasedComWrappers`, and all string/boolean marshalling is done manually via `IntPtr` to avoid AOT-unsupported `MarshalAs` attributes.

### Localization

Detects `CultureInfo.InstalledUICulture` at startup. Shows Chinese when the system language is Chinese; English for everything else.

## Debug tools

```shell
# Audio smoke test (temporarily switches default device and restores)
dotnet run --project tests/Smoke/Smoke.csproj -c Release
```

## License

MIT

</details>

<details>
<summary><b>中文</b></summary>

<a name="中文"></a>

Windows 系统托盘音频输出设备快速切换工具。

右键点击托盘图标，在菜单中直接切换默认播放设备，无需进入系统声音设置。

- **设备切换** — 枚举所有活动的音频渲染端点，点击即设为默认
- **三角色统一切换** — 同时设置 console / multimedia / communications 三个 ERole
- **滚轮调音** — 鼠标悬停托盘图标滚动滚轮调节音量，tooltip 即时显示设备名+百分比
- **开机自启** — 菜单中一键启用/禁用 Windows 自动启动
- **关于** — 菜单链接到 GitHub 仓库
- **自定义图标** — 波形图标作为嵌入资源，浅色/深色主题通用
- **单实例** — 全局 Mutex 确保只有一个实例运行
- **AOT 单文件** — 发布为单个原生可执行文件，无需运行时

## 技术栈

- C# 13 / .NET 10
- 原生 Win32 P/Invoke（CsWin32 源生成器）
- Core Audio API（`MMDeviceEnumerator`、`IPolicyConfig`、`IAudioEndpointVolume`）
- 原生托盘图标与上下文菜单（`Shell_NotifyIcon` / `TrackPopupMenu`）
- AOT + 单文件发布

> 注：Windows Forms 与 .NET 10 AOT 剪裁不兼容，UI 层使用原生 Win32。

## 使用

1. 从 [Releases](https://github.com/Wildfire2282/audio-switcher/releases) 下载 `audio-switcher.exe`
2. 双击运行（无需管理员权限）

托盘图标出现后右键即可选择默认音频设备。

## 构建

```shell
dotnet build
```

## AOT 单文件发布

```shell
dotnet publish -c Release -r win-x64 --self-contained true
```

输出：`bin/Release/net10.0-windows/win-x64/publish/audio-switcher.exe`（约 2.7 MB）。

### 前提

- .NET 10 SDK
- Windows 10/11（x64）

## 项目结构

```
src/
├── Program.cs              # 入口：COM 初始化、单实例守卫、消息循环
├── TrayApp.cs              # 原生托盘图标、上下文菜单、滚轮钩子
├── AudioManager.cs         # 音频设备枚举 + 默认设备切换
├── TrayMenuBuilder.cs      # 菜单项构建
├── Locale.cs               # 中/英系统语言检测
├── ThemeDetector.cs        # Windows 主题检测（注册表）
├── StartupManager.cs       # 开机自启管理（Run 注册表键）
└── Interop/
    ├── ComInterfaces.cs    # Core Audio COM 接口（GeneratedComInterface）
    ├── Win32Defs.cs        # Win32 常量、结构、HRESULT 辅助
    └── NativeMethods.cs    # AOT 安全 CoCreateInstance + DPI 感知
tests/
└── Smoke/
    └── Program.cs          # 音频模块端到端冒烟测试
```

## 内部实现

### 音频切换

使用 `MMDeviceEnumerator` 枚举活动渲染端点，通过 `IPolicyConfig`（`PolicyConfigClient` COM 类）切换默认设备。核心实现参考 [`com-policy-config`](https://github.com/sidit77/com-policy-config)。

同时覆盖三个 ERole：

- `eConsole` — 控制台
- `eMultimedia` — 多媒体
- `eCommunications` — 通信

### 托盘图标

单个波形图标（`assets/icon.png`）作为嵌入资源。运行时通过 `Icon.Save` 转换为标准 `.ico` 格式以保留 Alpha 通道，在任何主题下都能清晰显示。

### 自启

通过 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 实现，仅影响当前用户。

### 滚轮调音

`WH_MOUSE_LL` 低层鼠标钩子截获全局滚轮事件。悬停检测使用 `Shell_NotifyIconGetRect` 判断光标是否在图标矩形内——无定时器、无延迟。通过 `IAudioEndpointVolume` 以 1% 步进调音，tooltip 即时显示设备名+百分比。

### DPI 感知

启动时调用 `SetProcessDpiAwarenessContext`（`DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2`），原生菜单在 4K 高 DPI 显示器上以原生分辨率渲染。

### AOT COM 互操作

所有 Core Audio COM 接口使用 `[GeneratedComInterface]`（源生成 COM）而非 `[ComImport]`（运行时模式，AOT 下失败）。RCW 创建通过 `StrategyBasedComWrappers`，字符串/布尔编组手动通过 `IntPtr` 完成，以避免 AOT 不支持的 `MarshalAs` 属性。

### 本地化

启动时检测 `CultureInfo.InstalledUICulture`，系统语言为中文时显示中文，其余一律英语。

## 调试工具

```shell
# 音频模块冒烟测试（会临时切换默认设备并恢复）
dotnet run --project tests/Smoke/Smoke.csproj -c Release
```

## 许可

MIT

</details>
