# R2 验收真实 MySQL 与授权隔离

Phase 2 · 环境与 MySQL　·　[返回交付总表](../PLAN.md)

| 开工条件 | 内容 |
|---|---|
| 顺序前置 | [T13 接齐八项 MCP 工具和 CLI 输出](T13-mcp-cli-contracts.md) |
| 架构与追踪 | [架构基线](../../architecture/ARCHITECTURE.md) §7.2、§9.3、§14.3、§18 Phase 2、§19.1–19.3 · US-04–06、US-09 |

## 交付目标

确认两个项目、两个环境和两个数据库身份之间的隔离，以及三个入口的一致授权结果。

## 实施与产出

1. 准备项目 A/B、同别名 DEV/TEST、带风险标记的 PROD 测试配置及假秘密。记录只读数据库权限、测试数据标识和证据编号。全部连接都指向受控测试资源。
2. 从 MCP/CLI 复跑工具链、查询、授权撤销和超时，检查共享 application 调用路径。核对桌面后端同一服务和系统存储实测，完整 UI 在 Phase 3 验收。

## 验收

- **R2-A01**　E2E-06、E2E-07 在真实数据库上通过。A→B、DEV→未授权 TEST、未指定目标、PROD 默认访问和任何写操作均拒绝。拒绝后无连接副作用。
- **R2-A02**　复查 SEC-001 至 SEC-010 与 SEC-012 的当前证据，扫描 SQLite/WAL、备份、stderr、MCP 及日志。配置回滚 SEC-011 随客户端适配交付。
- **R2-A03**　并发运行两个 MCP 和一个 CLI 后，记录仍可按请求回查。MCP 退出后连接池和任务全部回收。

## 交接

登记 R2 结论、驱动及数据库版本和证据。把脱敏测试数据与夹具交给客户端及桌面任务。

## 执行记录

2026-11-27，Phase 2 通过。夹具修订 `f946042`，本机 macOS 26.5.2 arm64、MySQL 8.4.11、`mysql_async 0.37.1`。使用同一临时 SQLite 库、两个独立工作区、回环 TLS 服务和两个专用只读账号。CLI 与 MCP 测试宿主使用产品分发器、注册表、任务执行器和原生 Keychain，仅指定隔离存储根目录。[脱敏实录](../evidence/R2-mysql-isolation.json) · [端到端脚本](../../../tests/mysql/r2.py)

### R2-A01 · 项目、环境与数据库身份

| 入口 | 实际行为 |
|---|---|
| 项目 A | CLI/MCP 的 `project_context` 均只列出 DEV。对 `dev/primary-db` 查询得到 `R2-DEV` 和遮蔽列 `[REDACTED]`。 |
| 项目 B | 同一目录库中，B 只获准 TEST。`test/primary-db` 查询得到 `R2-TEST`。服务端会话视图独立核对两个账号各自所在的数据库。 |
| 数据库授权 | 两个账号分别用测试 CA 核验证书并登录，SHOW GRANTS 只有 USAGE 和表级 SELECT。直接 INSERT 及直接读取对方数据库的表都被 MySQL 以 1142 拒绝。 |
| 网关拒绝 | A 访问 B 专有资源、从 A 的目录冒用 B 绑定、A 读 TEST、B 读 DEV、缺少 target、注入绑定编号、跨库或写入均失败。两个项目都登记了 CRITICAL 的 PROD，用外观完整的已配置资源验证连接、结构、查询、写入和申请读取 grant 均被拒绝。拒绝批次前后，两个账号各自的累计连接数及已执行语句数增量均为零。 |
| 重查与恢复 | 两个项目分别从 CLI 和 MCP 得到同一工具链诊断。B 的查询在测试表锁下于 5009 ms 返回 TIMEOUT，解锁后查询成功。A 的读取授权撤销后，原 MCP 会话立即拒绝再次查询，原 evidenceId 仍可复查。 |

### R2-A02 · 安全用例与泄漏扫描

| 编号 | 本轮复核的断言 |
|---|---|
| SEC-001、002 | 未注册、目录不匹配的绑定不能访问资源；长驻进程在绑定撤销后拒绝直接调用。[绑定回归](../../../tests/workspace/trust.mjs) |
| SEC-003、004 | 双项目、同资源别名、目标缺失和 PROD 负面矩阵均在数据库调用前拒绝。[R2 实录](../evidence/R2-mysql-isolation.json) |
| SEC-005、010 | 真实工具清单和直接调用均无秘密获取或任意执行入口；正常、错误、调试环境与 EOF 下检查完整 STDIO 帧及 stderr。[八工具验收](../../../tests/security/live-contracts.mjs) |
| SEC-006、007 | 多语句、副作用、危险函数、越界表列与嵌套引用的拒绝均核对凭据读取次数及服务端语句计数。R2 另验证数据库自身拒绝跨库读取。[SQL 验收](../../../tests/mysql/review.py) |
| SEC-008 | 符号链接或 Windows junction 替换后，旧绑定失效。Windows 原生回归同时覆盖 UNC 路径。[绑定回归](../../../tests/workspace/trust.mjs) |
| SEC-009 | 在进程输出、SQLite 主库、保留的 WAL、含本批审计记录且通过 quick_check 的 SQLite 在线备份、应用日志目录及私有构建日志中扫描测试账号、随机口令、遮蔽列哨兵和子进程输出哨兵。UTF-8 和 UTF-16LE 均无命中。本机 WAL 984,712 字节，备份 249,856 字节。实录按通道登记扫描量；备份由验收脚本对活动库生成。 |
| SEC-012 | 未信任或未完成首次本地确认的具名构建被拒绝；构建哨兵与子进程用于核对是否真正启动。[执行回归](../../../tests/execution/tasks.mjs) |

桌面验收通过真实主窗口 IPC 调用 `list_environments`，与 CLI 和存续的 MCP 对照同一绑定。授予资源权限时三方可见同一别名，撤权后三方目录均为空，删除项目后三方均拒绝。数据库查询在 CLI/MCP 验收；桌面完整交互属于 Phase 3。系统凭据分别由 macOS Keychain、Windows Credential Manager 原生作业检查保存、取用、轮换、修复和清理。[原生 IPC 夹具](../../../apps/desktop/src-tauri/examples/shared_catalogue.rs) · [系统凭据用例](../../../crates/secrets/src/tests.rs)

### R2-A03 · 三进程并发与退出

MySQL 同时观测到三个等待元数据锁的查询，分别来自 A 的 MCP、B 的 MCP 和 A 的 CLI。放行后，三组 requestId/evidenceId 均可从原绑定回查到一致的 actor、项目、目标、资源及遮蔽统计；A 查 B 的报告被拒绝。给 A 的 MCP 发送 EOF 时有一项真实 Node 构建运行，父子 PID 和运行槽标记均消失，A 的连接释放，随后 CLI 可再次构建，B 仍可连接。两个会话退出后，服务器上的两个测试账号均为零活动连接。

### 复跑与交接

```sh
pnpm install --frozen-lockfile
pnpm lint && pnpm typecheck && pnpm unit && pnpm build
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
python3 -B tests/mysql/r2.py --snapshot /tmp/envlume-r2-new.json
python3 -B tests/mysql/review.py
python3 -B tests/contracts/live_mysql.py
cargo run --locked -p envlume-desktop --example shared_catalogue --features tauri/custom-protocol -- --fixture-cli target/debug/examples/mysql_review
```

三组 Python 脚本在本机使用隔离容器和合成数据。R2 可通过 `--mysql-bin-dir` 在临时目录启动独立的原生 MySQL 8.4 服务，供 Apple Silicon CI 复跑。Windows CI 核对原生凭据、绑定与 IPC，Linux CI 复跑 MySQL TLS、元数据与受限查询。快照路径须为新文件；复跑前准备 Docker、Node/npm 和仓库锁定的工具链。T14 可直接复用两个工作区、表标记和脱敏快照。SEC-011 配置回滚由 T15–T18 在客户端适配阶段验证。
