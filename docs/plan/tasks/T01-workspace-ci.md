# T01 建立工作区和三目标构建

Phase 0 · 工程与契约　·　[返回交付总表](../PLAN.md)

| 开工条件 | 内容 |
|---|---|
| 顺序前置 | 起点，可直接开始 |
| 架构与追踪 | [架构基线](../../architecture/ARCHITECTURE.md) §5、§13.4、§16.1–16.3、§18 Phase 0、§20–24 |
| 工作范围 | 仓库根目录、Cargo/pnpm workspace、`apps/desktop/`、`crates/cli/`、CI 与架构决策记录 |

## 交付目标

在现有文档仓库中建立可启动的桌面 Shell 和独立 CLI，让两个入口在三个目标架构上可复现构建。T04 在 CLI 中接入 STDIO。

## 实施与产出

1. 按 §24 把首张功能 Issue 的标题定为 `Phase 0: Domain Model + Workspace Binding + MCP STDIO Skeleton`，范围由 T01–T04 共同兑现。沿用已有 README、LICENSE 和架构文件，补上根 `AGENTS.md`、ADR 模板及首批决策记录，按 §14 建立威胁与测试编号对照。工程入口引用架构，并约定对真实环境操作前先用 `project_context` 核对项目。
2. 建立 Cargo workspace、domain/application 最小 crate、独立 `envlume` CLI 及 pnpm workspace 中的 Tauri 2 + React/TS/Vite Shell。实现版本输出与最小窗口，WebView 采用受限 capability 和 CSP。领域类型与用例契约由 T02 补齐。
3. 锁定依赖和工具要求，给出 macOS / Windows 可复现的开发命令。CI 执行 `cargo fmt --check`、`cargo clippy`、`cargo test` 和 `pnpm install --frozen-lockfile`，再运行仓库定义的 lint、typecheck、unit、build。接入 `cargo audit`、许可证与秘密扫描，明确原生构建作业各自的目标架构。

## 验收

- **T01-A01**　新检出环境按 README 构建，CLI 报告版本，桌面开发构建能启动。macOS arm64、macOS x64、Windows x64 均有编译结果。
- **T01-A02**　检查 Cargo 依赖图和前端权限清单，domain 的构建不引入 Tauri、SQLite、MCP。锁文件一致性和一个确实执行的测试出现在 CI 日志中。
- **T01-A03**　根工程入口引用架构、交付顺序和测试命令。ADR-0001 至 ADR-0010 可从架构登记表定位，新增决策模板具有 §20 的全部字段。

## 交接

移交工作区结构、构建命令和三目标 CI 运行记录。记录仓库现有 MIT 许可，发布前由 T21 复核依赖及随包声明。

## 执行记录

### 2026-11-23 · Phase 0 工程基线

首张功能 [Issue #1](https://github.com/Refinex-Space/envlume/issues/1) 跟踪 T01–T04。T01-A01 至 A03 已通过。实现修订为 `0e0a432`，三目标和安全作业使用同一对锁文件并全部通过。干净检出时观察到 Tauri CLI 会补写 Cargo 清单中的空 features 数组，交接修订已收录其标准形式，并将这份清单纳入 CI 的构建后差异检查。[CI 运行记录](https://github.com/Refinex-Space/envlume/actions/runs/45200081587)

| 验收 | 操作、预期与证据 |
|---|---|
| T01-A01 · CLI 与桌面 | 本机 macOS 26.5.2 arm64，Rust 1.99.1、Node 24.14.1、pnpm 10.33.0。从干净检出（目录名含空格）按 [README](../../../README.md#参与开发) 安装和构建，产出两个 arm64 Mach-O 可执行文件，`cargo run --locked -p envlume -- --version` 输出 `envlume 0.1.0-dev.1`。`pnpm desktop dev` 启动同版本标题的原生窗口，系统辅助功能树可读到“本地开发环境网关”。用 ego-browser 在本地 1420 端口展开“查看命令行版本”，出现 `envlume --version`。关闭开发服务，再运行嵌入前端的桌面二进制，原生窗口中仍能展开相同命令。全程无需账号。 |
| T01-A01 · 三目标编译 | 原生作业核对 `rustc -vV` 的 host，运行 `cargo build --locked -p envlume`、CLI 版本命令及 `pnpm desktop build --debug --no-bundle --target <target> -- --locked`。日志保留实际目标、版本、测试数、二进制路径与退出状态。[Apple Silicon](https://github.com/Refinex-Space/envlume/actions/runs/45200081587/job/137780925224) · [Intel macOS](https://github.com/Refinex-Space/envlume/actions/runs/45200081587/job/137780925773) · [Windows x64 MSVC](https://github.com/Refinex-Space/envlume/actions/runs/45200081587/job/137780925189) |
| T01-A02 · 隔离与质量门 | `cargo tree --locked -p envlume-domain --target all` 只有领域 crate 自身，CLI 图为 envlume → application → domain。Tauri 生成的 capability 清单只有本地 `main` 窗口、空权限列表，生产 CSP 以 `default-src 'none'` 开始。CI 运行锁定安装、Biome lint、TypeScript 检查、1 项 React 单测、Vite build、Rust fmt/clippy/test；2 项 CLI 进程测试验证版本 stdout 以及多余参数的退出码和 stderr。两份锁文件在构建后无差异。 |
| T01-A03 · 工程入口 | [AGENTS.md](../../../AGENTS.md) 与 README 已链接架构、计划、测试命令；核对 §20 的十个 ADR 链接均可打开，模板包含背景、备选、理由、风险、安全影响、迁移和证据。[架构登记表](../../architecture/ARCHITECTURE.md#20-关键架构决策adr-初始登记) · [模板](../../architecture/adr/TEMPLATE.md) · [威胁与测试编号](../../architecture/threat-tests.md) |

**供应链与后续验收**　仓库采用 MIT。Rust 许可证、各原生目标前端依赖许可证、Gitleaks Git 历史扫描均通过。`cargo audit` 退出码为 0，报告 `chrono`、`proc-macro-error` 的维护状态和 `glib` 的 unsound 警告（RUSTSEC-2026-0543、RUSTSEC-2024-0370、RUSTSEC-2024-0429）。针对三个目标运行 `cargo tree --locked -p envlume-desktop --target <target>` 均未列出这三个 crate。T21 仍需按届时锁文件和发行内容复核许可证、随包声明及通告。Intel 与 Windows 本轮取证覆盖原生编译和 CLI 运行，安装与原生桌面交互按 V02、V03 验收。[安全作业](https://github.com/Refinex-Space/envlume/actions/runs/45200081587/job/137780924895)
