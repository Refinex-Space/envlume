# T10 接入系统凭据存储与损坏修复

Phase 2 · 环境与 MySQL　·　[返回交付总表](../PLAN.md)

| 开工条件 | 内容 |
|---|---|
| 顺序前置 | [T09 落实环境、资源归属与逐请求授权](T09-target-resource-policy.md) |
| 架构与追踪 | [架构基线](../../architecture/ARCHITECTURE.md) §4.5、§7.1、§9、§10.1–10.5、§13.4、§17 · US-05 · SEC-009 |
| 工作范围 | `crates/secrets/`、application、系统原生 provider 与集成测试 |

## 交付目标

资源保存 credentialRef，连接器经授权读取秘密，用户可修复或更换失效凭据。

## 实施与产出

1. 实现 macOS Keychain、Windows Credential Manager 和受控的测试 provider。凭据通过一次提交写入系统存储。应用层对外只提供设置、替换、清理和可用状态。
2. 协调元数据与系统存储的不同事务边界，处理新增、更新中断和重试。变更引用或移除资源前检查引用关系，保留仍被合法使用的凭据。对新资源按环境隔离凭据。
3. 区分引用损坏、用户拒绝系统访问、系统存储不可用。提供本地重新绑定/录入的修复用例。日志清洗覆盖驱动和 OS 原始错误。

## 验收

- **T10-A01**　在两个原生系统用专用测试条目实测保存、连接器取用、轮换、删除以及损坏再修复。使用 fake provider 注入中断，核对旧配置可恢复。
- **T10-A02**　两个资源合法引用同一条目时删除一个，另一个仍可使用。并发替换或写入失败后不会静默指向错误凭据。
- **T10-A03**　假秘密标记仅在获准的 provider/连接器内使用。扫描 SQLite/WAL、备份、日志和序列化输出，均无明文。构建层没有访问 provider 的入口。

## 交接

移交两个平台的凭据生命周期记录与修复用例。证据仅记录条目状态和脱敏错误。

## 执行记录

2026-11-27，以修订 `a35c45c` 验收。所有系统条目均使用隔离 catalogue 和随机名称，仅提交合成账号。三个 native 作业的 `Verify native credential lifecycle and build isolation` 步骤均通过。[原生验证与安全作业](https://github.com/Refinex-Space/envlume/actions/runs/45932373220)

| 编号 | 操作与结果 |
|---|---|
| T10-A01 | 本机 macOS 26.5.2 arm64、macOS 15 arm64/Intel 的 Keychain 与 Windows Server 2025 x64 的 Credential Manager 均实测保存、获准连接器取用、轮换、条目损坏或缺失、重新录入和清除。未授权时连接器消费计数为 0。fake provider 另行注入拒绝访问与不可用，比较脱敏状态和可重试性。 |
| T10-A02 | 同一目标、相同服务器的两个资源经显式重绑共享系统条目。移除一个资源后，另一个仍能通过授权连接器使用它；清除最后一个引用才删除条目。并发轮换时验证锁冲突和过期快照拒绝。OS 已写入但报告失败、SQLite 指针发布失败、等待期间端点被改动时，均未将候选凭据挂到错误资源。重开目录后可重试清理。 |
| T10-A03 | 向用户名和密码分别放入测试标记，检查成功、拒绝和损坏报告，以及有活动 WAL 的 SQLite、备份和真实 CLI 终端输出。从 MCP 读取项目与环境视图，并验证工具列表和直接调用均不暴露秘密读取入口。构建执行器的 Cargo 依赖树独立核对，无 `envlume-secrets`。 |

[macOS arm64](https://github.com/Refinex-Space/envlume/actions/runs/45932373220/job/140287680348) · [macOS Intel](https://github.com/Refinex-Space/envlume/actions/runs/45932373220/job/140287680397) · [Windows x64](https://github.com/Refinex-Space/envlume/actions/runs/45932373220/job/140287680091) · [安全扫描](https://github.com/Refinex-Space/envlume/actions/runs/45932373220/job/140287680318)

### 恢复与复验

凭据内容由 OS 保管；SQLite 的事务只处理引用、状态和清理队列。候选条目先入队，原引用在新条目成功写入及快照复核后才切换。删除系统条目前按全库资源检查引用。清理被 OS 拒绝时返回已提交状态和 `cleanupPending`，本地 `credential cleanup --binding <id>` 可重试。恢复到新目录的备份使用独立 OS 命名空间；在恢复副本中清除旧引用后，原目录的系统条目仍可用。[恢复决议](../../architecture/adr/ADR-0013.md)

本机和 CI 使用以下命令；第一个测试同时覆盖真实系统条目、故障注入、共享、并发和快照扫描。第二个测试通过 PTY 输入合成账号，扫描终端、CLI JSON、MCP 帧及诊断流。开发时该测试实际捕获了密码库显示提示与关闭回显之间的竞争。修订后先关闭回显再显示提示，保留原哨兵断言。

```sh
cargo test --locked -p envlume-secrets -- --nocapture
cargo test --locked -p envlume --test workspace_trust
cargo tree --locked -p envlume-execution --prefix none
```

[系统适配与测试](../../../crates/secrets/src/tests.rs) · [CLI/MCP 脱敏用例](../../../tests/credentials/lifecycle.mjs) · [CLI 使用与修复步骤](../../../README.md#设置和修复-mysql-凭据)

### 质量门

本机通过锁定 pnpm 安装、lint、typecheck、unit、build，以及 Rust fmt、Clippy、workspace 构建和 T10 原生专项。一次本机全量测试在既有 T04 `server/discover` 15 秒冷启动检查处超时；预构建并启动 CLI 后，以相同的 `cargo test --workspace --locked` 和原超时断言复跑全绿。此次 CI 还覆盖随分支交付的 T09，三个原生作业分别运行全量 Rust 测试、独立 CLI 和桌面构建。安全扫描使用既有的全历史规则。首个临时提交的 Cargo workspace 长行曾被识别为疑似密钥；将路径逐行列出并按精确租约替换该提交后通过扫描。

### 交接给 T11

已有 DEV、TEST 两个 `primary-db` 资源已通过各自的 `project_context` 核对。对 DEV 单独设置、轮换、清除期间，TEST 保持 `NOT_SET`；验收结束时两者均为 `NOT_SET`。本机 `target/r1-handoff-a22d28d/t10-bindings.json` 记录夹具程序、catalogue、绑定编号和状态。T11 可用本地终端为新建的只读测试账号录入凭据，连接器通过 `NativeVault::with_login` 在已授权的 Rust 回调内使用。本次使用连接器测试替身，真实 MySQL 连接、TLS 和连接池由 T11 验收。
