# AudioSwitcher

Switch the default audio device, toggle mute, and cap the master volume from the Windows tray.

在 Windows 托盘切换默认音频设备、开关静音、限制主音量上限。

[English](#english) · [中文](#chinese)

---

## English

### Getting started

Run `audio-switcher-v<version>-x64.exe` from the [releases page](https://github.com/Wildfire2282/audio-switcher/releases/latest). It is a single file — no installer, no runtime to add — and it keeps to the tray: right-click the icon for the menu, **Exit** there to quit. If the icon is not visible, it is in the tray's hidden-icons list.

### Devices

- An **output** or **input** entry sets that device as the default; the checked entry is the current default.
- **Refresh Devices** re-enumerates the endpoints. Sleep/resume and replugging can leave the list stale.
- When no endpoint is enumerated at all, a grayed **No audio devices** line replaces the list.

### Mute and volume

- **Mute** toggles global mute. A middle-click on the tray icon does the same.
- Rolling the wheel over the tray icon adjusts the volume, accelerating `1%` → `2%` → `5%` while the roll continues; rolling back the other way starts a fresh `1%` step. A volume-up gesture — the wheel or the `volume_up` hotkey — also clears mute, as the volume-up key does; a volume-down gesture leaves it muted.
- Every volume change raises a small overlay above the tray icon: device name, slider, percentage. It hides about two seconds after the last change, and immediately on any mouse button, so a right-click never leaves it over the menu it opens. The slider holds one length for every read-out (`5%`, `100%`, muted), so the bar does not move while the volume changes.
- The overlay follows the Windows shell — light/dark theme, accent colour, system menu font — and matches the visual language of the shell's own menus.

### Volume limit

- **Volume Limit** caps the master volume at `25`, `50` or `75%`.
- **Enabled** toggles the cap on or off; picking a preset switches it on.

### Hotkeys

Hotkeys are unbound by default. **Open Hotkey Settings** opens the config folder; the `hotkeys` object in `config.json` takes one combination per action, and the file header documents the format bilingually. Changes apply after a restart. Holding a combination down does not repeat the action.

| Action | Config key | Step |
| --- | --- | --- |
| Toggle mute | `mute` | — |
| Volume up | `volume_up` | `2%` per press |
| Volume down | `volume_down` | `2%` per press |
| Next output device | `next_device` | — |
| Previous output device | `prev_device` | — |

Each key takes a combination string; `null` disables the action:

```json
{
  "hotkeys": {
    "mute": "Ctrl+Alt+M",
    "volume_up": null
  }
}
```

A combination another program owns is reported in a dialog and left unbound for the session; the setting stays in `config.json`, so it binds again once the other program exits, and the remaining hotkeys register normally.

### Config fields

`config.json` holds the settings the menu writes plus the hotkeys above; its header documents the same fields bilingually.

| Field | Values | Meaning |
| --- | --- | --- |
| `version` | number | Schema version the tool writes; it drives settings migration. |
| `lang` | `"system"`, `"zh"`, `"en"` | UI language; `"system"` follows the OS. |
| `volume_limit_enabled` | `true`, `false` | Whether the volume cap applies. |
| `volume_limit` | `1`–`100` | Master volume ceiling, in percent. |
| `autostart_mode` | `"off"`, `"user"`, `"admin"` | Logon start: none, a `Run` entry, or an elevated logon task. |
| `hotkeys` | object | One combination per action, as above. |

### System and settings

- **Open Volume Mixer** and **Open Sound Settings** open the system tools, centered on the screen.
- **Auto Launch** picks how the tool starts at logon: **Off**, **Standard** (a `Run` entry, no prompt), or **Administrator** (a logon task that starts the tool elevated, so the hover wheel keeps working over windows owned by elevated apps such as Tencent Androws). Switching to or from **Administrator** asks once for elevation; the group is grayed while the state cannot be read.
- The language submenu offers **Follow System**, **中文**, **English**.
- **About** opens the release homepage; **Exit** quits and releases the hotkeys.

The overlay is the single read-out for volume and mute: the tray icon carries no tooltip, which the shell would draw exactly over the overlay. The icon is slashed while muted. Failures raise a dialog; success is silent.

### Files

| What | Location |
| --- | --- |
| Config | `%APPDATA%\audio-switcher\` |
| Logs | `%LOCALAPPDATA%\audio-switcher\logs\` |

Both live outside the program's folder, so the exe can be moved or deleted on its own.

Log files older than 14 days are removed when the tool starts.

---

## 中文

### 开始使用

运行 [releases 页面](https://github.com/Wildfire2282/audio-switcher/releases/latest)的 `audio-switcher-v<版本>-x64.exe`。单个文件——无需安装、无需额外运行库——常驻托盘：右键图标打开菜单，菜单里的**退出**结束进程。若看不到图标，它在托盘的隐藏图标列表里。

### 设备

- 选择**输出**或**输入**条目即把该设备设为默认；打勾的条目是当前默认设备。
- **刷新设备**重新枚举端点。睡眠恢复与重插后列表可能过时。
- 完全没有枚举到端点时，用灰色的**未检测到音频设备**行替代列表。

### 静音与音量

- **全局静音**切换全局静音；中键点击托盘图标等效。
- 在托盘图标上滚动滚轮调节音量，持续滚动时按 `1%` → `2%` → `5%` 加速；反向滚动从新的 `1%` 步长开始。音量加的动作——滚轮或 `volume_up` 快捷键——同时解除静音，与音量加键一致；音量减保持静音。
- 每次音量变化都在托盘图标上方浮出小块浮层：设备名、滑块、百分比。最后一次变化约两秒后隐藏，任何鼠标键按下也立即隐藏，因此右键不会把它留在刚打开的菜单之上。滑块对所有读数（`5%`、`100%`、静音）保持同一长度，音量变化时条子不移动。
- 浮层跟随 Windows shell——深浅色主题、强调色、系统菜单字体——与 shell 自身菜单保持同一套视觉语言。

### 音量上限

- **音量上限**把主音量封顶在 `25` / `50` / `75%`。
- **启用**开启或关闭上限；选择预设百分比会自动开启。

### 全局快捷键

默认无绑定。**打开快捷键设置**打开配置文件夹；`config.json` 的 `hotkeys` 对象为每个动作填一个组合字符串，文件头以中英双语说明格式。改动在重启后生效。按住组合键不会重复触发该动作。

| 动作 | 配置键 | 步长 |
| --- | --- | --- |
| 静音切换 | `mute` | — |
| 音量加 | `volume_up` | 每次 `2%` |
| 音量减 | `volume_down` | 每次 `2%` |
| 下一个输出设备 | `next_device` | — |
| 上一个输出设备 | `prev_device` | — |

每个键填组合字符串；`null` 关闭该动作：

```json
{
  "hotkeys": {
    "mute": "Ctrl+Alt+M",
    "volume_up": null
  }
}
```

被其他程序占用的组合会弹窗报告，本次运行保持未绑定；该设置会留在 `config.json` 里，占用程序退出后即恢复生效，其余快捷键照常注册。

### 配置字段

`config.json` 保存菜单写入的设置与上文的快捷键；文件头以中英双语说明同样的字段。

| 字段 | 取值 | 含义 |
| --- | --- | --- |
| `version` | 数字 | 工具写入的结构版本，驱动设置迁移。 |
| `lang` | `"system"`、`"zh"`、`"en"` | 界面语言；`"system"` 跟随系统。 |
| `volume_limit_enabled` | `true`、`false` | 是否启用音量上限。 |
| `volume_limit` | `1`–`100` | 主音量上限（百分比）。 |
| `autostart_mode` | `"off"`、`"user"`、`"admin"` | 登录启动方式：不启动、注册表 `Run`、管理员登录任务。 |
| `hotkeys` | 对象 | 每个动作一个组合字符串，见上文。 |

### 系统与设置

- **打开音量合成器**与**打开声音设置**打开对应的系统工具，窗口居中显示。
- **开机自启**选择登录时的启动方式：**关闭**、**普通权限**（注册表 `Run`，不弹 UAC）、**管理员权限**（登录任务，以管理员启动，因此在管理员权限的应用窗口——例如腾讯应用宝 / Androws——上面悬停滚轮仍然有效）。在**管理员权限**与其它方式之间切换会弹一次 UAC；读取不到状态时整组置灰。
- 语言子菜单提供**跟随系统**、**中文**、**English**。
- **关于**打开 release 主页；**退出**退出并释放快捷键。

音量与静音的唯一读数来源是浮层：托盘图标不带提示条，系统会把它画在浮层正中。静音期间图标带斜杠。失败弹窗报告，成功静默。

### 文件

| 内容 | 位置 |
| --- | --- |
| 配置 | `%APPDATA%\audio-switcher\` |
| 日志 | `%LOCALAPPDATA%\audio-switcher\logs\` |

两者都在程序目录之外，exe 可单独移动或删除。

启动时清理超过 14 天的旧日志。
