# T02 入口契约 v1

桌面、CLI 和 MCP 适配器共用 Rust application。这里冻结调用参数、应用结果和权限词汇，供 T03、T04 和 T13 实现时复跑。首版 CLI 当前可执行的命令见项目 README。[架构 §8、§12、§15](../../docs/architecture/ARCHITECTURE.md) · [交付计划](../../docs/plan/PLAN.md)

## 请求

[tools.json](tools.json) 按工具名给出 Draft 2020-12 的 `inputSchema`、成功时 `result` 的 `resultSchema`，以及所需 capability。[examples.json](examples.json) 给出八项工具各自的参数与返回样例。MCP 适配器可取用 `inputSchema`；传输帧和协议协商由 T04 单独验证。

- 本地入口从可信注册表选定 binding 并建立 `Session`。每次调用 `Session::invoke` 都分配新的 request ID，校验参数并重新读取该 binding 的一致快照。仓储返回已撤销、已删除、失效或已换项目的记录时，调用立即失败。
- 能力模板中的 `<id>` 在具名任务上展开为任务 UUID，在远端操作上展开为资源 UUID。授权还要精确匹配 binding 和资源所属 target。稳定 ID 用 UUID 表示；工具参数中的 `taskId`、`target`、`resource`、`profile` 是逻辑别名，在可信作用域内解析。
- 别名长度为 1–64 个 ASCII 字符，首字符为小写字母，其余字符允许小写字母、数字、下划线和连字符。`database_schema.table` 可省略以请求允许的表清单。提供时采用 1–64 字符的单个表选择符。显式传入 `null` 会失败。
- 目标别名支持 `dev`、`test` 及用户注册的自定义值。环境类别与风险等级由本地记录决定。八项工具均拒绝未知字段和额外的身份、路径、端点或授权参数。
- `database_query.sql` 为 1–8192 个 Unicode 码点，至少包含一个非空白字符。T02 只检查输入形状和长度。T12 在向 MySQL 发出查询前负责 AST、表列权限、结果脱敏、行数、字节数和超时。

## 结果与退出码

[report.schema.json](report.schema.json) 定义公共外层，[cli.json](cli.json) 为七种退出码分别保存人类文本和 JSON。适配器写 JSON 时序列化整个 Report，按 `Report::exit_code()` 退出。样例中的时间、ID、耗时均为测试数据。

| CLI 退出码 | 语义 | 代表性错误 |
|---|---|---|
| 0 | 用例正常返回 | `ok: true`，结果见 `result` |
| 2 | 输入无效 | `INVALID_INPUT` |
| 3 | 信任、授权或策略拒绝 | `POLICY_DENIED`、`BINDING_REVOKED`、`TARGET_NOT_ALLOWED` |
| 4 | 依赖不可用 | `TOOLCHAIN_MISSING`、`CREDENTIAL_UNAVAILABLE` |
| 5 | 连接失败 | `NETWORK_UNREACHABLE` |
| 6 | 超时 | `TIMEOUT` |
| 7 | 内部错误 | `INTERNAL_ERROR` |

`schemaVersion` 为整数 1，`timestampUtc` 使用以 `Z` 结尾的 RFC 3339 时间，`durationMs` 为非负整数。每个响应均含 `requestId`。成功执行 `task_run`、`connection_test` 和 `database_query` 时，执行器须返回可定位记录的 `evidenceId`；已开始执行的失败也可携带该 ID。执行前拒绝时只生成请求编号。任务的 `result.exitCode` 来自子进程，CLI 退出码描述用例调用结果。

未知工具的 `operation` 统一为 `unknown`。失败信息使用分类和固定诊断文本，避免回显输入。授权完成前的响应省略解析后的资源能力；本地路径、主机、凭据引用和原始凭据留在适配器内部。

## 复跑与迁移

从仓库根目录运行以下命令。Rust 集成测试通过 [application 清单](../../crates/application/Cargo.toml) 注册到 workspace，JSON Schema 测试由根 `pnpm unit` 调用。

```sh
pnpm install --frozen-lockfile
node --test tests/contracts/schemas.test.mjs
cargo test --locked -p envlume-domain -p envlume-application
```

这是首个契约版本，T01 的 `envlume --version` 输出保持兼容。T03 将实体映射到 SQLite；T04 先消费 `project_context`；T13 已将八项工具接到共用服务。修改字段、错误码或能力映射时，同时更新 JSON Schema、正反样例、Rust 类型和消费者说明。变更已有接口兼容性时按架构 §20 记录 ADR、版本与旧消费者的迁移办法。

### 八项工具的真实服务验收

在 macOS 上使用本机 Node、npm、Rust、Docker 和可用的 Keychain，运行 `python3 -B tests/contracts/live_mysql.py`。脚本启动仅监听回环的 MySQL 8.4、创建两个临时只读账号和工作区，调用真正的 CLI 分发器、STDIO 服务和任务子进程。测试宿主只向 CLI 指定临时存储目录。加 `--snapshot <新文件>` 可保存本轮脱敏样本；临时账号、凭据条目和 SQLite 库随测试清理。跨平台协议、授权回归与正式 CLI 的启动检查使用工作区测试和原生 CI。[端到端断言](../security/live-contracts.mjs)
