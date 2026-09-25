# 项目级 agent 技能

布局为 Agent Skills 约定：`skills/<name>/SKILL.md`（+ 同级 `rules/` 等资源）。
`.agents/skills` 被 omp / Amp 直接读取；其它 harness 若只看自己的目录，把它链接过去即可，例如：

```powershell
New-Item -ItemType Junction -Path .claude\skills\rust-skills -Target (Resolve-Path .agents\skills\rust-skills)
```

## 已装的技能

| 技能 | 上游 | 固定版本 | 许可 |
| --- | --- | --- | --- |
| `rust-skills` | https://github.com/leonardomso/rust-skills | 1.5.1，commit `fd2a861`（2026-06-14） | MIT（见 `skills/rust-skills/LICENSE`） |

`skills/rust-skills/` 下的文件与上游逐字节一致（`SKILL.md`、`rules/`、`LICENSE`、`CHANGELOG.md`），不在其中做本地改动 —— 这样升级时 `diff -r` 就能判定。

## 升级

```sh
git clone --depth 1 https://github.com/leonardomso/rust-skills.git /tmp/rust-skills
cp /tmp/rust-skills/SKILL.md /tmp/rust-skills/LICENSE /tmp/rust-skills/CHANGELOG.md .agents/skills/rust-skills/
rm -rf .agents/skills/rust-skills/rules
cp -r /tmp/rust-skills/rules .agents/skills/rust-skills/
diff -r /tmp/rust-skills/rules .agents/skills/rust-skills/rules   # 无输出即一致
```

改动后同步本表的版本号与 commit。`rules/` 之外的 `checks/` 是上游自己的规则校验工具，不随技能安装。
