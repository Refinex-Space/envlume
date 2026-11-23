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

待执行。收尾时登记修订、平台/版本、实际命令或操作、对应验收编号、结果与脱敏证据位置。
