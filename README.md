# Audio Switcher

Windows 系统托盘音频输出设备快速切换工具。

右键点击托盘图标，在菜单中直接切换默认播放设备，无需进入系统声音设置。

- **设备切换** — 枚举所有活动的音频渲染（输出）端点，点击即设为默认
- **三角色统一切换** — 同时设置 console / multimedia / communications 三个 ERole
- **滚轮调音** — 鼠标悬停托盘图标，滚动滚轮即可调节音量，tooltip 实时显示百分比
- **开机自启** — 菜单中一键启用/禁用 Windows 自动启动
- **关于** — 菜单中点击"关于"直接跳转到 GitHub 仓库
- **主题适配** — 自动检测系统主题（浅色/深色），使用对应配色的托盘图标
- **单实例** — 通过全局 Mutex 确保只有一个实例运行
- **AOT 单文件** — 发布为单个原生可执行文件，无需额外运行时

## 技术栈

- C# 13 / .NET 10
- 原生 Win32 P/Invoke（CsWin32 源生成器）
- Core Audio API（`MMDeviceEnumerator`、`IPolicyConfig`、`IAudioEndpointVolume`）
- 原生托盘图标与上下文菜单（`Shell_NotifyIcon` / `TrackPopupMenu`）
- AOT + 单文件发布

> 注：Windows Forms 与 .NET 10 AOT 剪裁不兼容，因此 UI 层使用原生 Win32 实现。

## 使用

1. 从 [Releases](https://github.com/Wildfire2282/audio-switcher/releases) 下载最新的 `audio-switcher.exe`
2. 双击运行（无需管理员权限）

托盘图标出现后，右键打开菜单即可选择默认音频设备。

## 构建

```shell
dotnet build
```

## AOT 单文件发布

```shell
dotnet publish -c Release -r win-x64 --self-contained true
```

编译产物在 `bin/Release/net10.0-windows/win-x64/publish/audio-switcher.exe`。

### 前提

- .NET 10 SDK
- Windows 10/11（x64）

## 项目结构

```
src/
├── Program.cs              # 入口：COM 初始化、单例守卫、消息循环
├── TrayApp.cs              # 原生托盘图标、上下文菜单、滚轮钩子
├── AudioManager.cs         # 音频设备枚举 + 默认设备切换
├── TrayMenuBuilder.cs      # 菜单项构建
├── ThemeDetector.cs        # Windows 主题检测（注册表）
├── StartupManager.cs       # 开机自启管理（Run 注册表键）
└── Interop/
    ├── ComInterfaces.cs    # Core Audio COM 接口声明
    └── Win32Defs.cs        # Win32 常量、结构、HRESULT 辅助
tests/
└── Smoke/
    └── Program.cs          # 音频模块端到端冒烟测试
```

## 内部实现

### 音频切换

使用 `MMDeviceEnumerator` 枚举活动渲染端点，通过 `IPolicyConfig`（`PolicyConfigClient` COM 类）设置默认设备。核心实现参考了 [`com-policy-config`](https://github.com/sidit77/com-policy-config)。

切换会同时覆盖三个 ERole：

- `eConsole` — 控制台
- `eMultimedia` — 多媒体
- `eCommunications` — 通信

### 托盘图标

两张 PNG（浅色/深色）作为嵌入资源打包进二进制，启动时根据注册表 `AppsUseLightTheme` 值选择对应图标。

### 自启

通过 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 注册表键实现，仅影响当前用户。

### 滚轮调音

使用 `WH_MOUSE_LL` 低层鼠标钩子截获全局滚轮事件，配合 `WM_MOUSEMOVE` 跟踪光标悬停状态。仅在悬停于托盘图标上时调节音量，通过 `IAudioEndpointVolume` COM 接口以 1% 步进调整，并更新 tooltip 显示实时音量百分比。

## 调试工具

```shell
# 音频模块冒烟测试（会临时切换默认设备并恢复）
dotnet run --project tests/Smoke/Smoke.csproj -c Release
```

## 许可

MIT
