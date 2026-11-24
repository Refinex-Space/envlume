# T04 打通独立 STDIO 进程

Phase 0 · 工程与契约　·　[返回交付总表](../PLAN.md)

| 开工条件 | 内容 |
|---|---|
| 顺序前置 | [T03 建立 SQLite 存储、迁移与恢复](T03-sqlite-and-shared-state.md) |
| 架构与追踪 | [架构基线](../../architecture/ARCHITECTURE.md) §3.2–3.3、§5、§8.1–8.3、§9.2、§18 Phase 0 · SEC-010 |
| 工作范围 | `crates/mcp/`、`crates/cli/`、STDIO 契约测试 |

## 交付目标

用真实 STDIO 子进程贯通协议协商、工具发现和模拟 project_context。

## 实施与产出

1. CLI 增加 `envlume mcp --binding <id>`，采用 rmcp 协商协议，把日志分流至 stderr。冻结的协议参考版本与实际协商结果分别记录。
2. 以临时数据目录中的测试绑定和 application 替身返回模拟项目摘要。模拟数据限定在测试环境。默认运行遇到无效绑定时拒绝。T05 替换为真实 resolver。
3. 建立可重复启动和关闭子进程的契约客户端，解析完整 stdout 流，检查握手、tools/list、tools/call、错误和断开清理。

## 验收

- **T04-A01**　桌面关闭时从 CLI 独立启动，测试客户端发现 project_context 并收到指定 fixture。未知 binding 或试图覆盖身份的参数触发结构化拒绝。
- **T04-A02**　SEC-010 同时触发普通日志、输入错误和正常返回，逐帧解析 stdout，无非协议文本。关闭 stdin 后进程退出。
- **T04-A03**　检查进程和监听情况，首版没有后台 daemon 或管理 HTTP 端口，头部协议协商失败时 stderr 给出已脱敏的诊断。

## 交接

移交协议 transcript、协商版本和子进程测试命令，标明当前数据源为 fixture。

## 执行记录

### 2026-11-24 · 本地验收

实现修订 `a7b9c80`，基于 T03 的 `29f3acc`。环境为 macOS 26.5.2 arm64、Rust 1.99.1、Node 24.14.1，官方 Rust SDK 锁定为 `rmcp 3.11.0`。复跑命令为 `cargo test --locked -p envlume --test stdio`。[测试环境与复跑说明](../../../tests/contracts/stdio/README.md) · [逐帧实录](../../../tests/contracts/stdio/transcript.jsonl)

| 验收 | 实际操作与结果 |
|---|---|
| T04-A01 | 在临时 SQLite 注册表预置一个测试 binding，以独立 CLI 测试子进程运行共用入口。客户端在发现列表中仅收到 `project_context`，调用返回 T02 夹具的 `orders / orders-main / dev / project.inspect`。额外传入 `projectId`、`bindingId` 或 `cwd` 均返回 `INVALID_INPUT`。撤销 grant 后列表为空，直接调用同一工具得到 `POLICY_DENIED`。常规二进制与测试子进程均在两种握手流程中对未知绑定返回 `PROJECT_NOT_TRUSTED`。 |
| T04-A02 · SEC-010 | 测试客户端解析完整 stdout，包括末尾换行及退出前所有字节。普通启动日志和握手诊断出现在 stderr；正常结果、输入错误、工具拒绝都有可解析的响应帧。两次成功会话关闭 stdin 后退出码为 0。 |
| T04-A03 | 会话活跃期间使用 `lsof` 检查网络套接字、用 `pgrep` 检查子进程，均无结果。缺少必需 metadata 的请求收到协议错误，错误首部序列以退出码 2 结束，stderr 诊断未回显测试标记及本机路径。 |

**协议版本。** 架构参考版本 `2026-07-28` 已用 `server/discover` 和逐请求 metadata 实测；未支持的版本收到 `-32022` 后可继续使用参考版本。旧版 `initialize` 实测协商为 `2025-11-25`。以上结果来自仓库契约客户端，Claude Code、Codex 和 Cursor 的实际兼容性分别在 T16–T18 验收。

**质量门。** `cargo fmt --all --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings`、`cargo test --workspace --locked`、`cargo build --workspace --locked` 均通过。前端锁定安装、lint、typecheck、unit 和 build 通过。`cargo deny --locked check licenses` 通过。`cargo audit` 退出 0，保留此前登记的三条通告，其中 `chrono` 的维护状态通告现在也涉及 CLI 的 rmcp 依赖链。

**交接。** 生产入口默认拒绝绑定，成功的项目摘要仅由测试注册表及 application 替身提供。R0 复跑 T01–T04，并核对同一修订的三目标编译证据；T05 再接真实工作区解析。本节的平台结果仅覆盖 Apple Silicon。
