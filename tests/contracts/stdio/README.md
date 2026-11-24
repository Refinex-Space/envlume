# T04 STDIO 契约

从仓库根目录运行下列命令。测试需要项目锁定的 Node.js 和 Rust 工具链。

```sh
cargo test --locked -p envlume --test stdio
```

测试客户端启动独立子进程，通过换行分隔的 JSON-RPC 帧交换消息。运行目录和 SQLite 注册表位于一次性临时目录。服务器进程仅使用 stdin 和 stdout 传输协议，诊断写入 stderr。macOS 用 `pgrep` 和 `lsof` 检查进程及网络套接字，Windows 用 CIM 和 `netstat` 检查子进程及 TCP 端口。

## 两种启动环境

| 程序 | 注册来源、用例及预期行为 |
|---|---|
| 常规 `envlume mcp --binding <UUID>` | T04 的生产 resolver 默认拒绝绑定。合法格式但未经解析的 UUID 在握手时返回 `PROJECT_NOT_TRUSTED`，退出码为 3。真实工作区注册与解析由 T05 接入。 |
| 测试 CLI 子进程 | 编译同一 CLI 参数入口和同一 rmcp 服务，注入临时 SQLite 仓储及 T02 的 application 替身。它核对测试绑定，发现并调用 `project_context`。测试 client 身份由本地夹具预置。 |

测试客户端会分别调用两个程序，检查未知绑定在两个版本的握手中均收到结构化拒绝。常规 CLI 的运行参数、工作目录或客户端 metadata 均不能启用替身。

## 协议与证据

| 场景 | 记录 |
|---|---|
| 架构参考版本 `2026-07-28` | 发送 `server/discover` 获取支持的版本，再以逐请求 metadata 发起 `tools/list` 和 `tools/call`。客户端提交未支持的版本时收到 `-32022`，可继续用支持的版本调用。 |
| 旧版 `2025-11-25` | 发送 `initialize`，检查协商版本，再发送 `notifications/initialized`、工具发现和调用。 |
| 身份与权限 | 多传 `projectId`、`bindingId` 或 `cwd` 返回 `INVALID_INPUT`。运行中撤销 `project.inspect` 后，发现列表为空，直接调用返回 `POLICY_DENIED`。 |
| 传输与清理 | 比对每个响应的 JSON-RPC ID 和完整 stdout 帧数，检查 stderr 不含测试标记和本机路径。关闭 stdin 后等待进程退出；错误握手也检查退出码和脱敏诊断。 |

[实测 transcript](transcript.jsonl) 包含测试进程参数、按会话排列的输入输出帧、stderr 与退出码。请求编号、UUID、UTC 时间和耗时来自记录当次运行；测试比较稳定契约字段。要另存一次新的运行记录，可在运行测试时将 `ENVLUME_TEST_TRANSCRIPT` 设为一个尚不存在的绝对文件路径，该变量只由测试客户端读取。[T02 输入与权限](../tools.json) · [T02 预期摘要](../examples.json)
