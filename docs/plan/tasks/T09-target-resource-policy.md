# T09 落实环境、资源归属与逐请求授权

Phase 2 · 环境与 MySQL　·　[返回交付总表](../PLAN.md)

| 开工条件 | 内容 |
|---|---|
| 顺序前置 | [R1 验收项目与工具链闭环](R1-toolchain-review.md) |
| 架构与追踪 | [架构基线](../../architecture/ARCHITECTURE.md) §4、§9.1–9.3、§10.3、§14.2 · US-04、US-05 · SEC-003、SEC-004 |
| 工作范围 | `crates/policy/`、application、storage、环境与资源视图 |

## 交付目标

按 binding、target、resource 和 capability 判断每次访问，包括同名资源和已撤销的授权。

## 实施与产出

1. 通过 application 管理目标和逻辑资源。预设 DEV、TEST、STAGING、PROD，支持自定义别名，单独持久化 kind 和 riskLevel。资源绑定到已获本地确认的端点，切换环境只改变远端资源上下文。
2. 实现 §9.3 矩阵，绑定授权同时匹配状态、到期时间和能力。DEV/TEST 资源操作须授权，STAGING 健康及结构须显式授权、查询默认禁用，PROD 远端访问默认禁用且不能成为隐式目标。§7.2 对生产只读另有单独批准和表/列脱敏要求，缺少任一条件时拒绝。日常 UI 和自定义策略只能收紧基线。
3. 远端操作必须显式给出目标，连接器取凭据前先授权。变更已批准端点或提升级别必须重审其关联授权，避免沿用旧端点的许可。CLI environment list 和 MCP 环境视图只列可见摘要。

## 验收

- **T09-A01**　SEC-003 跨项目、跨 binding、跨目标同别名、未授权能力、已过期或运行中撤销分别拒绝。拒绝时测试替身记录的取密钥/建连次数均为零。
- **T09-A02**　SEC-004 检查所有目标均无法授予 v0.1 基础设施写权限，PROD 改名或成为唯一目标后默认策略仍生效，显式运行参数也不能放宽策略。
- **T09-A03**　同一 binding 的授权变更在三入口下一次调用生效。软删除项目撤销绑定和 grants，切换远端目标仍使用原工具链。

## 交接

移交可执行权限矩阵，包含被允许及被拒绝的组合和拒绝原因，T10–T13 复用同一入口。

## 执行记录

2026-11-27，修订 `1a4b853`，macOS 26.5.2 arm64。验收使用临时 SQLite、原生 Tauri WebView 和真实 STDIO 子进程。取凭据、建连使用带计数器的替身。仅使用隔离目录和占位地址。[策略代码](../../../crates/policy/src/lib.rs) · [端点确认决议](../../architecture/adr/ADR-0012.md)

### 可执行权限矩阵

下表适用于同项目、已信任 binding、显式 target、已确认且启用的资源。每项“可授予”还需当前 binding、目标 UUID、资源 UUID 和能力均匹配的有效 grant。别名仅在所属项目或目标内解析。

| 操作 | DEV / TEST | STAGING | PROD / 未分类 kind |
|---|---|---|---|
| `resource.connect`、`database.schema` | 可授予 | 本机显式授予 | 拒绝 |
| `database.read` | LOW、MEDIUM 可授予；HIGH 拒绝 | 拒绝 | 拒绝 |
| 基础设施写入、SSH 执行 | 拒绝 | 拒绝 | 拒绝 |

CRITICAL 风险拒绝全部远端操作。四个预设风险依次为 LOW、MEDIUM、HIGH、CRITICAL。用户可自定义别名，改名保留 kind/risk。现版本没有同时满足独立审批与表/列脱敏的生产读路径。环境目录只显示至少具备一项有效资源能力的别名，健康状态在 T11 接入连接器前为 `unknown`。

### 验收结果

- **T09-A01 / SEC-003**　按 5 种 kind（含历史自定义 kind）、4 种 risk、3 种远端能力执行 60 个组合。跨项目、跨绑定、跨目标同名、能力不符、过期、撤销、未确认或停用均拒绝；拒绝进入用例时，取密钥和建连计数均为 0。另在分发后、取引用前撤权，仍为 0/0；在已取引用、准备建连时撤权，建连计数仍为 0。T10/T11 的 provider 与 connector 在每次使用端点时调用 `AuthorizedResource::binding()?`，会重读绑定、原授权 ID 和端点快照。[请求与计数断言](../../../tests/contracts/contracts.rs)
- **T09-A02 / SEC-004**　四类目标都无法签发写或 SSH 能力。唯一目标为 PROD、将 PROD 改名为普通别名、带上额外的放宽选项、缺少显式目标等用例均未进入远端操作。`INVALID_INPUT` 表示非法字段或操作，`TARGET_NOT_ALLOWED` 表示目标、资源或强制矩阵拒绝，`POLICY_DENIED` 表示当前请求缺少有效授权或确认。[本地管理与 STDIO 用例](../../../tests/policy/environments.mjs)
- **T09-A03**　同一 SQLite 与同一绑定，分别从 main 窗口真实 IPC、独立 CLI 和保持打开的 MCP 读取。撤销前均看到 `dev / primary-db`，撤销后下一次调用均返回空目录，软删除项目后均返回 `PROJECT_NOT_TRUSTED`。已批准端点在确认期间被另一绑定更改时，旧确认被拒绝；主机、端口、数据库名、TLS 任一项变化会撤销该资源所有绑定的 grant 并解除旧引用。不变端点保留原引用。提升级别撤销目标下的 grant，有限期授权的目录能力也有截止时间。切换显式目标不改绑定的工具链 profile。[三入口原生用例](../../../apps/desktop/src-tauri/examples/shared_catalogue.rs) · [事务与并发确认](../../../crates/storage/tests/environment_grants.rs)

### 复验与交接

`cargo test --locked -p envlume-application --test contracts`、`cargo test --locked -p envlume-storage` 和 `cargo test --locked -p envlume --test workspace_trust` 覆盖上述矩阵、迁移与真实 CLI/MCP。编译 workspace_trust 后，把 Cargo 输出的测试可执行文件绝对路径代入下列命令，可在临时用户目录运行三入口对照。

```sh
cargo test --locked -p envlume --test workspace_trust --no-run
cargo run --locked -p envlume-desktop --example shared_catalogue --features tauri/custom-protocol -- --fixture-cli "<workspace_trust 可执行文件>"
```

本机通过 `pnpm install --frozen-lockfile`、`pnpm lint`、`pnpm typecheck`、`pnpm unit`、`pnpm build`、`cargo fmt --all --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings`、`cargo build --workspace --locked` 及 `pnpm desktop build --debug --no-bundle -- --locked`。开发期间曾在新目标目录跑通全量测试。最终修订上，`cargo test --workspace --locked` 和 `cargo test --workspace --release --locked` 均遇到既有 T04 `server/discover` 15 秒超时；采样时 CLI 主线程仍在 `_dyld_start`，尚未输出应用启动日志。保留原超时和断言；上述 T09 专项在最终修订上单独复验通过。T09 的真实三入口证据来自 macOS，本修订未在 Windows 复验。迁移 006 保留旧元数据和凭据引用、撤销旧远端权限；确认端点后仍需重新签发 grant。[迁移 006](../../../crates/storage/migrations/006_endpoint_approval.sql)

R1 留存的两个隔离绑定已通过 `project_context` 核对并完成配置。JDK 17 项目的 `dev / primary-db` 仅获远端 `resource.connect`，JDK 21 项目的 `test / primary-db` 仅获远端 `database.schema`；二者保持各自原有 JDK。交接清单位于本机 `target/r1-handoff-a22d28d/t09-bindings.json`，含新夹具路径、catalogue、绑定与项目编号。T10 在相应工作区使用清单指定的 `T05_FIXTURE_STORE`，先调用 `project_context` 再操作隔离测试条目。独立交接核对有一次夹具 STDIO 启动等待超时；用同一时限复跑后两份目录与 JDK 诊断均通过。此次未创建系统凭据，也未连接 MySQL。
