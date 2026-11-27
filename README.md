# Envlume

A local-first environment gateway for AI coding agents.

Envlume 面向同时维护多个项目的开发者。每个工作区可以分别指定本机工具链、可访问的环境资源，以及允许 AI 执行的构建和测试任务。目标是让 AI 知道当前项目该用哪个 JDK、能查哪个数据库，并让开发者拿到可以复核的执行结果。

**当前可从源码启动桌面 Shell，并通过桌面命令和独立 CLI 读取本地 SQLite 项目目录。** CLI 可导入本地工作区、确认和撤销信任；独立 MCP 进程返回该绑定当前获准的项目摘要和工具链诊断。[查看交付进度](docs/plan/PLAN.md)

## v0.1 的使用场景

假设你在同一台机器上维护两个服务，一个使用 JDK 8，另一个使用 JDK 17。下面是首版需要跑通的流程。

1. 在桌面端导入 Java 8 服务的 Git 工作目录，确认信任，选择本机 JDK 8 和 Maven，启用 `maven-test` 任务。JDK 选择只对该任务的子进程生效，系统 `JAVA_HOME` 和另一个服务的配置保持独立。
2. 给该工作区绑定 DEV MySQL，选择只读账号并授权读取指定表。密码保存在系统凭据存储中。即使 TEST 也有一个名为 `primary-db` 的资源，两个环境的连接和授权仍各自独立。
3. 将 Claude Code、Codex 或 Cursor 接入当前项目。客户端通过模型上下文协议（MCP）启动本地 `envlume mcp` 进程，先用 `project_context` 核对工作区，再用 `database_schema` 查看获准的表。关闭桌面窗口后，客户端仍可调用网关。
4. Agent 通过 `task_run` 执行已启用的 Maven 测试。开发者用返回的 `evidenceId` 查到实际工具版本、退出码、耗时和脱敏日志，核对测试是否通过。

## 运行方式与权限

| 入口 | v0.1 设计 |
|---|---|
| 桌面应用 | 使用 Tauri 2、React 和 TypeScript，负责项目导入、工具链选择、资源授权、客户端接入和执行记录查看。 |
| 独立 CLI | 供开发者和脚本诊断本机环境、管理项目、运行具名任务。 |
| MCP 服务 | 由 AI 客户端按工作区启动，通过标准输入输出（STDIO）通信，无需常驻后台服务。 |

三个入口共用 Rust 应用层。每次调用都会重新检查工作区绑定、目标资源与授权，撤销授权后，已经启动的 MCP 进程也会在下一次调用时被拒绝。SQLite 保存项目配置、凭据引用和脱敏证据，密码由 macOS Keychain 或 Windows Credential Manager 保管。

首版对 MySQL 开放连接检测、元数据读取和受限的单条 `SELECT`，生产目标默认禁用。构建任务会执行仓库自身的代码，因此需要先信任工作区并确认首次执行。权限控制覆盖经 Envlume 发起的操作，AI 客户端自带的终端仍由宿主管理。

## 交付路线

| 版本 | 计划范围 |
|---|---|
| v0.1 MVP | 项目绑定、JDK/Maven/Node/Python 检测、Maven/Node 具名任务、MySQL 只读访问、三个 AI 客户端接入。发行目标为 macOS Apple Silicon 与 Intel 的 DMG、Windows 11 x64 的 NSIS 安装包，以及对应的独立 CLI。 |
| v0.2 | 增加 Redis、Elasticsearch 只读访问、SSH 日志任务及 Gradle 执行。受限写操作的一次性审批按实际需求决定。 |
| v1.0 及以后 | 支持非敏感项目模板与策略的导入导出，按需求扩展企业凭据、连接器和隔离执行。 |

## 参与开发

- 了解模块边界、权限模型和发布标准，阅读[系统架构](docs/architecture/ARCHITECTURE.md)。
- 查找当前任务、前置依赖和验收要求，从[交付计划](docs/plan/PLAN.md)进入对应任务卡。
- 工程约定和真实环境操作边界见 [AGENTS.md](AGENTS.md)。首张 [Phase 0 功能 Issue](https://github.com/Refinex-Space/envlume/issues/1) 跟踪 T01–T04。

### 新检出环境

在 Apple Silicon 或 Intel macOS 上安装 Xcode Command Line Tools（`xcode-select --install`）。在 Windows x64 上安装含 **Desktop development with C++** 工作负载和 Windows SDK 的 Visual Studio Build Tools，以及 Microsoft Edge WebView2 Runtime。安装本机架构的 rustup 和 Node.js 24.14.1，Windows 使用默认 MSVC 工具链。系统依赖见 [Tauri 官方开发环境说明](https://v2.tauri.app/start/prerequisites/)。

从仓库根目录运行以下命令。rustup 首次运行 Cargo 时按 `rust-toolchain.toml` 安装 Rust、clippy 和 rustfmt。首轮需要联网下载工具和依赖。

```sh
npm install --global pnpm@10.33.0
pnpm install --frozen-lockfile
cargo run --locked -p envlume -- --version
pnpm desktop dev
```

CLI 应输出 `envlume 0.1.0-dev.1`。桌面窗口启动后可展开“查看命令行版本”。开发服务器仅监听 `http://127.0.0.1:1420`。按 Ctrl+C 结束开发会话，只预览 Web 界面可用 `pnpm dev`。

### 注册工作区并验证 MCP

从本地终端运行以下命令，把尖括号占位符替换为实际路径或上一步返回的 UUID。

```sh
envlume project add <path> --json
envlume project list --json
envlume project inspect --path <path> --json
envlume project add <worktree-path> --project <project-id> --json
envlume project revoke --binding <binding-id>
envlume project delete --project <project-id>
```

首次导入先显示所选路径、真实目录、Git 类型、构建文件提示和申请的权限。在终端输入 `trust` 确认后只授予 `project.inspect`。每次导入默认新建逻辑项目；可通过 `--project` 把另一个工作目录归入已有项目。每个目录独立确认和授权。重复导入身份未变的已信任目录会返回原绑定，已撤销的 grant 保持撤销。软删除项目需输入 `delete`，其全部绑定和 grant 随事务撤销。

将 MCP 客户端的启动目录设为已确认的工作区根目录，启动命令为 `envlume mcp --binding <binding-id>`。服务从当前用户的本地注册表读取确认来源，并在每次发现和调用时核验目录身份及授权。工作目录被移动、替换或所选链接改指后，旧绑定进入 `STALE`。重新确认会分配新 ID。Windows 支持本地磁盘的 junction，拒绝 UNC、映射网络盘和设备路径。

运行 `cargo test --locked -p envlume --test workspace_trust` 可在临时目录复跑真实注册、终端确认和运行中撤权。`cargo test --locked -p envlume --test stdio` 保留 Phase 0 的模拟协议回归。[T05 任务及验收](docs/plan/tasks/T05-workspace-trust.md) · [STDIO 契约实录](tests/contracts/stdio/README.md)

### 选择和诊断本机工具链

`envlume doctor --json` 查看主机上可发现的工具。要为项目选择版本，在已确认的工作区根目录执行以下命令，将占位符换成本机值。带空格的路径用引号包住。

```sh
envlume toolchain configure --binding <binding-id> --profile default --tool jdk 8 --path "<JDK Home>" --tool maven 3.9 --path "<Maven Home>"
envlume doctor --binding <binding-id> --profile default --json
```

`configure` 会预览这次指定的全部规则，输入 `configure` 后原子保存、选中 profile，并仅向当前 binding 授予 `toolchain.inspect`。可重复使用 `--tool <名称> <版本约束> [--path <绝对路径>]`；省略规则则使用项目约定和本机发现。修改另一已信任绑定正在使用的同名 profile 时，请改用新的 profile 名称。MCP 的 `tools/list` 只向获准的绑定列出 `toolchain_doctor`，参数示例为 `{"profile":"default"}`。

解析依次使用显式路径、持久化版本约束或经确认的 mise 静态版本、项目约定、本机安装。显式路径失效后，报告会提示修复。可选的 `--mise` 读取当前项目 `mise.toml` 或 `.mise.toml` 的 `[tools]` 数字版本并保存快照；修改文件后须再次确认才会更新。诊断只读取 wrapper 元数据和 Maven settings 的存在状态。报告中的路径已摘要化，`repair` 给出需要执行的操作。[T06 验收与支持范围](docs/plan/tasks/T06-toolchain-doctor.md)

### 运行具名任务

在已登记、已选择工具链 profile 的工作区根目录运行。将示例中的绑定 ID 替换为 `envlume project inspect --path . --json` 返回的 `binding.bindingId`。

```sh
envlume task enable maven-test --binding <binding-id> --timeout 600
envlume task run maven-test --binding <binding-id> --json
envlume task enable node-build --binding <binding-id>
envlume task list --binding <binding-id> --json
envlume task run node-build --binding <binding-id> --json
envlume task cancel node-build --binding <binding-id>
envlume task disable node-build --binding <binding-id>
```

启用时在本机终端输入 `enable`，首次构建输入 `run`。仓库的脚本和插件会以当前系统用户的权限运行。Maven wrapper 还会展示声明的 HTTPS 分发地址及可选的 wrapper JAR 地址，需要单独输入 `download`。修改 wrapper、分发地址、任务超时或选定的 profile 规则后须重新确认。自动化调用可使用已有确认，缺少确认时返回 `POLICY_DENIED`。

`maven-test` 执行 Maven 的 `-B -ntp test`，`node-build` 使用项目选定的本机包管理器执行 `run build`。子进程使用绑定的工具、独立构建主目录和按次创建的临时目录。同一用户最多同时运行两项任务，默认超时 600 秒，启用时可指定 1–3600 秒。`cancel` 停止该绑定下指定名称的所有运行实例，`disable` 同时撤销后续运行授权。

JSON 中的 `result.exitCode` 是构建进程的退出码。超时返回 CLI 退出码 6，取消或缺少授权返回 3。每条输出流仅从前 32 KiB 提取允许的构建事件，摘要返回字节数，并在超限时标明 `truncated`。已授权绑定可通过 MCP 的 `task_list`、`task_run` 使用同一执行器。首次构建确认在本地终端完成。[T07 任务与验证](docs/plan/tasks/T07-named-task-runner.md)

### 核对任务证据

在登记的工作区根目录，用任务返回的 `evidenceId` 查看已执行的构建；执行前的拒绝用 `requestId` 查询。命令会重新核验本地目录和绑定身份。

```sh
envlume report show --evidence <evidence-id> --binding <binding-id> --json
envlume report show --request <request-id> --binding <binding-id> --json
envlume report clear --binding <binding-id> --json
```

报告包含调用主体、工具版本、固定命令摘要、时间、耗时、退出码和诊断建议。Maven 输出中可识别的 Surefire 汇总会留下测试计数，可和项目的 `target/surefire-reports` 对照。构建返回非零时，CLI 调用可正常完成，报告的 `decision` 为 `FAILED`。取消和缺少工具记为 `SKIPPED`，授权拒绝记为 `DENIED`。记录在活动 SQLite 目录中保留 30 天，读写记录时清理过期项；`report clear` 删除当前绑定的记录。历史查询仅通过本地 CLI 提供。[T08 验收记录](docs/plan/tasks/T08-execution-evidence.md)

### 配置环境与资源

在已登记的工作区根目录执行以下命令。新增目标、配置端点、授予或撤销能力时，核对本机终端的预览并输入 `confirm`。示例使用保留域名，仅配置目录。

```sh
envlume environment set dev --binding <binding-id>
envlume environment set qa TEST MEDIUM --binding <binding-id>
envlume environment resource dev primary-db dev.example.invalid 3306 VERIFY_IDENTITY orders_dev --binding <binding-id>
envlume environment grant dev primary-db database.schema --ttl 3600 --binding <binding-id>
envlume environment list --binding <binding-id> --json
envlume environment revoke dev primary-db database.schema --binding <binding-id>
```

`set dev|test|staging|prod` 分别采用 LOW、MEDIUM、HIGH、CRITICAL 初始风险；也可显式指定别名、kind 和 risk。端点参数顺序为目标别名、资源别名、主机、端口、TLS 模式和可选的数据库名。TLS 模式接受 `VERIFY_IDENTITY` 或 `DISABLED`。确认提示展示具体端点。省略 `--ttl` 表示持续有效，直到撤销或配置变化。远端授权只发给当前绑定，并授予目录查看能力。

`environment rename <旧别名> <新别名> --binding <binding-id>` 保留语义类别和风险。CLI 的 `environment list --project <project-id> --json` 也会核对当前工作区。MCP 的 `environment_list({})` 和桌面只读 IPC 按相同权限过滤。桌面可在登记目录以 `envlume-desktop --binding <binding-id>` 启动。MySQL 远端操作还需确认拨号地址和元数据范围。[权限矩阵及交接](docs/plan/tasks/T09-target-resource-policy.md)

### 设置和修复 MySQL 凭据

在已登记的工作区根目录，对已确认的资源执行以下命令。设置和轮换时先核对项目、绑定和端点，再在本机终端分别输入用户名和密码；两个字段均关闭回显。SQLite 只保存引用。用 `--json` 可读取同一状态用于诊断。

```sh
envlume credential set dev primary-db --binding <binding-id>
envlume credential status dev primary-db --binding <binding-id> --json
envlume credential replace dev primary-db --binding <binding-id>
envlume credential repair dev primary-db --binding <binding-id>
envlume credential clear dev primary-db --binding <binding-id>
```

| 状态 | 处理方式 |
|---|---|
| `NOT_SET` / `AVAILABLE` | 新资源使用 `set`，已有可用账号需要轮换时使用 `replace`。 |
| `MISSING` / `DAMAGED` | 系统条目已丢失、内容无效或引用与目标不匹配。使用 `repair` 重新录入，或从同目标的有效资源重绑。 |
| `ACCESS_DENIED` / `UNAVAILABLE` | 检查系统访问授权，或解锁凭据库、切换到本地用户会话后重试。 |
| `CHANGED` / `BUSY` | 资源或原引用发生变化，或另一个进程正在修改凭据。重新核对资源后再提交。 |

`credential rebind <target> <destination> <source> --binding <id>` 明确共享同一已确认服务器的系统条目；轮换其中一个资源会产生独立条目。`credential remove-resource <target> <resource> --binding <id>` 停用资源、撤销其远端授权并解除凭据引用。`clear` 仅解除凭据。最后一个引用消失后才删除系统条目。若返回 `cleanupPending: true`，引用变更已保存，可在同一项目的已信任绑定运行 `credential cleanup --binding <id>`。恢复到新目录的 SQLite 副本需要重新核对凭据。[T10 验收与恢复策略](docs/plan/tasks/T10-native-credentials.md)

### 检测 MySQL 连接与读取结构

在已信任工作区，先用 `environment resource` 配置数据库名、主机、端口和 TLS。再准备本地策略文件，例如 `mysql-policy.json`。示例只允许回环服务的 `orders.id`、`orders.name` 和主键。

```json
{
  "ips": ["127.0.0.1"],
  "tables": {
    "orders": {"columns": ["id", "name"], "indexes": ["PRIMARY"]}
  }
}
```

```sh
envlume environment mysql-policy dev primary-db mysql-policy.json --ca fixture-ca.pem --binding <binding-id>
envlume credential set dev primary-db --binding <binding-id>
envlume environment grant dev primary-db resource.connect --binding <binding-id>
envlume environment grant dev primary-db database.schema --binding <binding-id>
envlume connection test dev primary-db --binding <binding-id> --json
envlume database schema dev primary-db --table orders --binding <binding-id> --json
```

省略 `--ca` 使用系统信任根；专用 CA 文件只提交公开证书。请先核对终端显示的 IP、CA 指纹及对象范围，再输入 `confirm`。首次设置路由或更改地址、CA 后需要重新绑定凭据和授权。只修改表列索引清单也需要重新授权。空 `tables` 仅供连接检测，省略 `indexes` 表示不公开索引。请为服务端账号单独授予所需表的只读权限。

已获准的 MCP 绑定会发现 `connection_test` 和/或 `database_schema`；长驻进程中的 `environment_list` 返回最近观测到的健康状态。连接证据可用本地 `report show` 复核。运行产品只需已配置的服务和系统凭据，Docker 用于复跑验收。[T11 验收与诊断](docs/plan/tasks/T11-mysql-connect-schema.md)

### 构建与验证

以下命令适用于 macOS 终端和 Windows PowerShell。先执行上面的锁定安装，并准备好本机 Maven 3、完整 JDK 与 npm。将 `JAVA_HOME` 指向测试 JDK；全量测试在临时工作区执行一次真实的 Maven 测试和 Node 构建，并从 Maven Central 获取夹具依赖。

```sh
pnpm lint
pnpm typecheck
pnpm unit
pnpm build
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo tree --locked -p envlume-domain --target all
cargo build --locked -p envlume
pnpm desktop build --debug --no-bundle -- --locked
```

本机构建的可执行文件位于 `target/debug/`，名称为 `envlume`、`envlume-desktop`，Windows 加 `.exe`。桌面 build 会将生产前端嵌入开发二进制，可关闭 Vite 后运行。三目标 CI 分别核对 rustc 宿主与目标，执行相同检查并显式传入目标三元组，产物位于 `target/<target>/debug/`。[CI 工作流](.github/workflows/ci.yml)

| 原生作业 | Rust 目标 |
|---|---|
| macOS Apple Silicon | `aarch64-apple-darwin` |
| macOS Intel | `x86_64-apple-darwin` |
| Windows x64 | `x86_64-pc-windows-msvc` |

`crates/domain` 只承载领域模型，`crates/application` 供 `crates/cli` 和 `apps/desktop/src-tauri` 共用，React 界面位于 `apps/desktop/src`。主窗口的 capability 只允许调用本地只读命令 `list_projects`。CLI 的 `envlume project list --json` 返回同一份项目 ID 和名称，新库返回空列表。CI 还执行 RustSec 漏洞检查、Rust 与前端依赖许可证扫描和 Git 历史秘密扫描。[威胁与验收编号](docs/architecture/threat-tests.md) · [T03 迁移、恢复与权限验收](docs/plan/tasks/T03-sqlite-and-shared-state.md)

## 许可证

本项目采用 [MIT 许可证](LICENSE)。
