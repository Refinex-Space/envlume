# T15 实现配置预览、合并与回滚

Phase 3 · v0.1 MVP　·　[返回交付总表](../PLAN.md)

| 开工条件 | 内容 |
|---|---|
| 顺序前置 | [T14 交付可独立定位和升级的 CLI](T14-stable-cli.md) |
| 架构与追踪 | [架构基线](../../architecture/ARCHITECTURE.md) §4.5、§11.2–11.4、§12、§14.2 · US-08 · SEC-011 |
| 工作范围 | `integrations/clients/`、application、client_integrations 仓储与 CLI |

## 交付目标

为三个客户端提供一套可恢复的安装事务，并让 CLI 和桌面调用同一流程。

## 实施与产出

1. 在 integrations/clients 与 application 定义 detect / preview / install / verify / uninstall。记录 client、binding、作用域、位置、配置指纹与状态。生成稳定 CLI 绝对路径和 binding 参数。
2. 按 JSON/TOML 结构解析，预览仅 Envlume 管理的改动。修改前备份、检测并发编辑、同文件原子替换。处理损坏文件、同名条目冲突和用户改过的条目，保留已有内容。
3. 实现 CLI integration install/verify/uninstall 的路由、交互确认与机器输出。用无副作用的 project_context 对照预期绑定后标为已接入。客户端拒绝、验证失败时安全恢复。卸载只清理本次登记的条目。

## 验收

- **T15-A01**　SEC-011 用含第三方服务/未知字段的 JSON 和 TOML 夹具验证重复安装、卸载、写入中断、验证失败、并发编辑与还原，第三方数据逐项保持一致。
- **T15-A02**　损坏配置和命名冲突有明确诊断，用户可取消预览，文件保持原样。对已被外部修改的内容，恢复不会覆盖用户新编辑。
- **T15-A03**　CLI `--json` 和退出码符合 §12。无可用客户端或无法完成验证时，状态与原因如实显示。生成的条目不含 Envlume 凭据。用含第三方假凭据的原配置验证备份权限、恢复保真，以及预览和日志的脱敏。

## 交接

移交两类格式的生命周期测试、配置指纹语义和 CLI 接口，后续适配器分别补真实客户端证据。

## 执行记录

待执行。收尾时登记修订、平台/版本、实际命令或操作、对应验收编号、结果与脱敏证据位置。
