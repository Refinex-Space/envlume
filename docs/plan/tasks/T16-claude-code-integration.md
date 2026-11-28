# T16 交付 Claude Code 项目接入

Phase 3 · v0.1 MVP　·　[返回交付总表](../PLAN.md)

| 开工条件 | 内容 |
|---|---|
| 顺序前置 | [T15 实现配置预览、合并与回滚](T15-integration-lifecycle.md) |
| 架构与追踪 | [架构基线](../../architecture/ARCHITECTURE.md) §8.1、§11.2–11.5、§12、§19.1 · US-08 · E2E-08、SEC-011 |
| 工作范围 | Claude Code 适配器、配置夹具、真实客户端验收 |

## 交付目标

在真实 Claude Code 中完成项目本地接入，用 project_context 验证绑定，再安全卸载。

## 实施与产出

1. 根据测试客户端版本核实本项目的 local scope、配置归属、信任和重载方式，优先项目本地登记。仓库 .mcp.json 共享模式需显式确认，避免把个人绝对路径提交到版本库。
2. 将五步生命周期接上 T15。展示实际作用域和修改差异。用独立测试项目与测试配置验证工具发现和绑定。客户端版本或作用域能力不满足时给出诊断。

## 验收

- **T16-A01**　E2E-08 在真实 Claude Code 记录版本、OS、配置作用域、协商结果、工具调用与卸载恢复，另一个项目不会获得此绑定。
- **T16-A02**　预置其他 MCP 服务后重复安装。触发验证失败和正常卸载，核对第三方条目、备份和原有配置。用隔离测试配置验证无凭据写入。

## 交接

移交真实客户端兼容记录及可复跑的 Claude Code 操作步骤，夹具测试和实际 POC 分别标注。

## 执行记录

真实客户端为 Claude Code 2.1.318，使用原生可执行文件、Envlume 的已安装 CLI 和独立的 Claude 配置目录。桌面退出后，Claude Code 在新的客户端进程中重新加载项目配置。测试仅向本机回环 Messages 夹具发出请求，夹具按固定步骤选择一个已发现的工具。真实客户端执行工具，Envlume 通过 STDIO 返回绑定数据。全程使用合成凭据，macOS 额外将对话进程的出站 TCP 限定到夹具端口。

| 验收 | 操作及证据 |
|---|---|
| T16-A01 / E2E-08 | 默认预览显示 local、项目键及稳定 CLI 路径。客户端报告连接状态、工具能力、协商版本；真实客户端执行 `mcp__envlume__project_context`，用 requestId 从本地审计核对 binding、LOCAL_STDIO、SUCCESS。兄弟项目查不到服务，卸载后原项目也查不到。客户端将 Git 子目录归到上层仓库时，预览报告 PROJECT_SCOPE_MISMATCH。 |
| T16-A02 / SEC-011 | 用 Claude 自身预置含合成凭据的第三方服务。取消安装保持未登记；已禁用服务导致安装失败，恢复原始字节及禁用选项。成功安装两次后通过校验。在另一项目写入新字段后卸载，保留该字段、第三方服务和账号夹具。扫描两次事务的四份备份，校验内容仅为撤销位置及长度或本服务 command/args。 |
| 格式夹具 | `cargo test --locked -p envlume-clients` 覆盖缺失层级、重复键、超长数字、UTF-8 偏移、并发、发布中断和外部编辑。 |

**复跑。** 先执行 `pnpm desktop build --debug --no-bundle --target <target> -- --locked`，再设置 `T16_CLAUDE_BIN` 为 Claude Code 2.1.318 的原生可执行文件绝对路径，运行 `node tests/installation/stable.mjs <桌面产物>`。可选的 `T16_TEST_SNAPSHOT` 指向尚不存在的收据文件。macOS 使用临时用户目录；Windows 在一次性测试用户中设置 `T14_DISPOSABLE_PROFILE=1`。[真实客户端脚本](../../../tests/integrations/claude_code.py) · [回环夹具](../../../tests/integrations/claude_model.py) · [原生 CI 作业](../../../.github/workflows/ci.yml)

本机 macOS 26.5.2 arm64 的实际协商版本为 `2026-07-28`。四份私有备份扫描通过，第二项目不可见，禁用回滚与正常卸载通过。[本机脱敏收据](../evidence/T16-claude-code.json)
