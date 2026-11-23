# T23 增加 Redis/Valkey 只读检查

Phase 4 · v0.2 资源扩展　·　[返回交付总表](../PLAN.md)

| 开工条件 | 内容 |
|---|---|
| 顺序前置 | [R3 裁决并交付 v0.1 MVP](R3-mvp-release.md) |
| 架构与追踪 | [架构基线](../../architecture/ARCHITECTURE.md) §7.1、§7.3、§14、§18 Phase 4 · US-10 |
| 工作范围 | `crates/connector-redis/`、registry、policy、MCP 与真实 Redis/Valkey 测试 |

## 交付目标

在同一套绑定、凭据、目标授权和证据链中增加受限 Redis/Valkey 检查。

## 实施与产出

1. 用原生驱动接入已登记端点、TLS、鉴权和数据库编号。定义实例能力、命令及 key pattern 矩阵，配置服务器 ACL 与客户端允许集合。按真实需求单列 Cluster/Sentinel 支持范围。
2. 实现 PING、有限 INFO 字段、有分页/频率上限的 SCAN 及已授权 key 的 GET/TYPE/TTL。注册前完成输入、数量、字节、超时、并发、误连、TLS 与凭据脱敏验证。

## 验收

- **T23-A01**　用两个目标及不同命名空间测试真实实例。受权 key 可读，越界 key、未批准命令和写入均拒绝。拒绝与超时可按 requestId 复核。
- **T23-A02**　游标分页有终点/上限，超大值截断或拒绝，日志无凭据或批量 key 内容。矩阵通过后才将新工具加入可见目录，原八项契约回归通过。

## 交接

移交驱动/服务版本、ACL 与权限矩阵、新工具 schema 以及真实查询/拒绝证据。

## 执行记录

待执行。收尾时登记修订、平台/版本、实际命令或操作、对应验收编号、结果与脱敏证据位置。
