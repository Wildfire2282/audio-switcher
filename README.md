# Audio Switcher

系统托盘音频设备切换工具 — 在 Windows 任务栏直接切换默认音频输出设备。

![GitHub release](https://img.shields.io/github/v/release/Wildfire2282/audio-switcher)
![Rust](https://img.shields.io/badge/language-Rust-orange)
![Windows](https://img.shields.io/badge/platform-Windows-blue)

## 功能

- 右键系统托盘图标，列出所有活跃的音频输出设备（渲染端点）
- 一键切换默认设备（同时作用于 Console、Multimedia、Communications 三类角色）
- 开机自启开关（通过 `HKCU\Run` 注册表）
- 自适应 Light/Dark 主题图标
- 单实例守护（全局 Mutex 防止多开）
- 刷新设备列表（新增/移除设备后无需重启）

## 截图

| Light 主题 | Dark 主题 |
|---|---|
| ![light](assets/icon_light.png) | ![dark](assets/icon_dark.png) |

托盘图标根据 Windows 主题色自动切换黑白版本。

## 系统要求

- Windows 10 / 11（x64）
- Visual Studio 2022 Build Tools 或 Visual Studio 2022（C++ 工作负载，用于 MSVC 链接器）

## 构建

```powershell
# 1. 确保已安装 Rust（https://rustup.rs）
# 2. 在 Visual Studio Developer PowerShell 中运行：
cargo build --release
```

或者使用 Package 脚本（自动配置 VS 环境并打包）：

```powershell
.\package.ps1
```

编译产物位于 `target/release/audio-switcher.exe`。

### 测试工具

项目附带三个独立测试二进制，可在 Developer PowerShell 中运行：

```powershell
# 枚举设备并逐一切换验证（会修改当前默认设备然后恢复）
cargo run --release --bin smoke_audio

# 检查菜单项状态
cargo run --release --bin menu_check

# 检测系统主题并验证图标嵌入
cargo run --release --bin theme_check
```

## 使用方法

1. 运行 `audio-switcher.exe`
2. 系统托盘出现音频图标
3. 右键图标 → 选择设备切换
4. 切换开机自启：右键 → 勾选「开机自动启动」
5. 右键 → 「退出」关闭程序

## 技术细节

- 音频设备枚举和默认设备切换通过 Windows COM API（`MMDeviceEnumerator` + `IPolicyConfig`）实现
- 使用 `PolicyConfigClient` COM 类（`com-policy-config` crate）切换默认设备
- 主题检测读取 `HKCU\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize\AppsUseLightTheme`
- PNG 图标在编译时通过 `include_bytes!` 嵌入二进制
- 隐藏控制台窗口（`#![windows_subsystem = "windows"]`）

## 依赖

- `tao` — 事件循环
- `tray-icon` — 系统托盘
- `windows` — Windows API 绑定
- `com-policy-config` — PolicyConfig COM 封装
- `widestring` — UTF-16 字符串处理
- `image` — PNG 解码

## 许可

MIT
