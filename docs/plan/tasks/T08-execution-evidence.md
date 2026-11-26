# T08 贯通任务证据与本地诊断

Phase 1 · 项目与工具链　·　[返回交付总表](../PLAN.md)

| 开工条件 | 内容 |
|---|---|
| 顺序前置 | [T07 实现具名任务、首次确认和进程控制](T07-named-task-runner.md) |
| 架构与追踪 | [架构基线](../../architecture/ARCHITECTURE.md) §1.3、§8.2、§12、§15 · US-03、US-07、US-09 · E2E-04、E2E-05、E2E-12、SEC-009 |
| 工作范围 | `crates/audit/`、execution、application、storage、CLI/MCP 任务与报告入口 |

## 交付目标

让实际构建、失败和拒绝都可追溯，CLI 与 MCP 可凭同一请求核对执行结果。

## 实施与产出

1. 在 application/audit 汇合 T05–T07。CLI 和 MCP 接入 `toolchain_doctor`、`task_list`、`task_run` 对应入口，沿用 T02 契约。CLI 提供本地按 evidenceId 查看脱敏报告的路径。
2. 生成 requestId，执行动作关联 evidenceId。保存主体、工作区/目录摘要、任务、目标（如适用）、实际版本、命令摘要、时间、耗时、退出码和断言结果。保留限量日志，定义保留期限与清理能力。
3. 将本机、工具链、工作区、策略和工作流诊断连到可操作的下一步。区分 Success、Failed、Denied、Skipped，缺工具和取消不能显示为测试通过。

## 验收

- **T08-A01**　导入真实 Java 8/Spring Boot 测试项目，运行 `mvn test`，对照独立测试报告核验退出码、耗时、版本和 evidenceId。再使一个断言失败，核对同一链路如实记录失败。
- **T08-A02**　在 Node 构建、未授权执行及超时场景中，CLI JSON 和 MCP 返回的请求可在本地关联，跨绑定不能读取另一份记录。
- **T08-A03**　SEC-009 用假秘密覆盖命令错误、stdout/stderr、持久化、报告及清理路径，检查摘要经过脱敏和截断。实际传给 Agent 的内容限于授权范围。

## 交接

移交 Java/Node 两个可复跑场景、成功与失败报告及脱敏扫描结果。

## 执行记录

2026-11-27，在 macOS 26.5.2 arm64 使用修订 `4f447ac` 的 Rust CLI、SQLite 和真实 STDIO 客户端验收。Node 24.14.1，pnpm 10.33.0，Rust 1.99.1。所有项目及数据库使用新建临时目录。

### T08-A01 · Java 8 与独立断言报告

夹具用 Spring Boot 2.7.18 启动应用上下文，JUnit 读取并验证一个 Spring bean。运行时在测试内断言 Java 规范版本为 `1.8`。从 Azul 官方下载 Zulu arm64 JDK 8，核对归档 SHA-256 `792935f732653f5c4fd4cdc68e3ade65cb1fa1fda6b6769fc5dd8098b330b298`，仅在临时目录解压。Maven 3.9.12 的两个运行均通过固定的 `maven-test` 启动。脚本逐项核对 SQLite 报告、CLI 请求编号与退出码、Surefire XML 中的实际 JVM 版本和断言计数。[发行包元数据](https://api.azul.com/metadata/v1/zulu/packages/c0f6e361-0067-49ca-b737-6b682a477f4a) · [测试项目](../../../tests/evidence/spring-boot-java8/pom.xml) · [验收脚本](../../../tests/evidence/java8.mjs)

| 决策 | requestId | evidenceId | JDK | 耗时 | 构建退出码 | Surefire |
|---|---|---|---|---:|---:|---|
| SUCCESS | `111b57f6-4001-462f-a349-5d23518311f2` | `372166b9-4812-4e4c-b7be-7ca4912dec11` | `1.8.0_514` | 2890 ms | 0 | 1 项测试，0 项失败 |
| FAILED | `8a4307de-e188-4794-a9d1-d17a67e5d642` | `d56f2d2d-6447-488d-80fe-8fcea4b31e37` | `1.8.0_514` | 2317 ms | 1 | 1 项测试，1 项失败 |

以上编号来自一次隔离验收，临时数据库随用例清理。用已安装的 JDK 8 重跑时，将 `T08_JAVA8_HOME` 指向其 Home，确保本机可发现 Maven 3.9，并可从 Maven Central 获取夹具依赖。在仓库根目录执行 `node tests/evidence/java8.mjs` 即可生成新编号和两份对照结果。

### T08-A02 · 同一请求与绑定隔离

- 从两个不同工作目录注册绑定。CLI 和 MCP 分别运行固定 Node 构建，从 CLI 按 `requestId` 和 `evidenceId` 查询，核对主体、退出码、时间、耗时和版本。STDIO 的客户端名称由测试故意自行填写，记录的主体为 `LOCAL_STDIO`。
- 未启用、缺少首次确认及附加命令参数均被拒绝，按请求编号可以查到原因。超时留下 `FAILED`，显式取消和缺少工具留下 `SKIPPED`。工作区、绑定与原始请求参数不匹配时，入口拒绝访问。没有可信绑定的请求只返回诊断，不会向所声称的绑定写入历史。
- 用另一已信任绑定查询这两个编号均返回 `POLICY_DENIED`。清理一个绑定后，另一个绑定已有的记录仍可读取。MCP 的 `task_list`、`task_run` 按实时授权展示，报告历史仅在本地 CLI 查询。[独立 CLI/STDIO 验收](../../../tests/execution/evidence.mjs)

### T08-A03 · 脱敏、保留与清理

将假密码分别置于环境、无效命令参数、stdout 和 stderr。两条流各输出超过 40 KB。检查 Agent 协议帧、CLI stdout/stderr、报告、隔离 SQLite 主库及存在时的 WAL，均未发现哨兵。每条流只从前 32 KiB 提取允许的事件或 Maven 汇总数字。Node 输出伪装成 Maven 汇总行时，断言计数为空。

迁移 005 保留原有执行记录。构建启动前先写入待完成记录，结束后原子更新，工作进程中断不会留下成功结论。活动库保留 30 天，读写时清理过期记录；测试将记录时间推进到过期边界以外，并直接核对 SQLite 删除结果。手工 `report clear --binding <id>` 在用户所在的受信任工作区生效。数据库备份按存储恢复策略单独管理。

### 质量门与交接

复跑 `cargo test --locked -p envlume --test workspace_trust` 可覆盖 T07 进程控制及 T08-A02/A03。本机通过 `pnpm install --frozen-lockfile`、`pnpm lint`、`pnpm typecheck`、`pnpm unit`、`pnpm build`、`cargo fmt --all --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings`、`cargo test --workspace --locked`、`cargo build --workspace --locked` 和 `pnpm desktop build --debug --no-bundle -- --locked`。首次全量测试出现既有 T04 `server/discover` 启动超时，按原 15 秒断言复跑通过。新解压的 JDK 有一次首次探测返回 `TOOLCHAIN_MISSING`；独立验收脚本现先核对 `java -version` 和 `javac -version`，再通过 Envlume 的原有探测预算运行构建。

R1 可使用同一 Java 项目、两个入口与编号查询命令验证平台闭环。本卡记录本机 macOS 原生实测，Windows 与 Intel macOS 的本次修订由后续平台验收补证。
