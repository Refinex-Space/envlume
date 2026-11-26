# T06 解析本机工具链并诊断版本冲突

Phase 1 · 项目与工具链　·　[返回交付总表](../PLAN.md)

| 开工条件 | 内容 |
|---|---|
| 顺序前置 | [T05 实现项目导入、绑定和信任撤销](T05-workspace-trust.md) |
| 架构与追踪 | [架构基线](../../architecture/ARCHITECTURE.md) §6.1–6.3、§12、§15.2、§17 · US-02、US-03 · E2E-05 |
| 工作范围 | `crates/toolchain/`、application、profile 仓储与 doctor 入口 |

## 交付目标

同机安装多套工具时，为每个项目选择可说明来源、版本和路径的工具链。

## 实施与产出

1. 实现显式路径/JDK Home、已锁定或获准的可选 mise 配置、项目原生约定、本机发现的解析优先级。macOS 兼顾无终端 PATH 的 GUI，Windows 使用原生路径。发现 wrapper 时仅解析信息。
2. 识别 JDK/java/javac 的版本、发行版和架构，Maven/JDK 配对、Gradle 基础信息、Node 和 npm/pnpm/yarn 的 lockfile、Python/uv/venv、Git/worktree、Rust/Cargo、Docker/mise 是否存在。
3. 提供可持久化的 ToolchainProfile、主机 doctor 和绑定级 toolchain_doctor 用例。报告来源、约束、实际版本、有效路径摘要与修复动作。Maven settings 只报告配置状态。

## 验收

- **T06-A01**　以不同显式路径、系统发现、缺失工具、版本冲突、带空格路径和 GUI 精简环境为输入，核对选择及原因。探测不安装工具、不改父进程环境。
- **T06-A02**　指定 Java 8 项目时优先选 JDK 8，与选用 17 的项目独立。识别 JDK 21 时仍服从指定版本。断网亦可诊断本机已有工具。
- **T06-A03**　对缺 JDK、包管理器与锁文件不匹配、失效解释器分别给出可执行的修复建议。测试报告和 doctor 输出均不含 settings 内容或环境凭据。

## 交接

移交跨平台工具发现夹具、两套项目配置和诊断样例，供 T07 与 R1 做实际执行对照。

## 执行记录

### 2026-11-26 · 解析器与入口

**T06-A01 至 A03 通过。** 验收修订 `00ee28a8b725d63ed4c82496d0ffdd5bb7f45811`，三目标原生作业及供应链检查均通过。[完整 CI](https://github.com/Refinex-Space/envlume/actions/runs/45829345357) · [Apple Silicon](https://github.com/Refinex-Space/envlume/actions/runs/45829345357/job/139941690180) · [Intel macOS](https://github.com/Refinex-Space/envlume/actions/runs/45829345357/job/139941690000) · [Windows x64](https://github.com/Refinex-Space/envlume/actions/runs/45829345357/job/139941689731)

在 T05 的 `26516f5` 上实现。沿用 SQLite 的 `toolchain_profiles`、binding 的 `profile_id` 和 T02 的 `toolchain_doctor` 契约。本地终端确认后，选中 profile、更新规则与授予本绑定诊断权限在同一事务内完成。在会话中调用 doctor 会重新验证绑定、grant 和当前选中的 profile；MCP 输入仍然只接受逻辑别名 `profile`。[CLI 用法](../../../README.md#选择和诊断本机工具链) · [profile 隔离测试](../../../crates/storage/tests/toolchain_profiles.rs)

| 验收 | 用例和证据 |
|---|---|
| T06-A01 | 以带空格的显式 JDK/Maven Home、本机多个候选、缺失路径和空 PATH 的 GUI 目录夹具核对选择顺序。显式路径失效报告 missing，不回退到其他 JDK。workspace 测试比较诊断前后父进程环境；模拟原生版本探针检查 cwd 和凭据哨兵。 |
| T06-A02 | 两个工作区分别选择 JDK 8 和 17，主机选择 21。CLI 与独立 MCP 读到相同的 `java`、`javac` 版本及 Maven 配对；修改 mise 文件不会改变已批准的版本快照。在本机用 macOS `sandbox-exec` 禁止网络后重复运行隔离的子进程套件，结果通过。 |
| T06-A03 | 缺 JDK、Java/编译器不一致、包管理器与 lockfile 不一致、失效 venv 和不兼容的 Maven/JDK 组合会给出修复动作。CLI/MCP 报告不会回显安装根目录、mise 环境值、Maven settings 文件内容、版本探针原始输出或环境凭据。 |

**Windows 实际安装。** Windows Server 2025 x64 的正式 CLI 通过本地终端为两个绑定独立保存显式 JDK Home。真实读数为项目一 `1.8.0_512`、项目二 `17.0.21.1`、主机 `21.0.13.1`，每项均验证 java 和 javac。此轮直接探测 runner 已有安装。[Windows x64 作业与版本实录](https://github.com/Refinex-Space/envlume/actions/runs/45829345357/job/139941689731)

**实际本机读数。** macOS 26.5.2 arm64，Rust 1.99.1、Node 24.14.1、pnpm 10.33.0。禁网下的主机 doctor 检出 Temurin 25.0.5 arm64（java 与 javac 一致）、Maven 3.9.12、Node 24.14.1、Python 3.14.6 和 Git 2.52.0，最终修订的一次预热后测量为 0.14 秒。将子进程 PATH 缩为 `/usr/bin:/bin` 并移除 Java/Maven Home 后，仍通过安装目录发现 Homebrew JDK 27 和 Node 22.22.1。禁网环境中另外选定本机两个 Azul JDK，java 与 javac 均分别报告 17.0.18 和 21.0.10。隔离夹具的 8/17/21 读数分别为 `1.8.0_412`、`17.0.11`、`21.0.4`。

一次主机 doctor 的 JSON 行如下，路径保留到工具逻辑位置。

```json
{"tool":"maven","source":"selected-home","constraintSource":"host","requirement":"*","version":"3.9.12","pathSummary":"<maven-home>/bin/mvn","status":"available","details":["paired JDK 25.0.5","user settings absent","installation settings configured"],"repair":null}
```

**探测范围。** JDK 核对同一 Home 的 java/javac；发行版与架构取自 release 元数据，缺字段显示 unknown。Maven、Gradle 读取发行目录中的核心库版本；Maven 配对检查已知最低 JVM 要求。POM 编译目标用于默认选择，显式较新 JDK 可面向较旧编译目标。Node 读取 `.nvmrc`、`engines.node`、`packageManager` 与 lockfile；Python 读取 `requires-python`、`.python-version` 及 venv 元数据。Git、uv、rustc 和 Cargo 支持本机版本探测，Docker 与 mise 为安装状态检查。工具缺失时的安装建议须由用户自行执行。[Maven 版本与所需 Java](https://maven.apache.org/docs/history.html) · [mise 配置参考](https://mise.jdx.dev/configuration.html)

**离线与输出边界。** mise 支持显式导入静态数字 pin，动态模板、插件、环境段、hooks 及远程安装均不执行。profile 支持数字前缀、比较范围、`^`、`~`、`~=` 及 `||` 组合；无法离线解释的约束返回诊断。wrapper 与项目 venv 使用元数据检查。可执行的版本探针使用固定 argv、隔离工作目录、清空后按需构造的子进程环境，限制输出与等待时间。MCP 通过既有 `tool/version/pathSummary/repair` 字段返回，`pathSummary` 包含来源、约束、状态和脱敏路径。成功生成报告的退出码为 0，各工具的缺失与冲突可从报告读取。

**质量门。** 本机完成 `cargo fmt --all --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings`、`cargo test --workspace --locked`、`cargo build --workspace --locked` 及 Tauri 开发构建。前端完成锁定安装、lint、typecheck、unit 与生产构建。七项解析器单测、profile 事务测试、两套独立 CLI/MCP 项目配置与既有契约、信任、存储回归均通过。许可证及秘密扫描通过，RustSec 保留三条既有依赖警告。[安全作业](https://github.com/Refinex-Space/envlume/actions/runs/45829345357/job/139941689548)

**复验中处理的问题。** Windows 上一次真实 JDK 8 检查曾得到安装不完整的诊断。以原生夹具延迟 javac 启动 2.4 秒复现了同样结果；原实现把单进程 2 秒超时与不可执行混为一类。修订后 java/javac 并行共享原有的单工具 5 秒预算，超时返回独立诊断，真实 JDK 的版本与修复动作断言保持原样。另一次本机全量测试触发 T04 已记录的 `server/discover` 15 秒超时，以相同测试参数复跑通过。

**交接给 T07。** `LocalToolchain::resolve` 返回本地 `Inventory`，其中可用工具的 `executable` 和 JDK 的 `java_home` 只在 Rust 内可见。T07 在批准具名任务后据此构造子进程环境并执行；wrapper 分发包下载和真实 Maven 测试执行仍由 T07/R1 验收。[解析测试](../../../crates/toolchain/src/lib.rs) · [双项目 CLI/MCP 夹具](../../../tests/toolchain/doctor.mjs)

在仓库根目录用以下命令复跑。

```sh
cargo test --locked -p envlume-toolchain
cargo test --locked -p envlume-storage --test toolchain_profiles
cargo test --locked -p envlume --test workspace_trust
cargo run --locked -p envlume -- doctor --json
```
