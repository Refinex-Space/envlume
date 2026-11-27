# T11 实现 MySQL 连接检测与元数据读取

Phase 2 · 环境与 MySQL　·　[返回交付总表](../PLAN.md)

| 开工条件 | 内容 |
|---|---|
| 顺序前置 | [T10 接入系统凭据存储与损坏修复](T10-native-credentials.md) |
| 架构与追踪 | [架构基线](../../architecture/ARCHITECTURE.md) §7.1–7.2、§8.2、§14、§15.2 · US-05、US-06 · E2E-06、E2E-07 |
| 工作范围 | `crates/connectors/`、`crates/connector-mysql/`、application、真实服务集成测试 |

## 交付目标

用 DEV、TEST 两个独立只读身份完成真实 MySQL 连通性检测和授权元数据读取。

## 实施与产出

1. 建立 typed connector registry，支持 test_connection、capabilities、health、close。按 binding→target→授权→credentialRef→TLS→操作→脱敏→审计的顺序调用。以锁定驱动实现独立资源连接池。
2. 端点来自本地已批准配置，校验 DNS 解析后的地址、端口和 TLS 证书，限制超时、连接数、空闲时间和响应大小。区分 DNS、TCP/VPN、TLS、认证及服务版本问题。
3. 实现 database_schema（允许的 schema/table/column/index），只显示许可范围内元数据。配置两套可区分 schema/账号的真实 MySQL 测试资源，验证连接复用、撤销和资源关闭。运行产品无需数据库客户端或 Docker。

## 验收

- **T11-A01**　DEV、TEST 均以 `primary-db` 注册，先后及并发调用仅返回各自标记表。不同项目无权解析另一项目的端点。撤销后原池无法继续使用该授权。
- **T11-A02**　用测试证书验证受信证书成功、校验失败时连接被拒绝，未批准地址/目标在访问前被拒。不可达、认证错误、超时均有独立诊断。
- **T11-A03**　只读账号可查看获准表的列和索引，隐藏表不出现在返回中。检查健康/错误报告、池大小和关闭后的连接计数。

## 交接

移交真实 MySQL 测试夹具、连接器权限矩阵与脱敏连接报告，供 T12 在相同资源上执行查询。

## 执行记录

2026-11-27，连接器验收修订 `8065dba`，策略文件入口追加核验修订 `cac4672`。本机 macOS 26.5.2 arm64 使用专用 Docker 容器 MySQL 8.4.11，镜像固定到多架构摘要，端口仅发布在回环。临时 CA、服务器证书、数据库、账号和系统凭据随夹具清理。Linux CI 使用相同容器和驱动，夹具在进程内提交生成的登录信息；本机另走原生 Keychain、独立 CLI 和真实 STDIO。产品只依赖锁定的 Rust 驱动及系统 TLS。[MySQL CI 日志](https://github.com/Refinex-Space/envlume/actions/runs/45941672294/job/140317156288) · [本机脱敏实录](../evidence/T11-mysql.json)

| 验收 | 实测与断言 |
|---|---|
| T11-A01 | DEV 和 TEST 都注册 `primary-db`，分别返回 `dev_marker` 和 `test_marker`。连续四次探测保持同一服务端连接 ID；四个并行元数据请求各自返回正确标记。跨项目调用取凭据次数为 0。撤销 DEV 授权后，原进程下一次调用返回 `POLICY_DENIED`，旧会话从服务器消失，重新签发 grant 后使用新的连接 ID。 |
| T11-A02 | 临时 CA 签发的 `localhost` 证书通过身份验证；不受信 CA 和身份不匹配均返回 TLS 诊断。DNS 无记录或解析 IP 未在本地清单中时，在取凭据前拒绝。独立验证 TCP 拒绝、MySQL 登录拒绝和接通后无握手响应，分别提示 TCP/VPN、认证和超时，最后一项本机耗时 5007 ms。所有拨号均使用已校验地址；测试参数中的额外 `host` 字段返回 `INVALID_INPUT`。 |
| T11-A03 | 独立 MySQL 客户端核对两套账号的 `SHOW GRANTS`，实际写入均被拒绝。账号有权限看到的 `hidden_table` 仍被本地策略过滤。标记表仅返回 `id`、`name` 及允许的 `PRIMARY`、`name_idx`，未列出的索引以及含隐藏列的复合索引也被过滤。环境健康报告与成功/认证失败相符。服务端观测到并发后 DEV 3 条连接（两个 grant 的池）、TEST 2 条，空闲回收及显式 close 后均为 0。 |

连接检测的本机样本为 `requestId=890e2533-38a1-43a0-b680-8ab552abe62d`、`evidenceId=324b12ef-d4dc-41aa-8794-bdf1b6d778df`、92 ms。CLI 报告与 SQLite 审计核对到同一目标及资源，MCP 展示的输入 schema 与 T02 一致。样本来自已清理的测试目录，重跑会生成新编号。

### 范围和复跑

- SQLite 迁移 008 默认不给旧资源添加拨号许可。配置文件只能由受信绑定的本机管理入口读取，并限制本地文件类型与大小。变更 IP/CA 后解除旧凭据并撤销远端 grant；只改元数据清单撤销 grant、保留凭据。[策略测试](../../../crates/storage/tests/environment_grants.rs) · [地址与 TLS 决议](../../architecture/adr/ADR-0014.md)
- 一个连接器实例最多 8 个并发操作、8 个活动池，每个绑定/目标/资源/授权代际池最多 2 条连接，最小为 0，空闲期限为 30 秒。数据库 I/O 使用 5 秒截止时间。最多批准 16 个 IP、32 张表、512 个列与索引条目；有界读取结果，256 KiB 响应预算计入结构化内容与转义文本副本。超限整次失败。
- 在仓库根目录运行 `python3 -B tests/mysql/review.py`，需要 Docker 和 OpenSSL。在 macOS 上该脚本还核对 CLI、MCP 与原生 Keychain，为保留系统 ACL 使用同一测试二进制启动正式分发入口。运行时不需要在宿主安装 MySQL 客户端。全量质量门使用 `cargo fmt --all --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings`、`cargo test --workspace --locked` 以及 README 所列 pnpm 和桌面构建命令。[容器及 SQL 夹具](../../../tests/mysql/review.py) · [连接器验收入口](../../../crates/cli/examples/mysql_review.rs)
- 初次依赖审计在 Linux TLS 路径检出 OpenSSL Rust 绑定 0.10.77 的公告，已将锁文件更新至 0.10.82，未增加审计例外。[安全作业](https://github.com/Refinex-Space/envlume/actions/runs/45941672294/job/140317155950)

### 移交 T12

复用上述临时账号、数据库与策略清单。T12 可从同一授权句柄和结果大小预算接入受控 SELECT 的 AST、行列限制和脱敏。T11 只提供连接检测与元数据操作；现有 T10 交接绑定保留原占位地址及凭据状态。
