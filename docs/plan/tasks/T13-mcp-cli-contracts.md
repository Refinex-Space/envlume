# T13 接齐八项 MCP 工具和 CLI 输出

Phase 2 · 环境与 MySQL　·　[返回交付总表](../PLAN.md)

| 开工条件 | 内容 |
|---|---|
| 顺序前置 | [T12 限制 MySQL 查询并脱敏结果](T12-mysql-read-query.md) |
| 架构与追踪 | [架构基线](../../architecture/ARCHITECTURE.md) §8、§9、§12、§15、§19.2 · US-03、US-06、US-07、US-09 · SEC-005、SEC-010 |
| 工作范围 | `crates/mcp/`、`crates/cli/`、`tests/contracts/`、`tests/security/` |

## 交付目标

用真实应用服务替换协议夹具，通过独立 CLI 和 MCP 读取受权项目、运行任务并查询 MySQL。

## 实施与产出

1. 将 project_context、environment_list、toolchain_doctor、task_list、task_run、connection_test、database_schema、database_query 接到共用服务。启动时核对本地绑定，每次调用重查授权。tools/list 只展示已授权能力。
2. 对照 T02 测成功、拒绝、超时、未知字段、版本协商与断开。不从 roots、transport、cwd 或 tool hint 获得权限。远端工具只接收注册别名，返回内容屏蔽秘密和内部地址。
3. 实测 §12 当前可用命令、CLI `--json`、stderr 和退出码 0/2/3/4/5/6/7。集成子命令由 T15–T18 补齐。诊断覆盖 L1–L7，按请求追踪，执行动作可关联证据。本地查看历史需要绑定和权限。

## 验收

- **T13-A01**　从 STDIO 建立会话，实调八项工具，核对 schema 和授权矩阵。在已打开的会话撤销 grant，直接调用原工具马上收到拒绝。
- **T13-A02**　SEC-005 从实际工具清单和调用入口核对没有 secret getter、通用执行入口。SEC-010 验证正常、错误、调试日志和断连场景的 stdout。SQL、日志内的非可信文本不会改变权限或触发动作。
- **T13-A03**　比较 CLI/MCP 对同一操作的状态、requestId/evidenceId 与本地记录。未知选项、缺工具、拒绝、连接失败、超时、内部错误均映射到约定退出码及脱敏诊断。

## 交接

移交实际协议快照、CLI 输出对照表及端到端脚本，R2 复跑后才进入客户端安装阶段。

## 执行记录

2026-11-27，八工具实现修订 `3125fe1`。macOS 26.5.2 arm64、Node 24.14.1、MySQL 8.4.11。复跑前执行 `pnpm install --frozen-lockfile`，再运行 `python3 -B tests/contracts/live_mysql.py --snapshot <新文件>`。独立进程使用产品 CLI 分发器、STDIO 服务、真实构建子进程、原生 Keychain 和临时只读账号。测试入口指定隔离 SQLite 目录，使凭据读写使用同一签名程序。正式 `envlume` 可执行文件另外接受原生 CLI、工具链、任务与授权回归。[协议输出与 CLI 记录](../evidence/T13-contracts.json) · [端到端脚本](../../../tests/security/live-contracts.mjs)

| 验收 | 实测结果 |
|---|---|
| T13-A01 | 在同一 STDIO 会话逐一核对八项工具的输入 schema，对 CLI 与 MCP 的成功报告和结果运行 T02 JSON Schema 校验。先查 `project_context` 再读 DEV/TEST。现场撤销读取授权后，原会话直接调用得到 `POLICY_DENIED`，发现结果移除查询工具，结构读取仍可用。 |
| T13-A02 | 实际清单只有八项。直接请求读取秘密、通用执行、通用 SQL 或历史报告均得到 JSON-RPC 参数错误。每个已知工具都拒绝额外 cwd；客户端的 roots 声明与 tool hint 不改变授权。数据库中的提示文本以标量返回，任务输出只留下摘要。调试环境、错误和 EOF 下逐帧核对 stdout、stderr 及退出状态。旧版握手协商 `2025-11-25`，新版使用 `server/discover` 和逐请求 metadata，不支持的版本返回 `-32022`。 |
| T13-A03 | 实测下表七种退出码。44 个 CLI/MCP 请求各有独立编号，实际执行的三类操作可按 evidenceId 回查同一记录。L1–L7 分别覆盖本地运行目录、依赖、工作区、网络、数据库服务、策略与工作流。审计对照绑定、actor、操作、requestId、evidenceId 和错误码。 |

| CLI 退出码 | 实际操作 | 报告 |
|---|---|---|
| 0 | 查询 DEV 已批准的数据列 | 成功，有 evidenceId，姓名列返回 `[REDACTED]` |
| 2 | 空白查询或未知选项 | `INVALID_INPUT` |
| 3 | 撤销 `database.read` 后再次查询 | `POLICY_DENIED` |
| 4 | 缺少 Node 构建清单，或 MySQL 拒绝登录 | `TOOLCHAIN_MISSING` / `CREDENTIAL_UNAVAILABLE` |
| 5 | 已登记回环端口拒绝连接 | `NETWORK_UNREACHABLE` |
| 6 | 已登记回环服务接通后不完成握手 | `TIMEOUT` |
| 7 | 隔离工作区的私有构建目录不可用 | `INTERNAL_ERROR` |

§12 的 `--version`、全局及绑定 doctor、project add/list/inspect/context、environment list、task list/run、mcp 已有对应独立进程测试。`environment list --project` 会核对当前工作区的项目归属。集成安装、验证与卸载按计划交给 T15–T18。R2 可从上述夹具继续验收跨项目与跨目标隔离。

历史权限回归发现，保留受信路径并使绑定的授权过期后，仍能读取旧记录。T13-A03 的输出验收已通过，历史读取和清理的权限验收仍待完成。
