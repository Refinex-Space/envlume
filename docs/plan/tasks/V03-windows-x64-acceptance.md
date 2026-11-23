# V03 验收 Windows 11 安装与恢复

Phase 3 · v0.1 MVP　·　[返回交付总表](../PLAN.md)

| 开工条件 | 内容 |
|---|---|
| 顺序前置 | [V02 验收 Intel macOS 安装与 MCP](V02-macos-intel-acceptance.md) |
| 架构与追踪 | [架构基线](../../architecture/ARCHITECTURE.md) §4.3、§6.3、§10.2、§11、§16、§19 · E2E-03、E2E-11 |

## 交付目标

在干净 Windows 11 x64 用户环境验证 NSIS、WebView2、Credential Manager 和独立 MCP。

## 实施与产出

1. 核对候选包哈希、架构和签名状态。验证安装、卸载、重装、升级及 WebView2 已存在/缺失两条路径。记录向用户显示的处理方式与离线行为。
2. 用包含空格及非 ASCII 的项目目录跑客户端、Maven/Node、MySQL 和本地审计，核对 Windows ACL、Credential Manager 及配置路径转义。在运行中的 CLI 上演练更新/回退。

## 验收

- **V03-A01**　E2E-03、E2E-11 在 Windows 11 x64 真正执行。关闭窗口后调用 MCP，回显正确项目与证据，安装前后 PATH 和 JAVA_HOME 未被静默修改。
- **V03-A02**　升级/恢复后绑定、资源、凭据引用、第三方客户端配置完整。junction/UNC、取消后的子进程清理、WebView2 缺失各有实测记录或需处理的阻塞。

## 交接

移交 Windows 11 x64 原生验收记录和安装器日志摘要。Windows 10 x64 按尽力兼容单列，Windows arm64 和 Linux 不列入首发支持。

## 执行记录

待执行。收尾时登记修订、平台/版本、实际命令或操作、对应验收编号、结果与脱敏证据位置。
