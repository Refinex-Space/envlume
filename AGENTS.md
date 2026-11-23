# Envlume 工程入口

- 用简体中文沟通，保留命令、日志和标识符原文。提交信息遵循 Conventional Commits。
- 从[交付计划与当前检查点](docs/plan/PLAN.md)选择任务。先读任务卡和[架构基线](docs/architecture/ARCHITECTURE.md) §1、§4、§5、§8、§9、§14、§18、§19。运行形态、权限、凭据、绑定和接口兼容性变更遵循 §20 的 ADR 流程。
- 领域层保持纯 Rust。桌面、CLI 和 MCP 共用 application 层；WebView 的权限按窗口最小化。接触真实环境前，先用 `project_context` 核对项目、绑定和授权目标。该工具尚未接入或核对失败时，停在本地隔离测试环境。
- 在根目录 `pnpm install --frozen-lockfile`，执行 `pnpm lint`、`pnpm typecheck`、`pnpm unit`、`pnpm build`，再运行 `cargo fmt --all --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings`、`cargo test --workspace --locked`。各平台准备和启动命令见 [README](README.md#参与开发)。
- 修 bug 先写能复现的失败测试。用户可见的前端改动用 ego-browser 在本地操作，只使用测试账号。任务卡按验收编号记录命令、平台、结果和剩余项；[威胁对照](docs/architecture/threat-tests.md)用于选择负面用例。
