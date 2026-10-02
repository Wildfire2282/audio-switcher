# 托盘图标偶发空 tooltip —— 完整调查交接文档

> 状态：**现象已复现并量化，成因未定，修复未做**。分支 `fix/tray-empty-tooltip` 已建。
>
> ⚠️ **最重要的一点：成因没有定论。** 第 4 节的实测数据是事实，
> 但"是什么画的、为什么"是**推断**。第 7 节列出两个互斥可能，**都未被证实**。
> 不要把第 11 节的小结当成已确认的根因。
> 全文事实标 `[实测]`、推断标 `[推断]`。

---

## 1. 现象

鼠标悬浮在 audio-switcher 的托盘图标上时，**偶发**（大部分时间不出现）弹出一个 tooltip 框，
**框存在但里面没有任何文字**。框约 31×25 像素，悬在图标正上方。

用户报告"触发原因不明，大部分时间一切正常"。

---

## 2. 环境

| 项 | 值 | 来源 |
|---|---|---|
| 系统 | Windows 11 26H2，build 26300.9457 | `cmd /c ver` |
| 显示器 | 3840×2160 单屏，虚拟屏幕原点 (0,0) | `GetSystemMetrics` |
| 项目 | Rust，audio-switcher（托盘音量切换器） | — |
| 运行实例 | `audio-switcher-v0.9.2-x64`，pid 7544 | `tasklist` |
| 相关依赖 | `tray-icon` 0.24.2、`muda` 0.19.3、`windows` 0.62.2 | `Cargo.lock` |

---

## 3. 关键依赖事实 `[实测]`

本节全部来自**阅读 crate 源码**（路径与行号已给），是确定的代码行为。
但请注意：**这些事实本身并不证明空框就是它们造成的** —— 见第 7 节。

### 3.1 tray-icon 0.24.2 注册托盘图标时做了什么

源码 `tray-icon-0.24.2/src/platform_impl/windows/mod.rs`，函数 `register_tray_icon`（约 566–611 行）：

```rust
let mut h_icon = std::ptr::null_mut();
let mut flags = NIF_MESSAGE;
let mut sz_tip: [u16; 128] = [0; 128];          // ← 永远是全零数组

if let Some(hicon) = hicon {
    flags |= NIF_ICON;
    h_icon = *hicon;
}

if let Some(tooltip) = tooltip {
    flags |= NIF_TIP;                             // ← 只有传了 tooltip 才加
    ...
}

let mut nid = NOTIFYICONDATAW {
    uFlags: flags,
    ...
    szTip: sz_tip,                                // ← 无论如何都传进结构体
    ...
};
Shell_NotifyIconW(NIM_ADD, &mut nid as _)
```

**含义**：`NIF_TIP` 标志没设，但 `szTip` 字段仍然被填了一个全零数组发过去 ——
即发送方没有声明"这里有一个提示"，但也没有让 shell 收到"无提示"这个状态。

### 3.2 图标停在 legacy（老）接口版本

- 整个 crate **没有任何 `NIM_SETVERSION` 调用**（grep 确认）。
- `NOTIFYICONDATAW.uVersion` 默认为 0，即 `NOTIFYICON_VERSION_0`。
- Microsoft 文档：`NIF_SHOWTIP` 的语义是"**当 uVersion 设为 NOTIFYICON_VERSION_4 时，
  标准 tooltip 会被抑制**，应用可自行绘制 popup；想显示才需要显式指定 NIF_SHOWTIP"。
- 即：**老接口下标准 tooltip 一定会被画**；新接口下默认不画。

### 3.3 我们自己的代码没有设置过任何 tip

`src/ui/tray.rs` 中构建图标：

```rust
let tray = TrayIconBuilder::new()
    .with_icon(icon)
    .with_menu(Box::new(handles.menu.clone()))
    .with_menu_on_left_click(false)
    .build()?;
```

无 `with_tooltip`。`set_icon` 发送的 `uFlags` 只有 `NIF_ICON`，不触碰 tip。
`src/ui/tray.rs::update_icon_if_changed` 还有 `pushed_mute` 守卫，
只在静音状态真正变化时才发 `NIM_MODIFY`（不是每次滚轮）。

muda 完全不碰 `Shell_NotifyIcon`（grep 无匹配）。

### 3.4 升级 tray-icon 不会改变上述两点

已检查最新版本 **0.26.0** 的 windows 实现：同样没有 `NIM_SETVERSION`、没有 `NIF_SHOWTIP`。
即"升级依赖"不会改变 3.1/3.2 的行为。`[推断]` 因此它也就不会消除空框——
这一推论只有在成因确实是 3.1/3.2 时才成立。

---

## 4. 运行时证据 `[实测]`

本节是**本机观测到的现象**。数据是真的；但对数据的**解释**在每小节末尾标出 `[推断]`，
不要把解释当结论。

### 4.1 我们的图标在系统里没有 tip 文字 —— UI Automation 直接印证

用 UI Automation 枚举 `Shell_TrayWnd` 下所有按钮，**只有一个按钮名字为空**：

```
btn '显示隐藏的图标'                     3321,2088 48x72
btn '红果免费短剧'                       3369,2088 48x72
btn ' [Hysteria2] hy2-GreenCloud(173…)'  3417,2088 48x72
btn ''                                   3465,2088 48x72   ← 这是 audio-switcher
btn '任务栏输入指示 单击右键…'            3513,2088 48x72
btn '任务栏输入指示 简体中文(中国大陆)…'   3561,2088 66x72
btn '网络 Abbie / Internet 访问'          3633,2088 36x72
btn '音量 扬声器 (EDIFIER M25): 15%'      3675,2088 36x72
```

这个空名字的按钮就是我们的图标 `[推断]`（位置上在 Hysteria2 与输入法指示器之间，
且应用进程确实注册了托盘图标 —— 但两者的**位置对应**未做严格验证）。
`[实测]` 它的 UIA Name 为空、其余每个图标都有名字 —— 这与 3.1 的代码事实**一致**。

### 4.2 空框的几何位置

用户 Win+Shift+S 截图（裁剪后 400×179），像素分析结果：

```
任务栏上边缘：  y = 107
空框 bbox：     x = 222..252, y = 65..89   →  31 × 25 px，中心 x = 237
耳机图标 glyph：x = 234..253              →  中心 x = 243.5
```

- 空框**横向以悬停的图标为中心**（237 vs 243.5，差 6px 在抗锯齿/对齐误差内），
  纵向位于任务栏上方 18~43 px。`[推断]` 这与 shell 标准 tooltip 的位置一致。
- **排除 OSD**：`AudioSwitcherVolumeOsd` 卡片是 230×60 逻辑像素、带 4px 圆角和洋红色
  color key，且只在滚轮后显示 `VISIBLE_MS = 2000` ms。31×25 的空框尺寸对不上，
  且纯悬浮（无滚轮）不会触发它。
- **排除我们的其他窗口**：应用进程的全部顶层窗口为
  `AudioSwitcherVolumeOsd`、`tray_icon_app`、MSCTFIME UI、IME，后三个都不是可见窗口。

### 4.3 对照实验 —— 不只我们这样，系统图标更频繁

用像素差分法：对每个托盘图标，把鼠标移开 → 截取任务栏上方区域作为基线 →
移到该图标上悬浮 2.4 秒 → 再截图 → 统计变化像素数（阈值 24/通道）。
**每个图标 8 次**。

```
show-hidden    x=3345 bubbleHits=1/8 maxChangedPx=2480
hongguo        x=3393 bubbleHits=0/8 maxChangedPx=0
hysteria2      x=3441 bubbleHits=0/8 maxChangedPx=0
no-name(我们)  x=3489 bubbleHits=1/8 maxChangedPx=516
ime-indicator  x=3537 bubbleHits=0/8 maxChangedPx=0
ime            x=3594 bubbleHits=0/8 maxChangedPx=0
network        x=3651 bubbleHits=0/8 maxChangedPx=0
volume         x=3693 bubbleHits=4/8 maxChangedPx=1249   ← 频率最高
```

**结论**：Windows 自带的**音量**图标 8 次里出 4 次，比我们（1/8）**高得多**。
其余第三方图标（红果、Hysteria2）为 0，可能只是各自 tip 有文字导致判定不同 `[推断]`。

`[推断]` 这**削弱了**"空框是我们代码特有 bug"的假设（音量图标由系统拥有，不由我们控制）。
但它**不足以**断定成因是系统行为：样本仅 8 次，且"有 tip 文字的图标也弹空框"
这一情形在别处有报告（见 7 节），本实验没有排除它。

### 4.4 探测方法上的一个坑（避免别的 AI 重蹈覆辙）

**PowerShell 默认不是 DPI-aware，`SetCursorPos` 会被钳制**。
早期探测里请求 `(3687, 2100)`，实际只移动到 `(2559, 1439)`，
导致"悬浮了却什么都没抓到"的假阴性，浪费了大量时间。

**必须先调用**：

```csharp
[DllImport("user32.dll")] static extern bool SetProcessDpiAwarenessContext(IntPtr v);
SetProcessDpiAwarenessContext(new IntPtr(-4));   // PER_MONITOR_AWARE_V2
```

另一个坑：PowerShell 里 `function Tips() {...}` 会解析失败（与内置别名冲突），
函数名要写成 `Get-TipWindows` 这类动词开头的形式。

另外：**Win11 26H2 的托盘 tooltip 画在任务栏的 XAML island 内部，不是顶层窗口**。
枚举 `tooltips_class32` 永远抓不到东西（该类窗口存在但 `IsWindowVisible` 恒为 false）。
**唯一可靠的检测手段是屏幕像素截图。**

---

## 5. 项目自身的硬约束（任何修复都必须遵守）

### 5.1 门禁测试会直接拒绝

`tests/architecture.rs::tray_carries_no_tooltip` 扫描整个 `src/`（已剥离注释），
以下字符串**任何一处出现即测试失败**：

```
with_tooltip / update_tooltip / set_tooltip / tooltip / NIF_TIP
```

注释里写明理由：*"the shell draws a tooltip exactly where the volume overlay appears
and covers it"* —— tooltip 会**遮挡音量浮层**。

### 5.2 `AGENTS.md` 红线

> 托盘不得有 tooltip：反馈通道是 `platform::osd` 浮层。

违反时规矩是**改代码不改测试**。

### 5.3 `windows` crate feature 集合被锁定

`tests/architecture.rs::windows_features_are_the_approved_set` 逐项比对 `Cargo.toml`。
好消息：`Win32_UI_Shell` **已在列表中**，所以 `Shell_NotifyIconW`、`NOTIFYICONDATAW`、
`SetWindowSubclass`、`DefSubclassProc`、`RemoveWindowSubclass` **无需新增 feature 即可使用**。

### 5.4 体积预算

release 二进制上限 **819,200 字节**，`scripts/package.ps1` 落盘前强制。

---

## 6. 现有 OSD 的精确参数（评估"用浮层盖住"方案时需要）

`src/platform/osd.rs` + `src/platform/osd/win/layout.rs`：

| 项 | 值 |
|---|---|
| 窗口类名 | `AudioSwitcherVolumeOsd` |
| 卡片尺寸 | `CARD_W=230` × `CARD_H=60` 逻辑像素（按 DPI 缩放） |
| 与图标间距 | `GAP=10` 逻辑像素（浮层在图标**上方**） |
| 圆角 / color key | `RADIUS=4` / 洋红 `(0xFF,0x00,0xFF)` |
| 扩展样式 | `WS_EX_TOPMOST \| WS_EX_TOOLWINDOW \| WS_EX_NOACTIVATE \| WS_EX_LAYERED \| WS_EX_TRANSPARENT` |
| 显示时长 | `VISIBLE_MS = 2000` ms，仅滚轮后显示 |
| 几何锚点 | 水平居中于托盘图标 rect，空框 bbox 完整落在浮层范围内 ✅ |

---

## 7. 尚未解决的核心问题（决定选哪条修复路线）

### ❓ 关键未知：这个空框究竟是什么

两种可能，**解法完全不同**：

- **(a) shell 把我们传的空字符串 tip 画成了框**
  → 填上文字（方案 A）即可根治。
- **(b) shell 额外多画了一个框，与 tip 内容无关**
  → 填文字只会变成"名字 + 空框"（方案 A 失效）；
    换新版接口（方案 B）也可能关不掉；
    只剩"浮层常驻盖住"（方案 C）。

### 支持 (b) 的外部证据

有开发者在**图标已设置正常 tip 文字**的情况下，仍报告多弹一个空白框，
并被维护者确认为 Windows 自身的 bug：

- `hardcodet/wpf-notifyicon#65` / `#123`
  —— 报告者原话："it happens intermittently. For example, you won't see the empty
  Windows tray icon tooltip at all on many hovers but then it'll be there on the
  next one without anything in particular seeming to have changed."
- `dotnet/winforms#15051` —— `NotifyIcon.Text` **从未设置**也出现空框，Win11 25H2 26200.9457，
  明确指出是间歇性的，附最小复现仓库。
- `anthropics/claude-code#91606` —— 枚举托盘工具栏证明：无 tip 的图标其 tip 字段是 `""`
  而非"不存在"，NVDA 会朗读为"未命名按钮"。
- `theotherp/nzbhydra2#956`、`microsoft/PowerToys#49555`、`EarTrumpet#461`、
  `wpf-notifyicon` PR #96（`NIF_SHOWTIP` 相关讨论）。

**注意：这些外部证据只能证明"这是系统侧的已知问题"，不能替我们判定 (a) 还是 (b)。**
本机的对照实验（4.3）也倾向 (b)：音量图标有文字却出框更频繁。

### ❓ 次要未知：为什么偶发

`[实测]` 它**不是**"鼠标停上去就必然弹"（8 次里出 1 次）。
`[推断]` 与提示内容无关 —— 这只是由音量图标更频繁推出的，**没有直接证据**：
也可能只是触发阈值不同。shell 内部的判定逻辑不可控、不可见。

---

## 8. 候选方案

### 修复类

| 方案 | 做法 | 工作量 | 能否根除 | 风险 / 代价 |
|---|---|---|---|---|
| **A** 填应用名 | `szTip` 写入应用名 | 一行 | **取决于 (a)/(b)** | 可能变成"名字+空框"；遮挡 OSD（违反 5.1/5.2）；用户无法再看到设备名等信息 |
| **B** 换新版接口 | `Shell_NotifyIcon(NIM_SETVERSION, uVersion=NOTIFYICON_VERSION_4)`，不设 `NIF_SHOWTIP` | **~200 行 + 消息钩子** | **可能无效**（若为 (b)） | 见下 |

**方案 B 的隐藏成本（必须知道）**：
1. v4 会改变回调消息打包（`LOWORD(lParam)`=事件，`HIWORD(lParam)`=icon id，
   `wParam`=坐标），而 **tray-icon 0.24.2 只解 v0 格式 → 直接切换会导致右键菜单完全失效**。
2. 因此必须额外用 `SetWindowSubclass` 挂一个子类过程，把 v4 消息翻译回 v0 格式再交给库。
   这是本项目目前**没有的一类代码**（带 `unsafe` 的 Win32 钩子钩子）。
3. Explorer 重启时托盘图标会重建（tray-icon 的 `TaskbarCreated` 处理），
   **重建后 uVersion 回落为 0，必须重新发送 `NIM_SETVERSION`** —— 极易遗漏。
4. tray-icon **不暴露 `internal_id`（uID）**，只能拿到 `window_handle()`（hwnd）。
   经推断：crate 内 `static COUNTER` 从 1 开始，`TrayIconId::new_unique()` 先取走 1，
   `TrayIcon::new()` 再取走 2，因此首个图标的 `uID == 2`。**这个推断尚未成功验证**
   （`Shell_NotifyIconGetRect` 在本机返回 0×0，未能确认）。

### 绕过类

| 方案 | 做法 | 工作量 | 效果 | 代价 |
|---|---|---|---|---|
| **C** 浮层常驻盖住 | 浮层改为常驻显示 | 改几行 | **可能盖不住** | 见下 |
| **D** 接受现状 | 不改 | 无 | 空框偶发 | 系统行为，音量图标同样有，用户基本无感 |

**方案 C 的三个真实代价**：
1. **Z 序不保证**。`WS_EX_TOPMOST` 只保证排在普通窗口之上；
   任务栏的 XAML island 本身也是置顶的，两者**同处置顶再排序**，
   谁压谁取决于激活/移动先后。**必须实测**。
2. **常驻卡片本身**。任务栏上方永远挂一个 230×60 的音量卡片 —— 比偶发空框更烦人。
3. **盖住邻居与溢出菜单**。230px 宽度横跨左右相邻图标（左边 Hysteria2、右边输入法指示器），
   它们的提示框会被遮挡；点"显示隐藏的图标"弹出的面板**不是置顶窗口，会被压在下面**。

---

## 9. 建议的推进顺序

1. **先做 A 的验证版**（改一行 + 编译），悬浮几次观察：
   - 现象 = "只有应用名" → **(a) 成立**，收工
   - 现象 = "名字 + 空框" → **(b) 成立**，A 失效，转 B
   这一步最便宜，且结果对 B 和 C 都有指导意义。
2. 若 (a)：处理遮挡 OSD 的问题（如仅在浮层未显示时才保留名字），并处理门禁 5.1。
3. 若 (b)：做 B；B 无效再考虑 C。
4. **C 无论如何都需要用户拍板** —— "任务栏上方永远挂个卡片"通常比偶发空框更不可接受。

**更彻底的先行验证（不改代码，推荐）**：
对**正在运行的**实例直接发送一条 `NIM_SETVERSION` 切换指令
（需先确认 `hwnd=131792` + `uID=2`），然后悬浮几次看框还在不在。
- 框消失 → 方案 B 值得写
- 框还在 → 方案 B 作废

副作用：切换期间右键菜单失效，重启应用即恢复。

---

## 10. 调查过程中产生的临时文件（**已清理**）

`scripts/` 目录下曾有 17 个探测脚本，均为本次调查临时产物，**已于调查结束后全部删除**：

```
probe-tray-tip.ps1        probe-tray-owner.ps1      probe-tray-owner2.ps1
probe-tray-tree.ps1       probe-tray-rects.ps1      probe-tray-id.ps1
probe-tip-hover.ps1       probe-tip-sweep.ps1       probe-tip-hunt.ps1
probe-tip-ab.ps1          probe-window-diff.ps1     probe-any-window.ps1
probe-tray-shot.ps1       probe-bubble-scan.ps1     probe-nid-layout.ps1
probe-cursor.ps1          probe-screen.ps1          scan_bubble.py
```

截图产物在 `%LOCALAPPDATA%\Temp\bubblescan\` 和 `%LOCALAPPDATA%\Temp\trayshots\`。
`scripts/` 现仅剩项目自有的 `package.ps1` 与 `smoke.ps1`。

**用户投诉记录**：调查期间这些脚本会控制鼠标来回移动以模拟悬浮，
其中 PowerShell 版本（`probe-bubble-scan.ps1`）在用户要求停止后仍在后台运行约 6 分钟。
**后续若再做自动化探测，必须先与用户确认，并保证可一键终止。**

---

## 11. 小结

**已确证 `[实测]`**：现象存在且可复现；空框出现在 shell 绘制 tip 的位置；
我们的图标在系统侧确实没有 tip 文字（UIA 名称为空）；系统自带音量图标出现得更频繁
（4/8 vs 我们 1/8）；`tray-icon` 0.24.2 注册时发送空 `szTip` 且从未调用 `NIM_SETVERSION`
（读 crate 源码确认）。

**未确证 `[推断]`**：**"空框的成因"没有定论。** 第 7 节的 (a)/(b) 两个可能
——「shell 把空 tip 渲染成了框」还是「shell 额外多画了一个框」——**都未被证实**。
音量图标更频繁这一数据倾向于 (b)（即系统行为），但 8 次样本量小，
且无法排除"有文字的 tip 也照样弹空框"这一已见于第三方 issue 的情形。

**因此**：A / B / C 三个方案**全部建立在未验证的前提上**。
唯一能一次性判定前提的，是第 9 节末尾那个**不需要改代码**的验证
（对运行中的实例发一条 `NIM_SETVERSION`，再悬浮观察）—— 它至今没做。

不做修复的理由是**偶发、频率低、纯外观问题**，而非技术上不可行。