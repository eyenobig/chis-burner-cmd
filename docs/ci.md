# CI / 构建（GitHub Actions）

仓库用一条 GitHub Actions 工作流 [.github/workflows/build.yml](../.github/workflows/build.yml) 在三个平台上跨平台编译 `cfb`，并产出 **Tauri sidecar 命名**的二进制，可直接丢进 [beggar_chis](../../beggar_chis)/`src-tauri/binaries/`。

## 触发条件（双重门禁）

构建**只在同时满足两个条件**时才会跑：

| 门禁 | 位置 | 含义 |
|------|------|------|
| **门禁 A：必须有 tag** | 触发器 `on.push.tags: ['v*']` | 只在推送 `v*` 标签时触发。普通 push、PR、其它分支一律不构建。 |
| **门禁 B：tag 必须在 master 上** | `gate` 作业 | 用 `git merge-base --is-ancestor` 校验 tag 指向的提交是 `master` 的祖先；不在 master 上直接拦下，连构建都不跑。 |

> 固定分支由 `gate` 作业里的 `env: RELEASE_BRANCH`（默认 `master`）一处控制，想换分支改这里即可。

## 构建矩阵

每个目标在匹配的**原生 OS runner** 上构建（非交叉编译，避开工具链坑）：

| 目标 triple | runner | 产物名（sidecar） |
|------|------|------|
| `x86_64-pc-windows-msvc` | windows-latest | `cfb-x86_64-pc-windows-msvc.exe` |
| `x86_64-unknown-linux-gnu` | ubuntu-latest | `cfb-x86_64-unknown-linux-gnu` |
| `x86_64-apple-darwin` | macos-15-intel | `cfb-x86_64-apple-darwin` |
| `aarch64-apple-darwin` | macos-14 | `cfb-aarch64-apple-darwin` |

> Linux 额外装 `libudev-dev`（`serialport` 依赖）；macOS/Windows 走系统原生 API，无需额外包。
> 精简平台：删掉 matrix `include` 里不要的条目即可。
>
> **关于 Intel macOS runner：** GitHub 已于 2025-12 下线 `macos-13`（最后的 Intel 镜像），
> Intel 构建改用官方替代镜像 `macos-15-intel`。若该镜像将来也被弃用，Intel Apple 可改用交叉编译
> （在 `macos-14` ARM runner 上 `cargo build --target x86_64-apple-darwin`），ARM 端已自带双架构。

流水线：`gate → build ×4 → release`。每个目标会跑一次 `cfb help` 做启动冒烟（不碰串口硬件），通过后重命名为 sidecar 名上传。`release` 把 4 个二进制打成 GitHub Release 资产，**正文来自 CHANGELOG.md**（见下节）。

> **容错：** `release` 用 `if: !cancelled()`，单平台 build 偶发失败不阻塞发版——已成功的平台照常发布。
> 兜底是 flatten 步骤：至少要有一个 `cfb-*` 二进制，否则 release 作业报错退出（不会发空 Release）。
> 这是给 `beggar_chis` 的「win/mac/linux 至少三主平台」稳定供货的保险。

## CHANGELOG / 版本记录

[CHANGELOG.md](../CHANGELOG.md) 是 **GitHub Release 正文的唯一来源**，纯手写维护（不自动生成）。发版 CI 会按 tag 号从 CHANGELOG 抽取对应段落注入 Release 正文。

约定：

- 标题格式**必须**是 `## [vX.Y.Z] - YYYY-MM-DD`（带方括号版本号、ISO 日期），版本号要和 git tag 完全一致（含 `v` 前缀）。CI 用 `awk` 按 `## [vX.Y.Z]`（或 `## vX.Y.Z`）定位段落，抽到下一个 `## ` 二级标题前为止。
- 日常改动先记在 `## [Unreleased]` 下，按 `新增` / `变更` / `修复` 分组。
- 新版本在**最上面**，`## [Unreleased]` 永远是第一条。

**抽不到不阻断发版**：若 CHANGELOG 里没有当前 tag 的段落（漏写 / 格式不符），release 作业会退回 GitHub 自动生成的提交记录作为正文，并在日志里告警——发版本身不会因文案缺失而失败。

## 完整发版流程（git-flow）

> tag 触发用的是**被 tag 那次提交里的 workflow 文件**，所以必须先把本文件合到 master，再在 master 上打 tag——顺序不能反。

```bash
cd z:/Project/chis-burner-cmd

# ① 在 dev 上改代码 / workflow
git checkout dev
git add -A
git commit -m "<中文一句话>"
git push origin dev

# ② 合 dev → master（只接受快进）并推
git checkout master
git merge --ff-only dev
git push origin master

# ③ 把 CHANGELOG.md 的 `## [Unreleased]` 改成 `## [vX.Y.Z] - YYYY-MM-DD`，
#    并在文件最上面新开一个空的 `## [Unreleased]`，提交并推到 master。
#    （Release 正文来自 CHANGELOG 这一段，见上一节。）
# ④ 在 master 上打 tag 并推 tag —— 这一步才触发构建
git tag v0.1.0
git push origin v0.1.0
```

## 拿到产物

- **稳定下载 URL**（给 beggar_chis 用最方便）：
  `https://github.com/eyenobig/chis-burner-cmd/releases/download/v0.1.0/cfb-x86_64-pc-windows-msvc.exe`
  （换 triple / 换 tag 号即对应其它平台/版本）
- 或 GitHub **Releases** 页手动下载，或 Actions 运行页 **Artifacts** 区按目标下载 zip。

放进 [beggar_chis](../../beggar_chis)/`src-tauri/binaries/`，按当前构建目标挑文件名（Tauri 打包时按 triple 自动选）。

## 不需要配 secret

Release 用自动 `GITHUB_TOKEN`，`permissions: contents: write` 已写在 workflow 里，零额外配置。

## 故障排查

- **Actions 没跑**：`Settings → Actions → General` 确认是 "Allow all actions"；确认 tag 是 `v*` 开头、且 push 了 tag（不是只 push 分支）。
- **`gate` 红了 "tag 不在 master"**：tag 打在了 dev-only 的提交上。要么先把 master 快进到该提交，要么删掉 tag 在 master 上重打：`git tag -d v0.1.0 && git push origin :refs/tags/v0.1.0`。
- **想重新发同一版本**：删掉 GitHub 上的 Release，删本地 + 远端 tag，再重打重推。
- **`cargo build --locked` 失败**：`Cargo.lock` 与 `Cargo.toml` 不一致，本地跑一次 `cargo build` 更新 lock 并提交，再推。
