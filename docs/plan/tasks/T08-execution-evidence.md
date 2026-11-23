# T08 贯通任务证据与本地诊断

Phase 1 · 项目与工具链　·　[返回交付总表](../PLAN.md)

| 开工条件 | 内容 |
|---|---|
| 顺序前置 | [T07 实现具名任务、首次确认和进程控制](T07-named-task-runner.md) |
| 架构与追踪 | [架构基线](../../architecture/ARCHITECTURE.md) §1.3、§8.2、§12、§15 · US-03、US-07、US-09 · E2E-04、E2E-05、E2E-12、SEC-009 |
| 工作范围 | `crates/audit/`、execution、application、storage、CLI/MCP 任务与报告入口 |

## 交付目标

让实际构建、失败和拒绝都可追溯，CLI 与 MCP 可凭同一请求核对执行结果。

## 实施与产出

1. 在 application/audit 汇合 T05–T07。CLI 和 MCP 接入 `toolchain_doctor`、`task_list`、`task_run` 对应入口，沿用 T02 契约。CLI 提供本地按 evidenceId 查看脱敏报告的路径。
2. 生成 requestId，执行动作关联 evidenceId。保存主体、工作区/目录摘要、任务、目标（如适用）、实际版本、命令摘要、时间、耗时、退出码和断言结果。保留限量日志，定义保留期限与清理能力。
3. 将本机、工具链、工作区、策略和工作流诊断连到可操作的下一步。区分 Success、Failed、Denied、Skipped，缺工具和取消不能显示为测试通过。

## 验收

- **T08-A01**　导入真实 Java 8/Spring Boot 测试项目，运行 `mvn test`，对照独立测试报告核验退出码、耗时、版本和 evidenceId。再使一个断言失败，核对同一链路如实记录失败。
- **T08-A02**　在 Node 构建、未授权执行及超时场景中，CLI JSON 和 MCP 返回的请求可在本地关联，跨绑定不能读取另一份记录。
- **T08-A03**　SEC-009 用假秘密覆盖命令错误、stdout/stderr、持久化、报告及清理路径，检查摘要经过脱敏和截断。实际传给 Agent 的内容限于授权范围。

## 交接

移交 Java/Node 两个可复跑场景、成功与失败报告及脱敏扫描结果。

## 执行记录

待执行。收尾时登记修订、平台/版本、实际命令或操作、对应验收编号、结果与脱敏证据位置。
