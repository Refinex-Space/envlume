# T05 实现项目导入、绑定和信任撤销

Phase 1 · 项目与工具链　·　[返回交付总表](../PLAN.md)

| 开工条件 | 内容 |
|---|---|
| 顺序前置 | [R0 验收骨架与契约](R0-foundation-review.md) |
| 架构与追踪 | [架构基线](../../architecture/ARCHITECTURE.md) §4.2–4.5、§9.1–9.3、§12、§14 · US-01 · SEC-001、SEC-002、SEC-008 |
| 工作范围 | `crates/workspace/`、application、storage、CLI/MCP 项目入口与原生路径测试 |

## 交付目标

把真实工作目录注册为独立 Workspace Binding，确保移动、撤销和多 worktree 的行为可预测。

## 实施与产出

1. 实现 `project add/list/inspect` 及相应 application 用例，解析规范路径、Git/worktree 元数据及技术栈线索。显式本地确认后生成并信任随机绑定，记录确认来源。非交互且需确认时拒绝。
2. 区分同项目的多个 worktree、同名目录和同 remote 的不同仓库。处理不存在、大小写别名、symlink、UNC 和 junction。移动、重命名或真实路径变化转为 STALE，重新确认后才可使用。
3. 接入真实 project_context。启动校验本地注册记录、配置来源和路径，每次调用重查绑定。支持撤销及项目软删除，事务内撤销关联权限，已启动的 MCP 同样生效。

## 验收

- **T05-A01**　导入两份共用 remote 的测试目录，得到独立绑定和信任状态。首次导入识别构建文件，只显示技术栈建议。
- **T05-A02**　SEC-001、SEC-002 从独立进程证明未绑定、运行中撤销或删除项目后调用失败。真实 project_context 只返回当前授权摘要。
- **T05-A03**　SEC-008 在 macOS 和 Windows 用临时目录验证路径越界、链接替换、移动及重新绑定。记录 UNC、junction 和大小写的原生结果及使用的文件系统。

## 交接

移交绑定状态转换表、原生路径用例、导入与撤销的 CLI/MCP 记录。

## 执行记录

待执行。收尾时登记修订、平台/版本、实际命令或操作、对应验收编号、结果与脱敏证据位置。
