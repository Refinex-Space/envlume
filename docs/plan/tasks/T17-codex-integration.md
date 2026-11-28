# T17 交付 Codex 可信项目接入

Phase 3 · v0.1 MVP　·　[返回交付总表](../PLAN.md)

| 开工条件 | 内容 |
|---|---|
| 顺序前置 | [T16 交付 Claude Code 项目接入](T16-claude-code-integration.md) |
| 架构与追踪 | [架构基线](../../architecture/ARCHITECTURE.md) §8.1、§11.2–11.5、§12、§19.1 · US-08 · E2E-09、SEC-011 |
| 工作范围 | Codex 适配器、TOML 配置夹具、真实客户端验收 |

## 交付目标

让 Codex 在受信任的项目作用域加载 Envlume，并能区分配置存在和工具已可调用。

## 实施与产出

1. 按实测版本核实 .codex/config.toml 的可信项目层和生效条件。用户级写入只走单独明确确认的路径。首版项目接入仍须通过项目作用域验收。
2. 接入 detect、差异预览、TOML 合并、验证及卸载。核对 Windows 反斜杠、带空格 CLI 路径、已有 MCP 表和无关配置项。

## 验收

- **T17-A01**　E2E-09 在真实 Codex 记录项目的信任状态、版本、协商结果与 project_context 对应的绑定。对未加载项目配置的情形给出可操作说明。
- **T17-A02**　在已有第三方配置的测试环境重复安装、拒绝更改、验证失败、撤销绑定和卸载，核对文件和调用结果。不会静默升级配置作用域。

## 交接

移交 Codex 项目注册、拒绝及回滚记录，标示测试配置位置与实际作用域的脱敏摘要。

## 执行记录

2026-11-28，本机 macOS 26.5.2 arm64，真实 Codex CLI 0.171.0-alpha.13.2。使用独立用户目录、临时 Git 项目和打包安装的 Envlume CLI。通过 app-server 的 STDIO 接口执行下列检查，模型轮次为零。[验收脚本](../../../tests/integrations/codex.py) · [脱敏收据](../evidence/T17-codex.json)

| 验收 | 操作及结果 |
|---|---|
| T17-A01 / E2E-09 | 核对可信项目层的真实目录、启用状态、完整启动参数及来源。临时 Codex 线程发现并调用 `project_context`；客户端与 Envlume 协商 MCP `2025-06-18`，requestId 对应正确 bindingId、LOCAL_STDIO、SUCCESS。显式未信任时，即使用户层提供完全相同的服务，也拒绝项目证明并提示核对信任。 |
| T17-A02 / SEC-011 | 已有第三方 MCP、合成私有设置、Windows 反斜杠和含空格路径。覆盖取消确认、安装失败回滚、重复安装、修改信任后重新验证、正常卸载，以及用户配置路径与安装目标重合时拒绝预览。卸载精确恢复原字节。日常配置核验期间第三方服务的启动标记始终缺失；撤销绑定后，同一 Codex 连接收到 BINDING_REVOKED。 |

**复跑。** 在 macOS 或一次性 Windows 测试用户内构建开发包。设置 `T17_CODEX_BIN` 为原生 Codex 可执行文件的绝对路径，可将 `T17_TEST_SNAPSHOT` 指向新的收据文件，运行 `node tests/installation/stable.mjs <桌面产物>`。Windows 另需设置 `T14_DISPOSABLE_PROFILE=1`。CI 固定使用官方 npm 分发的 Codex 0.172.0。[构建与 CI](../../../.github/workflows/ci.yml)
