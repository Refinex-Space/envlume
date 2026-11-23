# T24 增加 Elasticsearch 只读检索

Phase 4 · v0.2 资源扩展　·　[返回交付总表](../PLAN.md)

| 开工条件 | 内容 |
|---|---|
| 顺序前置 | [T23 增加 Redis/Valkey 只读检查](T23-redis-inspect.md) |
| 架构与追踪 | [架构基线](../../architecture/ARCHITECTURE.md) §7.1、§7.4、§14、§18 Phase 4 · US-10 |
| 工作范围 | `crates/connector-elasticsearch/`、registry、policy、MCP 与版本测试 |

## 交付目标

对已批准的 ES 索引提供版本可识别、有界且脱敏的只读查询。

## 实施与产出

1. 通过 reqwest 使用 HTTP(S) 官方接口，凭据经 provider 取得。根据测试过的 ES 版本建立支持矩阵，明确 7/8/9 等版本的探测结果及未验证项。
2. 开放 cluster health、授权 index metadata/mapping 和受控 _search。限制 DSL 深度、索引、_source 字段、结果规模及超时。禁用脚本、更新、删除类操作，沿用注册端点和 TLS 策略。

## 验收

- **T24-A01**　各声明支持的版本均有真实环境测试，未支持版本明确诊断。相同别名的另一目标、非授权索引或字段不会泄露。
- **T24-A02**　验证深层 DSL、超大响应、超时取消、错误证书、损坏凭据和恶意输入。检查账号权限、限流、审计与脱敏后，才注册 MCP 工具。

## 交接

移交 ES 实测兼容矩阵、权限表、schema、正反例及真实服务证据。

## 执行记录

待执行。收尾时登记修订、平台/版本、实际命令或操作、对应验收编号、结果与脱敏证据位置。
