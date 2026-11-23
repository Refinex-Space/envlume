# T02 冻结领域模型与入口契约

Phase 0 · 工程与契约　·　[返回交付总表](../PLAN.md)

| 开工条件 | 内容 |
|---|---|
| 顺序前置 | [T01 建立工作区和三目标构建](T01-workspace-ci.md) |
| 架构与追踪 | [架构基线](../../architecture/ARCHITECTURE.md) §1.3、§4、§5.1–5.2、§8、§9、§10.4、§12、§15 · US-01、US-09 |
| 工作范围 | `crates/domain/`、`crates/application/`、`tests/contracts/` |

## 交付目标

让三个入口使用同一组实体、授权上下文和结果类型，先固定可验证的输入输出。

## 实施与产出

1. 在 domain 定义 Project、WorkspaceBinding、ToolchainProfile、EnvironmentTarget、ResourceBinding、TaskDefinition、CapabilityGrant、ExecutionRecord、ClientIntegration，区分稳定 ID、逻辑别名和路径，明确生命周期及引用关系。
2. 在 application 定义按可信来源建立的 RequestContext、逐请求授权点、类型化用例和 repository/secret/connector/toolchain 接口。使用内存替身先验证拒绝路径，冻结每个操作对应的 capability。
3. 在 tests/contracts 固定 §8.2 八项工具的输入、返回与权限，归档 CLI 人类/JSON/退出码和统一结果样例。日期用 UTC，错误有分类。协议变更同时更新测试、迁移说明和消费者文档。

## 验收

- **T02-A01**　两个工作区可属于同一项目，不同目标可有同名资源。领域测试证明切换环境不改变 ToolchainProfile，项目软删除能表达绑定及授权撤销。
- **T02-A02**　用表驱动用例核对八项工具的必填字段、长度、枚举和 `additionalProperties: false`。多余身份字段、自由路径、原始连接地址被拒绝。此阶段可用契约 fixture 执行。
- **T02-A03**　CLI 结果含 `schemaVersion`，受控操作含 `requestId`，执行结果可关联 `evidenceId`。错误样例覆盖输入、拒绝、缺依赖、连接、超时、内部失败。

## 交接

交付领域类型、接口和可执行契约样例。T03 实现持久化，T04 先接一项模拟工具，T13 接齐八项。

## 执行记录

待执行。收尾时登记修订、平台/版本、实际命令或操作、对应验收编号、结果与脱敏证据位置。
