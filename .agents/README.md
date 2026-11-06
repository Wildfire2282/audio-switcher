# 项目级 agent 技能

布局为 Agent Skills 约定：`skills/<name>/SKILL.md`（+ 同级 `rules/` 等资源）。
`.agents/skills` 被 omp / Amp / DSH 直接读取；其它 harness 若只看自己的目录，把它链接过去即可，例如：

```powershell
New-Item -ItemType Junction -Path .claude\skills\rust-best-practices -Target (Resolve-Path .agents\skills\rust-best-practices)
```

## 已装的技能

| 技能 | 上游 | 固定版本 | 许可 |
| --- | --- | --- | --- |
| `rust-best-practices` | https://github.com/morganlinton/BeautifulRust | 1.5.0，commit `40db61c`（2026-10-30） | MIT（见 `skills/rust-best-practices/LICENSE`） |
| `rust-review` | 同上 | 同上 | MIT（见 `skills/rust-review/LICENSE`） |
| `rust-port` | 同上 | 同上 | MIT（见 `skills/rust-port/LICENSE`） |

`skills/*/` 下的文件与上游逐字节一致，不在其中做本地改动 —— 这样升级时 `diff -r` 就能判定。上游放在技能目录之外的东西按需取：许可 `LICENSE` 复制进每个技能目录；`evals/` 是上游自己的评测夹具，不随技能安装。`rust-best-practices` 带 `references/`，另外两个技能在正文里指向它。

## 升级

```sh
git clone --depth 1 --branch v1.5.0 https://github.com/morganlinton/BeautifulRust.git /tmp/beautiful-rust
for s in rust-best-practices rust-review rust-port; do
  cp /tmp/beautiful-rust/skills/$s/SKILL.md /tmp/beautiful-rust/LICENSE .agents/skills/$s/
  rm -rf .agents/skills/$s/references
  if [ -d /tmp/beautiful-rust/skills/$s/references ]; then cp -r /tmp/beautiful-rust/skills/$s/references .agents/skills/$s/; fi
  diff -r -x evals -x LICENSE /tmp/beautiful-rust/skills/$s .agents/skills/$s   # 无输出即一致
done
```

改动后同步本表的版本号与 commit。
