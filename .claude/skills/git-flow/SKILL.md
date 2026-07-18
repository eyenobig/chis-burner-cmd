---
name: git-flow
description: chis-burner-cmd 的 git 提交工作流（dev/master 双分支）。任何在本仓库提交、同步、合并、推送代码前必读——约定了分支角色、rebase 节奏、合并方向和 commit 风格，保证历史线性、master 永远稳定。
---

# git 提交工作流（dev / master）

本仓库（`chis-burner-cmd`）用最简双分支模型。**两个分支，一个方向。**

```
        ┌──────── rebase（master → dev，定期同步）────────┐
        ▼                                                │
   ┌─────────┐  merge --ff-only   ┌─────┐
   │  dev    │ ─────────────────▶ │ master │ ──▶ origin/master
   │ (开发)  │                    │ (主干) │
   └─────────┘                    └─────┘
```

## 分支角色

| 分支 | 角色 | 规则 |
|------|------|------|
| **master** | 主干 / 稳定基准 | 永远可发布。**禁止直接 commit**，只接受 dev 合并。远程跟踪 `origin/master`。 |
| **dev** | 日常开发 | 所有功能改动在这里 commit。基于 master，定期把 master 变基进来。 |

> “master 分支是每次需要变基处理的分支” = dev 定期 `rebase origin/master`，把主干新内容线性叠加到 dev 上。**不是**在 master 上 rebase。

## 标准动作

### 1. 日常开发（在 dev 上 commit）

```bash
git checkout dev
# 改代码…
git add -A
git commit -m "<动词开头的一句话，中文>"
```

- commit message 用中文（沿用仓库既有风格，如 `新增 …` / `修复 …` / `重构 …`）。
- 不要在 master 上直接 commit。

### 2. 同步主干（dev ← master，定期做）

开始新一天的工作前、或 master 有新提交时：

```bash
git checkout dev
git fetch origin
git rebase origin/master          # 把 dev 的提交线性叠到最新 master 之上
```

- 有冲突：解决后 `git add` + `git rebase --continue`，**不要**用 merge commit 绕过。
- rebase 后 dev 已推过的话需 `git push --force-with-lease origin dev`（仅 dev 允许 force-with-lease；**master 永远不 force**）。

### 3. 合并 dev → master（功能成熟时）

```bash
git checkout master
git merge --ff-only dev           # 只接受快进，保持线性
git push origin master
git checkout dev                  # 回到 dev 继续
```

- `--ff-only`：若不能快进说明 dev 没 rebase 到最新 master，回 dev 做「动作 2」再来。
- 合并后 dev 与 master 指向同一 commit，无需删 dev。

### 4. 推送

```bash
git push origin dev       # 开发中随时推（dev 是共享工作分支）
git push origin master    # 合并后推（master 只前进、不回退）
```

## 禁止

- ❌ 在 master 上直接 commit / 修改。
- ❌ 对 master 做 `push --force` / `reset --hard` 后推送（历史不可变）。
- ❌ 用 merge commit 把 master 并进 dev（用 rebase 保持线性）。
- ❌ 跨过 dev 直接往 master 接功能。

## 附：提交署名

commit message 结尾保留一行（harness 默认）：

```
Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>
```
