# R1 验收项目与工具链闭环

Phase 1 · 项目与工具链　·　[返回交付总表](../PLAN.md)

| 开工条件 | 内容 |
|---|---|
| 顺序前置 | [T08 贯通任务证据与本地诊断](T08-execution-evidence.md) |
| 架构与追踪 | [架构基线](../../architecture/ARCHITECTURE.md) §6、§14.3、§15、§18 Phase 1、§19.1 · US-01–03、US-07、US-09 |

## 交付目标

证明实际 Java 项目用指定 JDK 完成测试，同时系统环境和其他工作区仍可独立使用。

## 实施与产出

1. 在 macOS、Windows 记录操作系统、机器架构、安装的 JDK/Maven 和仓库修订。用有 JDK 8/17 的环境跑 Java 8 场景。覆盖 JDK 21 的选择用例并核对原环境。
2. 串起导入、信任、工具链诊断、首次批准、执行、证据查询，再撤销授权并使用原 MCP 进程复查。核对 Node 场景及子进程终止记录。

## 验收

- **R1-A01**　E2E-04、E2E-05、E2E-12 有真实工具输出和报告，执行前后 PATH/JAVA_HOME 一致。未信任目录、缺工具和失败断言能区分，且没有虚假的成功证据。
- **R1-A02**　核查 SEC-001、SEC-002、SEC-008、SEC-009、SEC-012 的当前平台证据。确认未决原生差异的影响，缺少本阶段必需行为则保留阻塞。

## 交接

记录 R1 结论和可复用的已信任测试绑定，交给 Phase 2 配置相互隔离的资源。

## 执行记录

**结论：通过。** 2026-11-27 评审修订 `a22d28d`，包含 T08 的执行证据实现。R1-A01 和 R1-A02 均通过，可进入 T09。三目标 CI 在同一修订执行了完整工作区测试、独立 CLI 复验和桌面构建。[原生 CI](https://github.com/Refinex-Space/envlume/actions/runs/45859925135)

### R1-A01 · Java 选择与真实构建

在两个平台上各创建 Java 8、17 和 21 三个独立绑定，先用 `doctor` 核对所选工具，再运行 Spring Boot 2.7.18 的 Maven 测试。成功与故意失败的两次调用均以 `requestId`、`evidenceId` 和实际退出码核对本地报告，测试数及 JVM 版本另与 Surefire XML 比对。构建结束、撤销 Java 8 绑定后，再读 17 和 21 两个绑定。宿主进程的 `PATH`、`JAVA_HOME`、通过 PATH 启动的 Java 版本，以及 macOS launchd 或 Windows 用户级与机器级的两个环境设置，前后逐项相等。[验收脚本](../../../tests/evidence/java8.mjs) · [Spring Boot 测试](../../../tests/evidence/spring-boot-java8/src/test/java/example/ApplicationTest.java)

| 平台 | 构建 JDK / Maven | 其他绑定 JDK 17 / 21 | 宿主 Java |
|---|---|---|---|
| macOS 26.5.2 arm64 | `1.8.0_514` / `3.9.12` | `17.0.18` / `21.0.10` | `25.0.5` |
| Windows Server 2025 x64 | `1.8.0_512` / `3.9.16` | `17.0.21` / `21.0.13` | `17.0.21` |

| 平台 | 构建结果 | requestId | evidenceId | 耗时 | 退出码 | Surefire 失败 / 总数 |
|---|---|---|---|---:|---:|---:|
| macOS 26.5.2 arm64 | SUCCESS | `af7845aa-bb61-4090-9499-672b6deaae7f` | `10a7d1b1-76cb-412e-849d-b5aa79dfa35d` | 25284 ms | 0 | 0 / 1 |
| macOS 26.5.2 arm64 | FAILED | `d99d6f89-bc95-4fe6-8643-81878ff19673` | `ec4f9193-356c-4600-b128-81562ad8abcb` | 5126 ms | 1 | 1 / 1 |
| Windows Server 2025 x64 | SUCCESS | `e22b9e45-e3fc-43cf-81ef-30a0b9fa187d` | `face3596-ace7-4742-883b-d365afd6448f` | 2952 ms | 0 | 0 / 1 |
| Windows Server 2025 x64 | FAILED | `d4ec46d3-4fa5-49df-a4a6-499da4eda323` | `2ab255e8-dcf6-40bb-8738-e5a5d65eab06` | 2725 ms | 1 | 1 / 1 |

macOS 本机使用校验过的临时 Zulu JDK 8，验收后清理该运行时。Windows 使用 runner 的真实安装和独立 CLI。三目标 CI 的常规 Maven/Node 构建另外覆盖 Apple Silicon、Intel macOS 和 Windows 的原生启动与输出。Windows 的原生系统为 Server 2025 x64，Windows 11 安装包继续按发布阶段验收。

### R1-A02 · 信任、进程与证据

| 编号 | 观察到的行为 |
|---|---|
| SEC-001、SEC-012 | 未知绑定或目录不匹配被拒绝。单独测试未启用任务、已有授权但首次构建未确认、附加 argv 和越界 cwd，均未启动工具探针或构建。 |
| SEC-002 | 同一 MCP 进程先执行已确认的 Node 构建，并由 CLI 读取证据。撤销该绑定后向原进程重发同一任务，返回 `BINDING_REVOKED`，没有新 evidenceId，原绑定的证据查询也被拒绝。 |
| SEC-008 | macOS 替换符号链接、Windows 替换 junction 后，旧绑定失效。另测目录移动、大小写别名和 Windows UNC 拒绝。 |
| SEC-009 | 假凭据未进入子进程、协议输出或 SQLite 主库/WAL。两条超量输出分别截断。跨绑定查询同一编号被拒绝，清理一个绑定后另一绑定的记录仍可读。 |
| E2E-12 · Node 终止 | 未授权、缺工具、超时、显式取消分别核对诊断及审计决策。实际子孙 PID 在超时、取消、断开、杀死调用方和运行中撤权后退出，配额可再用。 |

复验入口为 `cargo test --locked -p envlume --test workspace_trust`。[信任测试](../../../tests/workspace/trust.mjs) · [进程树测试](../../../tests/execution/tasks.mjs) · [实时撤权与证据测试](../../../tests/execution/evidence.mjs) · [Apple Silicon 作业](https://github.com/Refinex-Space/envlume/actions/runs/45859925135/job/140047972592) · [Intel 作业](https://github.com/Refinex-Space/envlume/actions/runs/45859925135/job/140047971993) · [Windows 作业](https://github.com/Refinex-Space/envlume/actions/runs/45859925135/job/140055596249)

### 质量门与平台差异

本机 macOS 26.5.2 arm64 完成锁定 pnpm 安装、lint、typecheck、unit、build，以及 Rust fmt、Clippy、workspace build/test。原有本地 debug 产物多次触发 T04 的 `server/discover` 15 秒超时；其中一次在启动后约 6 秒采样，主线程仍位于 `_dyld_start`，此时尚无应用 stderr 或协议响应。具体宿主原因未确定。在新临时目标目录采用 CI 的 `CARGO_PROFILE_DEV_DEBUG=0`、`CARGO_INCREMENTAL=0` 重新构建，并以相同超时、参数和断言跑完全套测试。原 debug 产物的冷启动仍是本机复测限制。

Windows 首轮全量 Rust 测试通过；在独立 CLI 复验时，超时回收后的正常 Node 构建也触发了它配置的 3 秒上限，报告 `TIMEOUT`、耗时 3034 ms。随后只重跑失败的 Windows 作业，原修订、阈值及断言均相同，独立 CLI、Java 8/17/21 及桌面构建全部通过。该次负载波动仍记录在案。[首轮 Windows 日志](https://github.com/Refinex-Space/envlume/actions/runs/45859925135/job/140047971970) · [复跑日志](https://github.com/Refinex-Space/envlume/actions/runs/45859925135/job/140055596249)

### Phase 2 交接

本机私有验收目录为仓库下的 `target/r1-handoff-a22d28d/`。`bindings.json` 保存夹具可执行文件、隔离 catalogue 路径、绑定编号与能力。已从两个目录分别启动 MCP 并通过 `project_context` 核对如下状态。数据库和绑定依赖对应本机目录的身份，移动或清理 `target` 后应重新登记。

| 工作目录 | 已信任绑定 | 已选工具 |
|---|---|---|
| `jdk-17` | `e024b6ef-9b3a-4759-b3c0-284eb9c4eb8d` | JDK 17.0.18 |
| `jdk-21` | `0ada81b2-0089-440d-b739-55e7043939a9` | JDK 21.0.10 |

两者各属独立项目，均只有 `project.inspect`、`toolchain.inspect`，目标列表为空。T09 可从相应工作目录使用清单中的夹具程序，设置 `T05_FIXTURE_STORE` 为清单的 `catalogue`，以 `mcp --binding <id>` 启动后先调用 `project_context`。Java 8 构建绑定已撤销。

需要在新机器重建时，准备 Maven 3.9，设置 `T08_JAVA8_HOME`、`R1_JAVA17_HOME`、`R1_JAVA21_HOME`，在仓库根目录执行 `node tests/evidence/java8.mjs --review`。默认清理临时工作区；将 `R1_HANDOFF_DIRECTORY` 指向尚不存在的本机私有目录可保留两个绑定，脚本输出新编号。
