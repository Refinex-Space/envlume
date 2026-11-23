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

待执行。收尾时登记修订、平台/版本、实际命令或操作、对应验收编号、结果与脱敏证据位置。
