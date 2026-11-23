# Envlume

A local-first environment gateway for AI coding agents.

Envlume 面向同时维护多个项目的开发者。每个工作区可以分别指定本机工具链、可访问的环境资源，以及允许 AI 执行的构建和测试任务。目标是让 AI 知道当前项目该用哪个 JDK、能查哪个数据库，并让开发者拿到可以复核的执行结果。

**架构基线和交付计划已就绪，v0.1 待开发。** 桌面安装包、CLI 和 MCP 服务将在后续任务中交付。[查看交付进度](docs/plan/PLAN.md)

## v0.1 的使用场景

假设你在同一台机器上维护两个服务，一个使用 JDK 8，另一个使用 JDK 17。下面是首版需要跑通的流程。

1. 在桌面端导入 Java 8 服务的 Git 工作目录，确认信任，选择本机 JDK 8 和 Maven，启用 `maven-test` 任务。JDK 选择只对该任务的子进程生效，系统 `JAVA_HOME` 和另一个服务的配置保持独立。
2. 给该工作区绑定 DEV MySQL，选择只读账号并授权读取指定表。密码保存在系统凭据存储中。即使 TEST 也有一个名为 `primary-db` 的资源，两个环境的连接和授权仍各自独立。
3. 将 Claude Code、Codex 或 Cursor 接入当前项目。客户端通过模型上下文协议（MCP）启动本地 `envlume mcp` 进程，先用 `project_context` 核对工作区，再用 `database_schema` 查看获准的表。关闭桌面窗口后，客户端仍可调用网关。
4. Agent 通过 `task_run` 执行已启用的 Maven 测试。开发者用返回的 `evidenceId` 查到实际工具版本、退出码、耗时和脱敏日志，核对测试是否通过。

## 运行方式与权限

| 入口 | v0.1 设计 |
|---|---|
| 桌面应用 | 使用 Tauri 2、React 和 TypeScript，负责项目导入、工具链选择、资源授权、客户端接入和执行记录查看。 |
| 独立 CLI | 供开发者和脚本诊断本机环境、管理项目、运行具名任务。 |
| MCP 服务 | 由 AI 客户端按工作区启动，通过标准输入输出（STDIO）通信，无需常驻后台服务。 |

三个入口共用 Rust 应用层。每次调用都会重新检查工作区绑定、目标资源与授权，撤销授权后，已经启动的 MCP 进程也会在下一次调用时被拒绝。SQLite 保存项目配置、凭据引用和脱敏证据，密码由 macOS Keychain 或 Windows Credential Manager 保管。

首版对 MySQL 开放连接检测、元数据读取和受限的单条 `SELECT`，生产目标默认禁用。构建任务会执行仓库自身的代码，因此需要先信任工作区并确认首次执行。权限控制覆盖经 Envlume 发起的操作，AI 客户端自带的终端仍由宿主管理。

## 交付路线

| 版本 | 计划范围 |
|---|---|
| v0.1 MVP | 项目绑定、JDK/Maven/Node/Python 检测、Maven/Node 具名任务、MySQL 只读访问、三个 AI 客户端接入。发行目标为 macOS Apple Silicon 与 Intel 的 DMG、Windows 11 x64 的 NSIS 安装包，以及对应的独立 CLI。 |
| v0.2 | 增加 Redis、Elasticsearch 只读访问、SSH 日志任务及 Gradle 执行。受限写操作的一次性审批按实际需求决定。 |
| v1.0 及以后 | 支持非敏感项目模板与策略的导入导出，按需求扩展企业凭据、连接器和隔离执行。 |

## 参与开发

- 了解模块边界、权限模型和发布标准，阅读[系统架构](docs/architecture/ARCHITECTURE.md)。
- 查找当前任务、前置依赖和验收要求，从[交付计划](docs/plan/PLAN.md)进入对应任务卡。
- 从零搭建工程，从 [T01 工作区与三目标构建](docs/plan/tasks/T01-workspace-ci.md) 开始。该任务交付首批可运行的构建和测试命令。

## 许可证

本项目采用 [MIT 许可证](LICENSE)。
