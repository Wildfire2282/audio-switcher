# audio-switcher 维护规则

入口：`src/lib.rs`（分层、不变量、任务→文件索引）。本文件只写过程约束与红线，不复述代码、README 或 crate 文档已有的内容。改本文件：只留稳定约束；会漂移的事实指向源头。

## 红线

- 托盘不得有 tooltip：反馈通道是 `platform::osd` 浮层。
- 不扩大 `windows` crate 的 feature 集合（门禁锁定）。
- 不新建 `docs/`、SPEC、scheme 文档树；新规范写进 crate 文档。
- 删代码删干净：不留 `_` 变量、兼容 shim、`// removed`。
- 改 `platform/` 必须同步 `#[cfg(not(windows))]` 分支；本机编译不到。
- 加依赖前跑 `scripts/package.ps1` 看体积。上限 819,200 字节，落盘前强制。
- `master` 有分支保护：禁止 force push 与删除。要改历史就开分支。

## 注释

规则在 `src/lib.rs` 的 `# Comments`。判据只有一条：删掉它，读者会不会写出错误的改动？改行为必须在同一次改动里改注释。

## 验证

- 宣称完成前跑 `pwsh scripts/smoke.ps1 -GateOnly`（build → test → fmt → clippy → manifest）与 `pwsh scripts/package.ps1`。
- `cargo test` 数量只增不减；变少要能说清原因。
- `tests/architecture.rs` 是门禁：分层、托盘 tooltip、`windows` feature 集合、文件体积、内联测试体积、本文件体积、发布描述格式。违反时改代码，不改测试。
- `#[ignore]` 测试会改本机真实录音设备或注入合成输入：单独显式执行，不要顺手跑。
- 测试全绿 ≠ 可用。UI 改动必须给运行时证据：悬停托盘图标滚轮 → 浮层出现并按期消失；空闲 CPU 秒数不涨；滚 50 次 GDI 句柄增量为 0。
- 排查运行时问题用 `$env:AUDIO_SWITCHER_LOG=debug`（或 `trace`）后重启，日志在 `%LOCALAPPDATA%\audio-switcher\logs\`。不要插临时日志。
- 渲染与几何改动扩 `platform/osd/win/tests.rs` 的像素回读测试；纯格式化放 `ui/osd/tests.rs`。

## 变更流程

- commit 英文、一行一事、写用户可见变更；多行用 `- ` bullets。
- 版本号单独 commit（`chore: bump version to X`）；`Cargo.toml` 与 `Cargo.lock` 同步。
- 行为变了同时改 `README.md`。它面向**终端用户**：只写怎么获取、怎么用、config 字段与文件位置；构建、门禁、CI、发布流程属于本文件与 workflow，不进 README。写法：当前状态的说明书，不写「现在 / 不再 / 曾经」这类语气；精准、简练、正式；中英分节与断言逐条平行。
- 只在明确要求时提交或推送；推送后确认远端同步。

## 打包与发布

- `dist/` 只保留当前发布件：`audio-switcher-v<版本>-x64.exe` 与同名 `.sha256`（小写哈希、两空格、裸文件名）。打包前先杀掉残留实例。
- 发布描述放 `.github/release-notes/<tag>.md`：不写哈希与体积，不从 commit log 生成；格式由 `tests/architecture.rs` 校验。
- CI 负责打包与发布：推 `v*` tag 或手动 `workflow_dispatch`；`release` job 依赖 `gate`，门禁不过不发布。
- 本机启用提交门禁：`git config core.hooksPath .githooks`；每个 clone 一次。
