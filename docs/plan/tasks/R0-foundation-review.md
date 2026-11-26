# R0 验收骨架与契约

Phase 0 · 工程与契约　·　[返回交付总表](../PLAN.md)

| 开工条件 | 内容 |
|---|---|
| 顺序前置 | [T04 打通独立 STDIO 进程](T04-mcp-stdio-skeleton.md) |
| 架构与追踪 | [架构基线](../../architecture/ARCHITECTURE.md) §5、§16.3、§18 Phase 0、§20、§24 |

## 交付目标

复核同库双入口、独立 MCP 模拟调用和三目标编译，决定能否进入真实工作区阶段。

## 实施与产出

1. 从同一修订复跑 T01–T04。记录三个目标的构建作业、领域和契约测试、迁移恢复、桌面与 CLI 读同库及 STDIO transcript。
2. 对照初始 ADR 核查依赖方向、无 daemon 和凭据边界。把模拟响应、实测行为和要由下一阶段替换的测试替身写入评审记录。

## 验收

- **R0-A01**　在干净测试目录重复运行，桌面和 CLI 回显相同项目，MCP 通过协议返回预期模拟摘要，正常与错误路径 stdout 均可解析。
- **R0-A02**　三个必需目标均有成功编译证据，单元和契约测试可重复。任一缺失时记录阻塞项及重跑条件。

## 交接

在总表登记 R0 的通过或阻塞结论、修订和证据，给 T05 留下真实绑定接入位置。

## 执行记录

### 2026-11-26 · 通过

**R0-A01、R0-A02 均通过，可进入 T05。** 评审修订为 `ec5d057ed2c1b9aa1ede2e9dca1e28967fe9a155`。以下三个作业从同一修订检出，分别核对 rustc 宿主三元组，并成功编译独立 CLI 与嵌入前端的 Tauri 桌面程序。[CI 全部四项作业](https://github.com/Refinex-Space/envlume/actions/runs/45810265070)

| 原生 runner | 已核对的目标 | 可复查的证据 |
|---|---|---|
| macOS 15 Apple Silicon | `aarch64-apple-darwin` | [构建、同库读数和 STDIO](https://github.com/Refinex-Space/envlume/actions/runs/45810265070/job/139875956314) |
| macOS 15 Intel | `x86_64-apple-darwin` | [构建、同库读数和 STDIO](https://github.com/Refinex-Space/envlume/actions/runs/45810265070/job/139875956380) |
| Windows Server 2025 x64 | `x86_64-pc-windows-msvc` | [构建、同库读数、STDIO 和用户 DACL](https://github.com/Refinex-Space/envlume/actions/runs/45810265070/job/139875956345) |

**R0-A01 · 同库与模拟调用。** 三台 runner 均在空白用户配置中还原 v1 SQLite 夹具。独立 `envlume project list --json` 和真实 WebView 调用 `list_projects` 返回相同的 `schemaVersion: 1`、项目名 `orders` 和 ID `11111111-1111-4111-8111-111111111111`。MCP 则使用独立测试子进程、临时绑定注册表和 application 替身；`project_context` 返回 `orders / orders-main / dev / project.inspect`。两种协议流程均通过，额外身份字段被拒绝，在同一会话撤权后工具消失、直接调用被拒绝，关闭 stdin 后进程退出。[双入口验收程序](../../../apps/desktop/src-tauri/examples/shared_catalogue.rs) · [STDIO 客户端与实录](../../../tests/contracts/stdio/README.md)

**R0-A02 · 可重复性。** 本机 macOS 26.5.2 arm64 以干净检出、Rust 1.99.1、Node 24.14.1、pnpm 10.33.0 复验。锁定安装、lint、typecheck、前端 unit/build、Rust fmt、clippy、workspace build 均通过。连续两轮 `cargo test --workspace --locked` 通过，均覆盖领域生命周期、九项 application 契约、CLI 进程、STDIO 及十一项存储测试。每轮新建临时数据库，两份协议实录各含 7 次进程启动、20 个输入帧、18 个响应帧；仅归一化请求 UUID、UTC 时间和耗时后，逐条与已归档实录相符。原生桌面样例在本机的两个新临时目录中也返回相同的项目 ID。[协议实录](../../../tests/contracts/stdio/transcript.jsonl) · [迁移、并发与恢复用例](../../../crates/storage/src/tests.rs)

**本机复验异常。** 首轮在 `server/discover` 达到 15 秒超时，另一次诊断复跑使用了系统 Node 22，也出现相同超时。根因尚未确认。随后在锁定的 Node 24.14.1 下，原提交、原断言和原超时值连续两轮通过；三台原生 CI 均通过该场景。

### 架构边界与交接

- **依赖方向（ADR-0001、0002）。** Cargo 依赖图确认 domain 未引入 Tauri、SQLite 或 rmcp，application 不依赖表现层。桌面和 CLI 共用 application 的项目读取用例，桌面主窗口仅获 `allow-list-projects`。[共用用例](../../../crates/application/src/projects.rs) · [主窗口权限](../../../apps/desktop/src-tauri/capabilities/main.json)
- **绑定与进程（ADR-0003、0006）。** 活跃 MCP 测试进程未检出网络监听或子进程，诊断只进入 stderr。正式 CLI 的 resolver 默认返回 `PROJECT_NOT_TRUSTED`。T05 从此处接入真实工作区注册、规范路径核验和撤销，再用已有契约复验。[生产入口](../../../crates/cli/src/main.rs) · [T05 任务卡](T05-workspace-trust.md)
- **数据与凭据（ADR-0005）。** 迁移中断回滚、较新 schema 拒写和备份恢复均通过；备份保留已提交到 WAL 的凭据引用。数据库、WAL、备份及迁移诊断通过合成密码标记检查。系统凭据 provider 的原生验收归 T10。[凭据边界决议](../../architecture/adr/ADR-0005.md)
- **供应链。** 依赖许可证及 Git 历史秘密扫描通过。`cargo audit` 成功退出，保留T01–T04 已登记的三条传递依赖通告，其中 `chrono` 进入 rmcp 的 CLI 依赖链。[安全作业](https://github.com/Refinex-Space/envlume/actions/runs/45810265070/job/139875955836)

在干净检出中可用 `pnpm install --frozen-lockfile`、`pnpm unit`、`pnpm build`、`cargo test --workspace --locked` 和 `cargo run --locked -p envlume-desktop --example shared_catalogue --features tauri/custom-protocol` 复跑主要用例。STDIO 测试可通过 `ENVLUME_TEST_TRANSCRIPT` 把每轮记录写入新的绝对路径。完整平台命令见 [CI 工作流](../../../.github/workflows/ci.yml)。
