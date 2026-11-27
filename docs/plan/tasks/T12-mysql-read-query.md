# T12 限制 MySQL 查询并脱敏结果

Phase 2 · 环境与 MySQL　·　[返回交付总表](../PLAN.md)

| 开工条件 | 内容 |
|---|---|
| 顺序前置 | [T11 实现 MySQL 连接检测与元数据读取](T11-mysql-connect-schema.md) |
| 架构与追踪 | [架构基线](../../architecture/ARCHITECTURE.md) §7.2、§8.2–8.4、§9.3、§14.2–14.3、§17 · US-06 · E2E-07、SEC-006、SEC-007 |
| 工作范围 | `crates/connector-mysql/`、policy、audit、SQL 与结果策略测试 |

## 交付目标

允许受权项目执行单条只读 SELECT，限制查询范围、数据库权限和返回数据。

## 实施与产出

1. 按 MySQL 方言解析 AST，用允许清单递归检查语句、表、列、表达式及函数，遇无法识别的语法拒绝。所有实际引用须解析到许可集合。
2. 组合专用只读账号或只读副本、表/列策略、列脱敏和查询/结果限额。默认执行超时 5 秒、最多 200 行、完整响应至多 256 KiB，本地策略可收紧。停止超限请求并释放池资源。
3. 对授权的 DEV/TEST 开放 database_query，返回受限行列和 evidenceId。数据库返回作为数据处理。审计保存摘要和策略决定。

## 验收

- **T12-A01**　E2E-07 在真实只读账号上读取允许的元数据和数据。独立验证测试账号的数据库权限。检查敏感列遮蔽，以及结果/审计中无原始秘密或大量行。
- **T12-A02**　SEC-006 拒绝多语句、危险函数、副作用语句、写与管理操作。SEC-007 对跨 schema、越界表、嵌套查询、别名及通配列展开逐项核验。被拒的 SQL 未发送到数据库。
- **T12-A03**　验证零行、长字段、多字节字符、超时、超量行和并发占满的响应及清理。用完整序列化字节数测量输出，调用方要求更大上限时仍遵守本地策略。

## 交接

移交允许/拒绝用例表、行/字节/耗时实测和可按 evidenceId 复查的查询报告。

## 执行记录

2026-11-27，实施修订 `6bc113b`。macOS 26.5.2 arm64、Rust 1.99.1、隔离容器 MySQL 8.4.11，两个独立数据库账号，原生 Keychain、CLI 和 MCP STDIO。[本机脱敏实录](../evidence/T12-query.json) · [查询边界](../../architecture/adr/ADR-0015.md)

| 验收 | 实际结果 |
|---|---|
| T12-A01 | 通过。客户端独立核对 SHOW GRANTS，直接 INSERT 得到 MySQL 1142。已批准的结构和行数据均可读；可读账号临时增加 UPDATE 后，查询在执行前被拒绝。对原列、重命名列和派生别名的直接投影均返回 `[REDACTED]`。通过 CLI 按 MCP 返回的 evidenceId 复查到匹配的 requestId、资源、行列数与遮蔽决定，报告中无 SQL 或行值。 |
| T12-A02 | 通过。12 个真实拒绝请求覆盖多语句、函数、锁、写操作、管理命令、跨库、越界表列、派生表、别名、通配符与遮蔽列的间接使用。凭据访问计数及 MySQL 对该账号的语句计数均未增加。允许的嵌套投影、表别名、COUNT 和通配展开均以真实数据核对。 |
| T12-A03 | 通过。220 行数据在默认策略下返回 200 行；本地收紧为 7 行、再收紧至 3 行，旧授权失效。零行、过长单行、多字节与转义字符分别验证。含 22 行的样本，完整 CLI 为 99,876 字节，完整 MCP 为 255,001 字节（含换行和两份结果）。8 KiB 策略样本完整 MCP 为 1,109 字节。持有测试表锁时同时发出 12 个请求，约 1604 ms 后得到池满及超时诊断；持锁期间检查服务器已无等待查询，释放锁后新查询成功。夹具关闭后 DEV/TEST 物理会话均为零。 |

复跑 `python3 -B tests/mysql/review.py`。macOS 额外经过真实 Keychain 和独立 CLI/MCP，Linux 的专用容器作业通过测试凭据适配器运行同一驱动。工作区检查使用 `pnpm install --frozen-lockfile`、`pnpm lint`、`pnpm typecheck`、`pnpm unit`、`pnpm build`、`cargo fmt --all --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings`、`cargo test --workspace --locked`。

### 移交 T13

CLI 的 SQL 从 stdin 读取。MCP `database_query` 沿用冻结的输入与结果结构。按 evidenceId 回查需在同一受信绑定中运行；夹具清理后，历史样本保存在上方脱敏实录。T13 可用同一夹具对照八项工具与 CLI 输出。
