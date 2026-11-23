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

待执行。收尾时登记修订、平台/版本、实际命令或操作、对应验收编号、结果与脱敏证据位置。
