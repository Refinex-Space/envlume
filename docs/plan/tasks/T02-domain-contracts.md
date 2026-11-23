# T02 冻结领域模型与入口契约

Phase 0 · 工程与契约　·　[返回交付总表](../PLAN.md)

| 开工条件 | 内容 |
|---|---|
| 顺序前置 | [T01 建立工作区和三目标构建](T01-workspace-ci.md) |
| 架构与追踪 | [架构基线](../../architecture/ARCHITECTURE.md) §1.3、§4、§5.1–5.2、§8、§9、§10.4、§12、§15 · US-01、US-09 |
| 工作范围 | `crates/domain/`、`crates/application/`、`tests/contracts/` |

## 交付目标

让三个入口使用同一组实体、授权上下文和结果类型，先固定可验证的输入输出。

## 实施与产出

1. 在 domain 定义 Project、WorkspaceBinding、ToolchainProfile、EnvironmentTarget、ResourceBinding、TaskDefinition、CapabilityGrant、ExecutionRecord、ClientIntegration，区分稳定 ID、逻辑别名和路径，明确生命周期及引用关系。
2. 在 application 定义按可信来源建立的 RequestContext、逐请求授权点、类型化用例和 repository/secret/connector/toolchain 接口。使用内存替身先验证拒绝路径，冻结每个操作对应的 capability。
3. 在 tests/contracts 固定 §8.2 八项工具的输入、返回与权限，归档 CLI 人类/JSON/退出码和统一结果样例。日期用 UTC，错误有分类。协议变更同时更新测试、迁移说明和消费者文档。

## 验收

- **T02-A01**　两个工作区可属于同一项目，不同目标可有同名资源。领域测试证明切换环境不改变 ToolchainProfile，项目软删除能表达绑定及授权撤销。
- **T02-A02**　用表驱动用例核对八项工具的必填字段、长度、枚举和 `additionalProperties: false`。多余身份字段、自由路径、原始连接地址被拒绝。此阶段可用契约 fixture 执行。
- **T02-A03**　CLI 结果含 `schemaVersion`，受控操作含 `requestId`，执行结果可关联 `evidenceId`。错误样例覆盖输入、拒绝、缺依赖、连接、超时、内部失败。

## 交接

交付领域类型、接口和可执行契约样例。T03 实现持久化，T04 先接一项模拟工具，T13 接齐八项。

## 执行记录

### 2026-11-24 · T02-A01 至 A03 通过

实现修订 `8d65bb0`，基于 T01 的 `25b91ab`。原生实测环境为 macOS 26.5.2 arm64、Rust 1.99.1、Node 24.14.1、pnpm 10.33.0。所有输入均为仓库内的合成数据和内存注册表。[契约说明与迁移约定](../../../tests/contracts/README.md)

| 验收 | 实际操作与结果 |
|---|---|
| T02-A01 | `cargo test --locked -p envlume-domain -p envlume-application` 通过。两个同项目 worktree 保留独立绑定；项目软删除同时撤销两个 binding 与 grant，重复删除保留首次时间，另一项目保持有效。DEV/TEST 各自的 `primary-db` 仅在授予对应资源 UUID 后可访问；切换后 profile ID 和工具规则相同。[生命周期测试](../../../crates/domain/tests/lifecycle.rs) · [应用层契约测试](../../../tests/contracts/contracts.rs) |
| T02-A02 | `node --test tests/contracts/schemas.test.mjs` 9 项通过。对八项工具逐一验证输入、结果与封闭字段集合。Rust 同步覆盖别名、表名、长度边界、错误类型与恶意附加参数；逐项移除所需 grant，均在进入用例前拒绝。在同一会话撤销绑定或 grant、令授权到期、替换绑定项目、调整资源风险和停用任务，后续调用均拒绝。[输入及结果 Schema](../../../tests/contracts/tools.json) · [调用样例](../../../tests/contracts/examples.json) |
| T02-A03 | 通过真实 application 调度和内存用例生成成功及六类失败报告，与 CLI 人类文本、JSON、退出码 0/2/3/4/5/6/7 逐项比对。每次调用生成独立 UUID，请求时间可解析为 UTC；执行用例的成功结果携带执行器返回的证据编号。另运行独立 CLI 进程测试，`cargo run --locked -p envlume -- --version` 输出 `envlume 0.1.0-dev.1`。[公共响应 Schema](../../../tests/contracts/report.schema.json) · [CLI 快照](../../../tests/contracts/cli.json) |

**工作区验证。** `cargo fmt --all --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings`、`cargo test --workspace --locked` 和 `cargo build --workspace --locked` 全部通过。Rust 实际执行 12 项测试，其中 9 项应用契约、1 项领域生命周期、2 项 CLI 进程。根目录 `pnpm install --frozen-lockfile`、`pnpm lint`、`pnpm typecheck`、`pnpm unit`、`pnpm build` 全部通过，unit 包含 1 项 React 单测和 9 项契约校验。

**依赖与平台。** `cargo deny --locked check licenses` 和 `pnpm licenses list --json | node scripts/check-js-licenses.mjs` 通过。仅在 Node 开发依赖中引入 Ajv 来执行 JSON Schema。Rust 复用现有锁文件中的 UUID、时间和序列化库。`cargo audit` 退出 0，保留 T01 已登记的三条通告。`cargo check --locked --tests -p envlume-domain -p envlume-application --target x86_64-apple-darwin --target x86_64-pc-windows-msvc` 通过；本轮在这两个平台做了交叉类型检查，未做原生运行。

**交接。** T03 实现 SQLite 仓储、一致快照与双入口读用例；T04 使用冻结的 `project_context` 样例验证真实 STDIO；T13 将八项工具接到实际用例并从安装版复核 CLI 输出。
