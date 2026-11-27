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

2026-11-27，事务实现修订 `7f019d3`，SQLite schema 10。本机 macOS arm64 使用隔离目录和合成凭据。桌面开发包、安装后的独立 CLI 和真实 STDIO 服务参与验收，客户端发现使用独立 PATH 夹具。[本机脱敏结果](../evidence/T15-integrations.json) · [原生入口脚本](../../../tests/installation/integration.mjs)

| 验收 | 实际检查 |
|---|---|
| T15-A01 / SEC-011 | 对 JSON 和 TOML 保留第三方服务、未知字段、JSON 长整数及 TOML 注释。重复安装保持原内容，正常卸载精确还原原始字节。模拟写入部分暂存文件、发布后中断、删除提交前后中断，以及原文件已被置换但新文件尚未发布。重启后按 SQLite 意图恢复。客户端适配器拒绝验证时回滚；独立编辑的第三方值保留。 |
| T15-A02 | 损坏配置、同名外部条目（含启动描述完全相同）均拒绝认领。取消确认不产生配置变更。确认期间发生外部编辑时返回 CONCURRENT_EDIT；验证期间有人改动管理条目时保留其编辑并返回 RECOVERY_CONFLICT。原生负面用例涵盖符号链接、Windows junction 和文件占用。 |
| T15-A03 | 实调 CLI install/verify/uninstall 与桌面原生 `--integration`，确认 stdout 是单个 JSON。无客户端返回 4，取消返回 3，未知参数返回 2。普通验证返回 CONFIGURED、gatewayVerified=true、clientVerified=false。CLI 输出、预览及诊断不含第三方测试秘密；私有备份与原件逐字节相同。Unix 检查权限位，Windows 实查备份 ACL。 |

复跑 `cargo test --locked -p envlume-clients`。三目标 CI 在 T14 的暂存安装测试中同时运行 T15 原生入口脚本。本机可运行 `pnpm desktop build --debug --no-bundle --target aarch64-apple-darwin -- --locked`，再运行 `node tests/installation/stable.mjs target/aarch64-apple-darwin/debug/envlume-desktop`。设置 `T15_TEST_SNAPSHOT` 为新文件路径可记录本轮摘要。

**移交 T16–T18。** 通过 application 的 `Adapter` 传入客户端发现与核验行为，共用 `envlume_clients::Service::execute` 完成确认、登记、发布和恢复。管理指纹只覆盖严格解析的 command/args，整文件指纹覆盖原始字节；原件缺失单独记录。真实客户端的版本、项目加载、信任和重载结论在各自任务实测。[接口和恢复约定](../../architecture/adr/ADR-0018.md)

Windows 首轮通过事务与 ACL 回归。原生交互脚本在安装确认时返回 CLIENT_UNAVAILABLE；portable-pty 的 Windows 环境构造会用注册表 PATH 覆盖父进程值。验收宿主现为集成确认显式传递同一隔离 PATH，继续检查无客户端返回 4、取消返回 3 及真实网关验证。
