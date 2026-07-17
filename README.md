# Audio Switcher

Windows 系统托盘音频输出设备快速切换工具。

右键点击托盘图标，在菜单中直接切换默认播放设备，无需进入系统声音设置。

## 功能

- **设备切换** — 枚举所有活动的音频渲染（输出）端点，点击即设为默认
- **三角色统一切换** — 同时设置 console / multimedia / communications 三个 ERole
- **开机自启** — 菜单中一键启用/禁用 Windows 自动启动
- **主题适配** — 自动检测系统主题（浅色/深色），使用对应配色的托盘图标
- **单实例** — 通过全局 Mutex 确保只有一个实例运行
- **轻量** — Release 构建约 500KB，无运行时依赖

## 截图

| 浅色主题 | 深色主题 |
|---|---|
| ![浅色图标](assets/icon_light.png) | ![深色图标](assets/icon_dark.png) |

## 使用

1. 从 [Releases](https://github.com/brandonmp/audio-switcher/releases) 下载最新 `audio-switcher-vX.Y.Z-win64.zip`
2. 解压到任意目录
3. 运行 `audio-switcher.exe`（无需管理员权限）

托盘图标出现后，右键打开菜单即可选择默认音频设备。

## 构建

```shell
cargo build --release
```

编译产物在 `target/release/audio-switcher.exe`，约 500KB。

### 前提

- Rust 1.75+
- Windows 10/11（x86_64）

## 项目结构

```
src/
├── main.rs         # 入口：COM 初始化、单例守卫、事件循环
├── lib.rs          # 库根，公开各模块
├── audio.rs        # 音频设备枚举 + 默认设备切换（COM → IPolicyConfig）
├── tray.rs         # 托盘菜单构建、图标加载、菜单动作分类
├── theme.rs        # Windows 主题检测（注册表）
├── startup.rs      # 开机自启管理（Run 注册表键）
└── bin/
    ├── smoke_audio.rs  # 音频模块端到端冒烟测试：枚举 → 切换 → 回滚
    ├── menu_check.rs   # 菜单项启用状态验证
    └── theme_check.rs  # 主题检测 + 图标加载验证
```

## 内部实现

### 音频切换

使用 `MMDeviceEnumerator` 枚举活动渲染端点，通过 `IPolicyConfig`（`PolicyConfigClient` COM 类）设置默认设备。核心实现参考了 [`com-policy-config`](https://crates.io/crates/com-policy-config) 的示例代码。

切换会同时覆盖三个 ERole：

- `eConsole` — 控制台
- `eMultimedia` — 多媒体
- `eCommunications` — 通信

### 托盘图标

两张 PNG（浅色/深色）在编译时 `include_bytes!` 嵌入二进制，启动时根据注册表 `AppsUseLightTheme` 值选择对应图标。

### 自启

通过 `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` 注册表键实现，仅影响当前用户。

## 调试工具

```shell
# 音频模块冒烟测试（会临时切换默认设备并恢复）
cargo run --release --bin smoke_audio

# 验证菜单项启用状态
cargo run --release --bin menu_check

# 检测主题 + 图标加载
cargo run --release --bin theme_check
```

## 许可

MIT
