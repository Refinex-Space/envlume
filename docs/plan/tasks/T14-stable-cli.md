# T14 交付可独立定位和升级的 CLI

Phase 3 · v0.1 MVP　·　[返回交付总表](../PLAN.md)

| 开工条件 | 内容 |
|---|---|
| 顺序前置 | [R2 验收真实 MySQL 与授权隔离](R2-mysql-policy-review.md) |
| 架构与追踪 | [架构基线](../../architecture/ARCHITECTURE.md) §3.2、§10.5、§11.1、§16.1–16.2 · US-08 · E2E-11 |
| 工作范围 | CLI 构建与定位、Tauri 打包布局、兼容检查和更新/恢复测试 |

## 交付目标

让项目 MCP 配置能引用稳定的 CLI 绝对路径，关闭桌面或升级应用后仍可启动。

## 实施与产出

1. 为桌面包提供同版本、匹配目标架构的独立 CLI，若用 externalBin，校验每个 target triple 命名。由平台目录 API 定位用户可执行路径，路径含空格和非 ASCII 时仍可启动。
2. 实现版本/数据兼容检查、原子安装替换和上一可用版本恢复。处理 Windows 正在使用的可执行文件、复制中断和不可写目录。把处理结果反馈给调用者。
3. 给集成层提供已验证的绝对路径及版本。PATH 加入为用户可选，项目生成的启动配置在默认 PATH 精简时也能工作。

## 验收

- **T14-A01**　用三个架构的开发包或暂存安装布局验证 `--version` 和 STDIO 调用，桌面关闭、工作目录不同或无开发环境 PATH 时均可运行。
- **T14-A02**　从旧 CLI 更新到同一稳定入口，尝试中断复制、目标占用和不兼容 schema，仍保留可恢复版本与数据。已有项目配置无需人工更改。

## 交接

移交 CLI 定位/更新接口及原生证据。正式签名安装包上的 E2E-11 在 V01–V03 再验。

## 执行记录

2026-11-27，实施修订 `f54a42d`。本机 macOS arm64 使用 Tauri 暂存布局、隔离用户目录以及从产品 CLI 源码另行编译的旧版验收构建 `0.0.0-t14.1`。当前版本为 `0.1.0-dev.1`，支持目录库 schema 9。[本机结果](../evidence/T14-cli.json) · [原生复验脚本](../../../tests/installation/stable.mjs)

| 验收 | 检查与结果 |
|---|---|
| T14-A01 | 在三个原生目标的开发宿主中嵌入对应 CPU 的 CLI。定位返回稳定绝对路径、版本、架构、最高 schema 和 SHA-256。空 PATH 下实调 `--version`、STDIO 握手及 `project_context`。含中文与空格的暂存目录与用户级可执行路径可用。从其他目录启动版本查询成功，用错误目录冒用已有绑定得到 `PROJECT_NOT_TRUSTED`。 |
| T14-A02 | 同一份 MCP 启动配置先后用于旧版、升级、恢复。macOS 升级时已打开的旧会话仍能调用；Windows 占用时返回 `BUSY`，旧会话仍可用，关闭后重试。覆盖复制途中读错、摘要或架构错误、临时文件遗留、收据先于重命名提交、并发更新、不可写目录和恢复时 schema 超前。目录库与上一可用版本均有独立断言，在线备份实查包含已提交的 WAL 数据。 |

复跑 `cargo test --locked -p envlume-storage cli_install`，再执行下列命令。Windows 的平台目录来自系统 Known Folder；在一次性 Windows 用户配置中设置 `T14_DISPOSABLE_PROFILE=1` 运行集成测试。CI 对每个目标传对应的原生桌面构建路径。本机空 PATH 结果在暂存安装布局取证。

```sh
pnpm desktop build --debug --no-bundle --target aarch64-apple-darwin -- --locked
node tests/installation/stable.mjs target/aarch64-apple-darwin/debug/envlume-desktop
```

**移交 T15。** 使用 `envlume_storage::cli_install::locate(&paths)` 返回的 `InstalledCli.path` 和 `release.version`。原生维护入口为桌面宿主 `--cli install|locate|restore`。客户端配置只存稳定绝对路径和独立参数，绑定校验仍由现有应用服务完成。正式签名 DMG、NSIS 安装包的 E2E-11 在 V01–V03 验收。
