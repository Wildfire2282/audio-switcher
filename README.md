# Audio Switcher

<a href="https://github.com/Wildfire2282/audio-switcher/releases">
  <img src="https://img.shields.io/github/v/release/Wildfire2282/audio-switcher?style=flat-square&color=blue" alt="Release">
</a>
<a href="https://github.com/Wildfire2282/audio-switcher/blob/main/LICENSE">
  <img src="https://img.shields.io/badge/license-MIT-green?style=flat-square" alt="License">
</a>

Windows 系统托盘音频输出设备快速切换工具。

![Tray icon](assets/icon.png)

## 功能

- **一键切换设备** — 右键托盘图标，菜单中直接选择默认播放设备，无需打开系统声音设置
- **滚轮调音** — 鼠标悬停托盘图标时滚动滚轮，实时调节音量，tooltip 显示设备名和百分比
- **左键静音** — 单击托盘图标快速静音/取消静音
- **开机自启** — 菜单中一键启用/禁用 Windows 自动启动
- **自动刷新** — 插入/拔出音频设备时自动更新设备列表
- **单实例** — 只能运行一个实例，不会重复启动
- **无运行时依赖** — 单个可执行文件，双击即用

## 使用

1. 从 [Releases](https://github.com/Wildfire2282/audio-switcher/releases) 下载 `audio-switcher.exe`
2. 双击运行（无需管理员权限）
3. 系统托盘出现波形图标后，右键选择音频设备

## 菜单说明

| 操作 | 说明 |
|------|------|
| 设备名称 | 点击设为默认播放设备，✓ 标记当前默认设备 |
| 开机自动启动 | 启用/禁用 Windows 开机自启动 |
| 刷新设备列表 | 手动刷新可用音频设备 |
| 关于 | 打开 GitHub 仓库 |
| 退出 | 关闭程序 |

## 技术栈

- C# / .NET 10
- Native Win32 P/Invoke (CsWin32 source generator)
- Core Audio API
- AOT + 单文件发布

## 下载安装

前往 [Releases 页面](https://github.com/Wildfire2282/audio-switcher/releases) 下载 `audio-switcher.exe`，双击运行。

## 许可

MIT
