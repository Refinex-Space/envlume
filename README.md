# Envlume

A local-first environment gateway for AI coding agents.

Envlume 面向同时维护多个项目的开发者。每个工作区可以分别指定本机工具链、可访问的环境资源，以及允许 AI 执行的构建和测试任务。目标是让 AI 知道当前项目该用哪个 JDK、能查哪个数据库，并让开发者拿到可以复核的执行结果。

**当前可从源码启动桌面 Shell，并通过桌面命令和独立 CLI 读取本地 SQLite 项目目录。** v0.1 的项目绑定、资源访问、MCP 服务和安装包按任务卡逐步交付。[查看交付进度](docs/plan/PLAN.md)

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
- 工程约定和真实环境操作边界见 [AGENTS.md](AGENTS.md)。首张 [Phase 0 功能 Issue](https://github.com/Refinex-Space/envlume/issues/1) 跟踪 T01–T04。

### 新检出环境

在 Apple Silicon 或 Intel macOS 上安装 Xcode Command Line Tools（`xcode-select --install`）。在 Windows x64 上安装含 **Desktop development with C++** 工作负载和 Windows SDK 的 Visual Studio Build Tools，以及 Microsoft Edge WebView2 Runtime。安装本机架构的 rustup 和 Node.js 24.14.1，Windows 使用默认 MSVC 工具链。系统依赖见 [Tauri 官方开发环境说明](https://v2.tauri.app/start/prerequisites/)。

从仓库根目录运行以下命令。rustup 首次运行 Cargo 时按 `rust-toolchain.toml` 安装 Rust、clippy 和 rustfmt。首轮需要联网下载工具和依赖。

```sh
npm install --global pnpm@10.33.0
pnpm install --frozen-lockfile
cargo run --locked -p envlume -- --version
pnpm desktop dev
```

CLI 应输出 `envlume 0.1.0-dev.1`。桌面窗口启动后可展开“查看命令行版本”。开发服务器仅监听 `http://127.0.0.1:1420`。按 Ctrl+C 结束开发会话，只预览 Web 界面可用 `pnpm dev`。

### 构建与验证

以下命令适用于 macOS 终端和 Windows PowerShell。从新检出开始，先执行上面的锁定安装。

```sh
pnpm lint
pnpm typecheck
pnpm unit
pnpm build
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo tree --locked -p envlume-domain --target all
cargo build --locked -p envlume
pnpm desktop build --debug --no-bundle -- --locked
```

本机构建的可执行文件位于 `target/debug/`，名称为 `envlume`、`envlume-desktop`，Windows 加 `.exe`。桌面 build 会将生产前端嵌入开发二进制，可关闭 Vite 后运行。三目标 CI 分别核对 rustc 宿主与目标，执行相同检查并显式传入目标三元组，产物位于 `target/<target>/debug/`。[CI 工作流](.github/workflows/ci.yml)

| 原生作业 | Rust 目标 |
|---|---|
| macOS Apple Silicon | `aarch64-apple-darwin` |
| macOS Intel | `x86_64-apple-darwin` |
| Windows x64 | `x86_64-pc-windows-msvc` |

`crates/domain` 只承载领域模型，`crates/application` 供 `crates/cli` 和 `apps/desktop/src-tauri` 共用，React 界面位于 `apps/desktop/src`。主窗口的 capability 只允许调用本地只读命令 `list_projects`。CLI 的 `envlume project list --json` 返回同一份项目 ID 和名称，新库返回空列表。CI 还执行 RustSec 漏洞检查、Rust 与前端依赖许可证扫描和 Git 历史秘密扫描。[威胁与验收编号](docs/architecture/threat-tests.md) · [T03 迁移、恢复与权限验收](docs/plan/tasks/T03-sqlite-and-shared-state.md)

## 许可证

本项目采用 [MIT 许可证](LICENSE)。
