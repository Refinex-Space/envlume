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

### 2026-11-26 · 注册与授权边界

**T05-A01 至 A03 通过。** 验收修订 `73c3102e4d7170174c723cc903481934fe1a80fc`，同一修订的三个原生作业和供应链检查全部通过。[完整 CI](https://github.com/Refinex-Space/envlume/actions/runs/45814893694)

| 验收 | 原生实测结果 |
|---|---|
| T05-A01 · 导入与隔离 | 两个同名、同 remote 的仓库分别生成 project ID 与 binding ID；真实 linked worktree 可用 `--project` 归入同项目并取得独立 binding ID。真实终端确认后记录 `LOCAL_CLI_TTY`，`pom.xml` 给出 Maven 提示。首次导入时管道输入 `trust` 被拒绝。 |
| T05-A02 · SEC-001、002 | 正式 CLI 的独立 MCP 进程返回当前项目、workspace 别名、可见目标和 capability 摘要。未知 ID、错误启动根目录均被拒绝。经另一 CLI 进程撤销后，同一会话下一次调用返回 `BINDING_REVOKED`；项目软删除后其他绑定也失效，另一项目仍可读取。 |
| T05-A03 · SEC-008 | 两台 macOS runner 的 APFS 上验证符号链接替换，Windows Server 2025 x64 的 NTFS 上验证本地 junction 替换。三个卷均实测大小写别名复用原 ID，移动及同路径换目录后旧权限失效，再确认产生新 ID。原生 Windows 额外通过普通及扩展 UNC、设备名和路径分量负例；测试未连接 SMB 共享。[Apple Silicon](https://github.com/Refinex-Space/envlume/actions/runs/45814893694/job/139891952400) · [Intel macOS](https://github.com/Refinex-Space/envlume/actions/runs/45814893694/job/139891952474) · [Windows x64](https://github.com/Refinex-Space/envlume/actions/runs/45814893694/job/139891952482) |

用户在本地终端预览所选入口、规范目录、Git 类型、构建文件提示及申请的 `project.inspect` 权限，输入 `trust` 完成首次确认。schema v3 记录所选路径、目录身份、确认来源 `LOCAL_CLI_TTY` 和 UTC 毫秒时间。确认前后及写入事务中均复核目录。已有项目可通过 `--project <id>` 显式关联其他 worktree；同名目录或同一 remote 保持独立绑定。Git 元数据只作发现线索，gitfile 在 8 KiB 内读取，其指向的外部路径不会被打开。[CLI 用法](../../../README.md#注册工作区并验证-mcp)

| 事件 | 绑定及授权结果 |
|---|---|
| 首次导入，取消、管道输入或确认期间目录被替换 | 拒绝新增绑定和授权。 |
| 显式确认新的目录身份 | 新随机 binding UUID，状态 `TRUSTED`，仅有一项 `project.inspect` grant。重复导入未变的绑定返回原 ID，保留当时的授权状态。 |
| 启动目录与注册根不符，或传入其他工作区的 binding ID | 拒绝本次调用，原工作区绑定继续有效。启动上下文取自本地进程；协议 metadata、仓库配置和工具参数不能指定注册表或重绑身份。 |
| 注册目录移动、消失、原路径换目录或已批准的链接改指 | 观察到变化时把旧绑定置为 `STALE` 并撤销关联 grant。登记时的目录对象身份、创建时间、规范路径及 Git 指纹共同参与复核。旧库中缺少可验证确认来源的绑定同样需要重新确认。 |
| 撤销绑定或软删除项目 | 在一个 SQLite 事务内标记 `REVOKED` 并撤销关联 grant；软删除覆盖该项目的所有绑定。已经启动的 MCP 会话在下一次发现或调用时重新查验。 |
| 对失效或已撤销的目录再次确认 | 新 binding ID；旧进程持有的 ID 继续被拒绝。 |

**路径决定。** T05 支持本地磁盘。在 Windows 上，UNC、扩展 UNC、设备路径及映射网络盘在访问工作区内容前拒绝，避免隐式 SMB 认证。本地 junction 的每级目标均须是本地路径。大小写由文件系统决定；规范路径及卷内对象身份用于识别别名。调用方提供的 `..` 路径分量一律拒绝。

**复跑。** `cargo test --locked -p envlume --test workspace_trust` 在临时目录创建两个同 remote 仓库及真实 linked worktree，通过伪终端执行确认，用独立 STDIO 进程保持会话并从另一 CLI 进程撤权。CI 在一次性用户配置中对编译出的正式 `envlume` 再运行同一套测试。实际 trace 记录见原生作业的 `Verify real workspace trust and live revocation with the installed CLI` 步骤。[子进程客户端](../../../tests/workspace/trust.mjs)

`cargo test --locked -p envlume-storage --test trust_transactions` 验证确认期间替换、同路径替换、旧记录、撤销和删除失败回滚、重复导入及真实授权摘要。后者覆盖同别名资源、未授权目标、PROD 和停用目标、过期 grant，以及返回值的端点与凭据引用检查。`cargo test --locked -p envlume-workspace --test discovery` 单独验证路径和 gitfile 边界。[事务测试](../../../crates/storage/tests/trust_transactions.rs) · [路径测试](../../../crates/workspace/tests/discovery.rs)

**质量门。** 本机 macOS 26.5.2 arm64 与 CI 使用 Rust 1.99.1、Node 24.14.1、pnpm 10.33.0。锁定安装、lint、typecheck、unit、Vite build、Cargo fmt/clippy/test/build 和 Tauri 开发构建均通过。六项存储信任测试与独立 CLI/STDIO 验收通过。首轮 Windows 伪终端因未回答 ConPTY 光标位置查询而超时，修正测试终端应答后，同一套交互断言通过。依赖许可证、秘密扫描及 `cargo audit` 通过，保留三条既有 RustSec 警告。[安全作业](https://github.com/Refinex-Space/envlume/actions/runs/45814893694/job/139891952042)

**交接。** T06 可沿已确认 binding 的 `project_id` 和 `profile_id` 接入工具链。真实 `project_context` 已通过本地注册表、工作目录和授权快照产生；直接 STDIO 启动不会伪认 Claude Code、Codex 或 Cursor 身份。三种客户端配置文件的安装、来源指纹和回滚仍由 T15–T18 接入。
