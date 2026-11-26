# T07 实现具名任务、首次确认和进程控制

Phase 1 · 项目与工具链　·　[返回交付总表](../PLAN.md)

| 开工条件 | 内容 |
|---|---|
| 顺序前置 | [T06 解析本机工具链并诊断版本冲突](T06-toolchain-doctor.md) |
| 架构与追踪 | [架构基线](../../architecture/ARCHITECTURE.md) §6.3–6.4、§9、§12、§14.2、§17 · US-07 · SEC-012 |
| 工作范围 | `crates/execution/`、toolchain、application、policy、CLI 任务入口 |

## 交付目标

让已信任工作区只执行用户启用的固定任务，同时控制子进程环境、超时和取消。

## 实施与产出

1. 维护 TaskDefinition，区分 workspace trust、任务授权和首次构建确认。暴露业务用例 `task list/run`，参数使用已解析的可执行文件、固定 argv、绑定内工作目录和非敏感选项。
2. 构造子进程环境白名单并应用选定 JDK/Node 等覆盖。校验可执行文件与工作路径，不经通用 Shell 解释模型输入。验证 Maven 与 Node 在 macOS/Windows 的实际启动路径，特别处理 wrapper 和包管理器启动器差异。
3. 只有受信任工作区可运行批准的 wrapper，若需下载 distribution，先展示 URL 并取得项目批准。首次执行提示仓库脚本可运行本机代码。headless 缺确认就拒绝。
4. 实现并发上限、限定输出、超时、取消和子孙进程回收。让执行结果携带版本、时间、退出码与脱敏摘要，供 T08 存档。

## 验收

- **T07-A01**　SEC-012 对未信任工作区、未启用任务、缺首次确认逐一验证，均无子进程启动。模型提供的自由命令、额外 argv 或越界 cwd 被拒绝。
- **T07-A02**　分别运行固定 Maven 测试和 Node 构建。检查带空格/非 ASCII 路径，断言子进程使用选定工具，凭据哨兵不可见，父进程与系统环境快照一致。
- **T07-A03**　超时、显式取消及调用端断开后子进程树退出，连接和并发配额释放。超量日志可见截断标识。

## 交接

移交真实任务调用、首次确认和超时/取消证据。远端目标独立于本地任务授权。

## 执行记录

2026-11-26，修订 `02f4f04` 通过 T07-A01、A02、A03。三个原生作业分别用隔离存储和独立 CLI 二进制运行同一组验收，并完成桌面开发构建。[三目标 CI](https://github.com/Refinex-Space/envlume/actions/runs/45845902869)

| 实测平台 | Node | Maven | 构建使用的 JDK | 证据 |
|---|---|---|---|---|
| macOS 15 arm64 | 24.14.1 | 3.10.1 | 21.0.13 | [原生作业](https://github.com/Refinex-Space/envlume/actions/runs/45845902869/job/139999366442) |
| macOS 15 Intel | 24.14.1 | 3.9.16 | 21.0.12.1 | [原生作业](https://github.com/Refinex-Space/envlume/actions/runs/45845902869/job/139999366545) |
| Windows Server 2025 x64 | 24.14.1 | 3.9.16 | 17.0.21 | [原生作业](https://github.com/Refinex-Space/envlume/actions/runs/45845902869/job/139999366185) |
| 本机 macOS 26.5.2 arm64 | 24.14.1 | 3.9.12 | 25.0.5 | 本卡复跑命令，临时 APFS 工作区 |

本机通过 `pnpm install --frozen-lockfile`、`pnpm lint`、`pnpm typecheck`、`pnpm unit`、`pnpm build`、`cargo fmt --all --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings`、`cargo test --workspace --locked`、`cargo build --workspace --locked` 及 `pnpm desktop build --debug --no-bundle -- --locked`。远端许可证和 Git 历史秘密扫描通过，RustSec 报告三条既有警告。[安全作业](https://github.com/Refinex-Space/envlume/actions/runs/45845902869/job/139999365999)

原生回归覆盖了 macOS 只剩已退出主进程时的进程组清理和输入管道的半关闭。Windows 用 Job Object 内的真实 npm 脚本验证系统 `ComSpec` 使用原生路径分隔符，并用子进程退出码 0 和 23 核对回收后的结果。固定启动器和 wrapper 分项确认的决定见 [ADR-0011](../../architecture/adr/ADR-0011.md)。

### 验收方法

| 编号 | 本地隔离用例 | 可复查的断言 |
|---|---|---|
| T07-A01 | 未注册、撤销、同项目的另一绑定、未启用、首次非交互执行；额外 argv、自由命令和越界 cwd；SQLite 中篡改任务定义 | 拒绝发生在工具版本探针和构建之前。探针与构建的标记文件均未生成。更改超时、撤销后再启用或修改分发 URL 都需再次确认。 |
| T07-A02 | 在 `T07 空格 build` 运行真实 `npm run build` 和 Maven `test` | Node 记录实际 executable 与版本；JUnit 比较所选 JDK 与实际 JVM，Surefire 报告为 1 项通过。子进程与输出均检查凭据哨兵。额外的非零退出构建验证 CLI 与子进程退出码各自的语义。 |
| T07-A03 | 同时占满两个执行槽，启动子、孙进程并记录 PID，分别触发超时、取消、关闭输入、强杀调用进程及运行中撤权 | 第三个构建被拒绝，记录的 PID 全部退出，随后可再运行。stdout 超过 70 KB、stderr 超过 40 KB 时，两份摘要分别报告截断。CLI 的成功与拒绝响应均按 T02 JSON Schema 验证。 |

[原生进程用例](../../../tests/execution/tasks.mjs) · [绑定与首次确认事务用例](../../../crates/storage/tests/task_permissions.rs) · [CLI 与进程组回归](../../../crates/execution/src/tree.rs)

在装有完整 JDK、Maven 3、Node 和 npm 的开发机上，从仓库根目录复跑。设置 `JAVA_HOME` 为测试用 JDK。首次运行 Maven 夹具需要从 Maven Central 取得固定的 JUnit 和测试插件依赖。

```sh
pnpm install --frozen-lockfile
cargo test --locked -p envlume-storage --test task_permissions
cargo test --locked -p envlume-execution
cargo test --locked -p envlume --test workspace_trust
```

**首次确认与启动方式。** 工作区信任、当前绑定的运行授权、首次构建确认分别判断。批准只对选定绑定、授权、任务模板、超时、profile 和启动脚本摘要有效。已安装 Maven 3 经选定的 Java 加载 Classworlds，包管理器经选定 Node 加载安装包的 JS 入口。Windows 上仅经批准的仓库 `mvnw.cmd` 使用系统 `cmd.exe` 和固定字面参数。wrapper 的 HTTPS 地址另行展示并确认。原生 wrapper 夹具写入启动标记并验证 URL 变更后的拒绝，使用 `example.invalid`，不发起网络下载。报告将 wrapper 声明的分发版本标为 `maven-wrapper-declared`。

**执行与输出。** 每个本机用户最多同时运行两项任务；POSIX 为构建创建进程组，Windows 在挂起状态下将构建进程加入 Job Object 后再执行。独立工作进程持有配额和调用方存活管道。两条日志流各自排空，仅检查前 32 KiB，向调用方返回计数、截断状态和允许的 Maven 事件。子进程环境从空环境构造，并使用独立 HOME 与临时目录。获准的构建脚本仍具有当前用户的文件与网络权限。

**交接给 T08。** `RunResult` 提供 requestId、evidenceId、版本、UTC 时间、耗时、构建状态、子进程退出码及两份脱敏摘要。T08 将这些数据存档，再接入证据查询、MCP 的任务入口和跨绑定报告隔离。T07 的执行编号目前仅标识本次运行。
