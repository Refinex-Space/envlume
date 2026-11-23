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

待执行。收尾时登记修订、平台/版本、实际命令或操作、对应验收编号、结果与脱敏证据位置。
