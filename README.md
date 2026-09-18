# AudioSwitcher

Switch the default audio device, toggle mute, and cap the master volume from the Windows tray.

在 Windows 托盘切换默认音频设备、开关静音、限制主音量上限。

[English](#english) · [中文](#chinese)

---

## English

### Usage

#### Devices

- An **output** or **input** entry sets that device as the default; the checked entry is the current default.
- **Refresh Devices** re-enumerates the endpoints. Sleep/resume and replugging can leave the list stale.
- When no endpoint is enumerated at all, a grayed **No audio devices** line replaces the list.

#### Mute and volume

- **Mute** toggles global mute. A middle-click on the tray icon does the same.
- Rolling the wheel over the tray icon adjusts the volume, accelerating `1%` → `2%` → `5%` while the roll continues.
- Every volume change raises a small overlay above the tray icon: device name, slider, percentage. It hides about a second after the last notch, and immediately on any mouse button, so a right-click never leaves it over the menu it opens. The slider holds one length for every read-out (`5%`, `100%`, muted), so the bar does not move while the volume changes.
- The overlay follows the Windows shell — light/dark theme, accent colour, system menu font — and matches the visual language of the shell's own menus.

#### Volume limit

- **Volume limit** caps the master volume at `25`, `50` or `75%`.
- **Enabled** turns the cap off.

#### Hotkeys

Hotkeys are unbound by default. **Open Hotkey Settings** opens the config folder; the `hotkeys` object in `config.json` takes one combination per action, and the file header documents the format bilingually. Changes apply after a restart.

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

A combination another program owns is reported in a dialog and left unbound; the remaining hotkeys register normally.

#### System and settings

- **Volume mixer** and **Sound settings** open the system tools.
- **Run at startup** toggles login autostart. It is grayed while the state cannot be read.
- The language submenu offers **Follow System**, **中文**, **English**.
- **About** opens the release homepage; **Exit** quits and releases the hotkeys.

The overlay is the single read-out for volume and mute: the tray icon carries no tooltip, which the shell would draw exactly over the overlay. The icon is slashed while muted. Failures raise a dialog; success is silent.

### Build

```powershell
cargo build --release
# -> target/release/audio-switcher.exe
```

The binary is self-contained: icons, `VERSIONINFO` and the DPI manifest are embedded at build time, every dependency is a Rust static library, and the MSVC CRT is linked statically (`.cargo/config.toml`), so the target machine needs no VC++ Redistributable.

`scripts/package.ps1` builds and stages the release artifact `dist/audio-switcher-v<version>-x64.exe` with a `sha256sum` sidecar. `dist/` holds the current release only. The script fails unless the image imports OS DLLs alone and stays inside the size budget.

`scripts/smoke.ps1` is the gate: build, tests, `cargo fmt --all -- --check`, `cargo clippy --all-targets -- -D warnings`, and the DPI manifest. CI runs it on every push, and a pre-commit hook runs it locally (`git config core.hooksPath .githooks`, once per clone); `-GateOnly` omits the step that moves the machine's real audio devices. `tests/architecture.rs` fails the suite on a broken layering rule, a tooltip on the tray icon, a changed `windows` feature set, an oversized file, or an oversized inline test block.

CI publishes releases: a `v*` tag (or the workflow run with a tag) packages the artifact and attaches it to a GitHub release whose description is read from `.github/release-notes/<tag>.md`. The release job requires the gate, so a build that fails it is not published.

| What | Location |
| --- | --- |
| Config | `%APPDATA%\audio-switcher\` |
| Logs | `%LOCALAPPDATA%\audio-switcher\logs\` |

Runtime state lives outside the exe directory.

---

## 中文

### 使用

#### 设备

- 选择**输出**或**输入**条目即把该设备设为默认；打勾的条目是当前默认设备。
- **刷新设备**重新枚举端点。睡眠恢复与重插后列表可能过时。
- 完全没有枚举到端点时，用灰色的**无音频设备**行替代列表。

#### 静音与音量

- **静音**切换全局静音；中键点击托盘图标等效。
- 在托盘图标上滚动滚轮调节音量，持续滚动时按 `1%` → `2%` → `5%` 加速。
- 每次音量变化都在托盘图标上方浮出小块浮层：设备名、滑块、百分比。最后一次滚动约一秒后隐藏，任何鼠标键按下也立即隐藏，因此右键不会把它留在刚打开的菜单之上。滑块对所有读数（`5%`、`100%`、静音）保持同一长度，音量变化时条子不移动。
- 浮层跟随 Windows shell——深浅色主题、强调色、系统菜单字体——与 shell 自身菜单保持同一套视觉语言。

#### 音量上限

- **音量上限**把主音量封顶在 `25` / `50` / `75%`。
- **启用**关闭上限。

#### 全局快捷键

默认无绑定。**打开快捷键设置**打开配置文件夹；`config.json` 的 `hotkeys` 对象为每个动作填一个组合字符串，文件头以中英双语说明格式。改动在重启后生效。

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

被其他程序占用的组合会弹窗报告并保持未绑定；其余快捷键照常注册。

#### 系统与设置

- **音量合成器**与**声音设置**打开系统工具。
- **开机自启**开关登录自启；读取不到状态时置灰。
- 语言子菜单提供**跟随系统**、**中文**、**English**。
- **关于**打开 release 主页；**退出**退出并释放快捷键。

音量与静音的唯一读数来源是浮层：托盘图标不带提示条，系统会把它画在浮层正中。静音期间图标带斜杠。失败弹窗报告，成功静默。

### 构建

```powershell
cargo build --release
# -> target/release/audio-switcher.exe
```

产物自包含：图标、`VERSIONINFO` 与 DPI manifest 在构建时嵌入，所有依赖都是 Rust 静态库，MSVC CRT 静态链接（`.cargo/config.toml`），目标机器无需 VC++ 运行库。

`scripts/package.ps1` 构建并落盘发布工件 `dist/audio-switcher-v<version>-x64.exe` 与 `sha256sum` 格式的校验和文件。`dist/` 只保留当前发布件。镜像若导入 OS DLL 之外的库、或超出体积预算，脚本失败。

`scripts/smoke.ps1` 是门禁：构建、测试、`cargo fmt --all -- --check`、`cargo clippy --all-targets -- -D warnings` 与 DPI manifest。CI 在每次推送时运行，本地由 pre-commit hook 运行（`git config core.hooksPath .githooks`，每个 clone 一次）；`-GateOnly` 省略会改动本机真实音频设备的那一步。`tests/architecture.rs` 在下列情形使测试失败：分层规则被破坏、托盘图标带上提示条、`windows` feature 集合变动、单文件过大、内联测试块过大。

发布由 CI 完成：推 `v*` tag（或以 tag 运行 workflow）即打包工件并附到 GitHub release，描述取自 `.github/release-notes/<tag>.md`。release job 依赖门禁，门禁不过的构建不会发布。

| 内容 | 位置 |
| --- | --- |
| 配置 | `%APPDATA%\audio-switcher\` |
| 日志 | `%LOCALAPPDATA%\audio-switcher\logs\` |

运行时状态位于 exe 目录之外。
