# Envlume 从零到发布的交付计划

**首个交付终点是 R3 的 v0.1 MVP。** 开发者在干净的 macOS 或 Windows 11 上安装 Envlume，导入已有 Java 项目，为工作区选定 JDK/Maven，授权 DEV MySQL，并接入 AI 客户端。关闭桌面后，Agent 仍能查表结构、运行 Maven 测试，用户能用 `evidenceId` 核对真实结果。首发覆盖 Claude Code、Codex、Cursor，以及 Apple Silicon、Intel macOS 和 Windows 11 x64。

基线为 2026-11-23 的架构 v1.0.0。计划起点为含 README、MIT LICENSE 和架构文档的仓库，工程交付状态见下方任务表和当前检查点。Phase 0–3 实现首版，Phase 4 对应 v0.2，Phase 5 对应 v1.0 和由真实需求决定的 v1.0+ 能力。运行形态、权限、密钥、绑定或接口兼容发生变化时，按 §0、§20 先处理 ADR 并同步基线。[架构全文](../architecture/ARCHITECTURE.md)

## 从哪里开始

1. 首次执行从 [T01 建立工作区和三目标构建](tasks/T01-workspace-ci.md) 开始。架构 §24 规定的第一张功能 Issue 覆盖整个 Phase 0，其四个实施任务按表中顺序交付。
2. 再次接手时先看下面的任务表和“当前检查点”。从上到下选择本版本首个前置已完成的任务，打开链接阅读该卡的目标、实施范围、验收编号和交接要求。`T` 为实施，`V` 为原生平台验收，`R` 为阶段评审。
3. 开始新模块先读架构 §1、§4、§5、§8、§9、§14、§18、§19 和任务卡标注的专节。核对当前代码、锁文件与上一个任务的证据。架构 §23 提供官方文档入口，库 API 和客户端行为在实施时按锁定或实测版本核对。
4. 先确定用例、契约和负面用例，再接适配器。修 bug 先保存失败测试，再修复重跑。前端变更用 ego-browser 在本地测试环境实际操作，涉及 Tauri、系统凭据、安装器或外部客户端时补相应原生实测。
5. 收尾时在任务卡末尾记录修订、平台/版本、实际命令或操作、预期与结果、脱敏证据位置及剩余项。在本表更新状态和当前检查点。下一位执行者应能据此复跑。实测记录应使用隔离的测试项目、账号、资源和占位凭据。

### 顺序、状态与续做

- **顺序**以本表行为准。“前置”指顺序上的交接点，详细技术输入见各卡。先前阶段通过 R 评审才能进入下一阶段。一个任务可跨会话或平台续做，未通过的强制验收项留在原编号。
- **状态**使用“待开始、进行中、阻塞、已完成”。后续版本初始为“待排期”，只有明确选入版本才转为“待开始”。条件任务若决定延期，在该行记“已延期”、原因及重新评估条件。已有范围决定即可交接，阶段评审只对实际选入的能力验收。已承诺项缺证据时不能以延期销项。
- **编号**用于 Issue、验收项、修订和证据引用。新增任务分配未使用的编号，在正确依赖位置插入。拆分、延期或退役保留原编号和去向。顺序或发布范围变更时，在下方变更记录写明原因、受影响任务和新的前置。
- **阶段评审**给出“通过”或“阻塞”，记录证据及下一动作。缺平台、客户端、测试资源或公发签名条件时写清哪一项待验。允许继续当前任务中独立的验证，评审通过前不跨阶段。R3 区分候选包验收、具备发布条件及实际发布。

## 阶段和交付顺序

| 架构阶段 | 可观察的退出条件 | 评审 |
|---|---|---|
| Phase 0 · 工程与契约 | 三目标编译，桌面/CLI 同库，独立 STDIO 进程返回模拟项目摘要 | [R0](tasks/R0-foundation-review.md) |
| Phase 1 · 项目与工具链 | 真实 Java 8 项目用指定 JDK/Maven 跑出证据，系统环境不变，未信任任务被拒绝 | [R1](tasks/R1-toolchain-review.md) |
| Phase 2 · 环境与 MySQL | DEV/TEST 隔离，八项 MCP 工具接真实用例，未授权目标和生产默认访问被拒绝 | [R2](tasks/R2-mysql-policy-review.md) |
| Phase 3 · v0.1 MVP | 三客户端真机接入，三目标干净安装/升级，全部发布验收有证据，发行制品可下载核验 | [R3](tasks/R3-mvp-release.md) |
| Phase 4 · v0.2 | Redis/ES/SSH 的受控操作和新增任务模板可验证，按需审批另有范围决定 | [R4](tasks/R4-resource-extension-review.md) |
| Phase 5 · v1.0 / v1.0+ | 非敏感模板可跨工作区复用，所选企业能力有兼容、安全和升级证据 | [R5](tasks/R5-enterprise-release-review.md) |

### Phase 0 · 工程与契约

| 任务卡 | 交付重点 | 前置 | 状态 |
|---|---|---|---|
| [T01 建立工作区和三目标构建](tasks/T01-workspace-ci.md) | 工作区、三目标 CI、ADR 与质量门 | — | 已完成 |
| [T02 冻结领域模型与入口契约](tasks/T02-domain-contracts.md) | 领域类型、权限词汇与可执行契约 | T01 | 已完成 |
| [T03 建立 SQLite 存储、迁移与恢复](tasks/T03-sqlite-and-shared-state.md) | 共享数据库、迁移与恢复 | T02 | 已完成 |
| [T04 打通独立 STDIO 进程](tasks/T04-mcp-stdio-skeleton.md) | 独立 STDIO 与受控模拟响应 | T03 | 已完成 |
| [R0 验收骨架与契约](tasks/R0-foundation-review.md) | 同库双入口、模拟协议和编译证据 | T04 | 已完成（通过） |

### Phase 1 · 项目与工具链

| 任务卡 | 交付重点 | 前置 | 状态 |
|---|---|---|---|
| [T05 实现项目导入、绑定和信任撤销](tasks/T05-workspace-trust.md) | 独立绑定、信任、失效与撤销 | R0 | 已完成 |
| [T06 解析本机工具链并诊断版本冲突](tasks/T06-toolchain-doctor.md) | 多版本解析和可操作诊断 | T05 | 已完成 |
| [T07 实现具名任务、首次确认和进程控制](tasks/T07-named-task-runner.md) | 首次授权、子进程环境和取消 | T06 | 已完成 |
| [T08 贯通任务证据与本地诊断](tasks/T08-execution-evidence.md) | 真实 Maven/Node 执行与证据 | T07 | 已完成 |
| [R1 验收项目与工具链闭环](tasks/R1-toolchain-review.md) | Java 8 项目与宿主环境对照 | T08 | 已完成（通过） |

### Phase 2 · 环境与 MySQL

| 任务卡 | 交付重点 | 前置 | 状态 |
|---|---|---|---|
| [T09 落实环境、资源归属与逐请求授权](tasks/T09-target-resource-policy.md) | 目标风险、资源归属与动态 grant | R1 | 已完成（专项通过，全量复验见任务卡） |
| [T10 接入系统凭据存储与损坏修复](tasks/T10-native-credentials.md) | 两个平台的凭据存储与引用生命周期 | T09 | 已完成 |
| [T11 实现 MySQL 连接检测与元数据读取](tasks/T11-mysql-connect-schema.md) | TLS、只读账号、池隔离和 schema | T10 | 已完成 |
| [T12 限制 MySQL 查询并脱敏结果](tasks/T12-mysql-read-query.md) | AST、行列/字节限额与脱敏 | T11 | 已完成 |
| [T13 接齐八项 MCP 工具和 CLI 输出](tasks/T13-mcp-cli-contracts.md) | 八项工具、错误和 CLI 契约 | T12 | 已完成 |
| [R2 验收真实 MySQL 与授权隔离](tasks/R2-mysql-policy-review.md) | 双项目、双目标、双账号实测 | T13 | 已完成 |

### Phase 3 · v0.1 MVP

| 任务卡 | 交付重点 | 前置 | 状态 |
|---|---|---|---|
| [T14 交付可独立定位和升级的 CLI](tasks/T14-stable-cli.md) | 稳定绝对路径与原子替换 | R2 | 已完成 |
| [T15 实现配置预览、合并与回滚](tasks/T15-integration-lifecycle.md) | 五步安装生命周期与配置保护 | T14 | 已完成 |
| [T16 交付 Claude Code 项目接入](tasks/T16-claude-code-integration.md) | 真实 Claude Code 作用域与回滚 | T15 | 已完成 |
| [T17 交付 Codex 可信项目接入](tasks/T17-codex-integration.md) | 真实 Codex 项目信任与回滚 | T16 | 已完成 |
| [T18 交付 Cursor 项目接入](tasks/T18-cursor-integration.md) | 真实 Cursor 项目配置与回滚 | T17 | 待开始 |
| [T19 完成桌面项目、工具链与任务流程](tasks/T19-desktop-projects-toolchains.md) | 桌面项目、工具链、构建与取消 | T18 | 待开始 |
| [T20 完成资源、接入引导和审计界面](tasks/T20-desktop-resources-and-activity.md) | 桌面资源授权、集成、活动与设置 | T19 | 待开始 |
| [T21 生成可升级的三平台候选包](tasks/T21-release-candidate.md) | 签名候选包、升级与供应链 | T20 | 待开始 |
| [T22 复跑用户全流程、安全与性能预算](tasks/T22-release-regression.md) | 全链路、安全负例和性能实测 | T21 | 待开始 |
| [V01 验收 Apple Silicon 安装与升级](tasks/V01-macos-arm64-acceptance.md) | Apple Silicon 干净用户发行验收 | T22 | 待开始 |
| [V02 验收 Intel macOS 安装与 MCP](tasks/V02-macos-intel-acceptance.md) | Intel 原生安装、升级和运行 | V01 | 待开始 |
| [V03 验收 Windows 11 安装与恢复](tasks/V03-windows-x64-acceptance.md) | Windows 11 x64 原生安装与恢复 | V02 | 待开始 |
| [R3 裁决并交付 v0.1 MVP](tasks/R3-mvp-release.md) | v0.1 发布门和已发行制品核对 | V03 | 待开始 |

### Phase 4 · v0.2 资源扩展

| 任务卡 | 交付重点 | 前置 | 状态 |
|---|---|---|---|
| [T23 增加 Redis/Valkey 只读检查](tasks/T23-redis-inspect.md) | 受限 key/命令与 ACL | R3 | 待排期 |
| [T24 增加 Elasticsearch 只读检索](tasks/T24-elasticsearch-read.md) | 版本感知的索引/字段查询 | T23 | 待排期 |
| [T25 增加已批准主机的 SSH 日志任务](tasks/T25-ssh-log-tasks.md) | 固定主机密钥与具名日志任务 | T24 | 待排期 |
| [T26 增加 Gradle 执行及具名任务模板](tasks/T26-additional-task-templates.md) | Gradle 执行与模板授权 | T25 | 待排期 |
| [T27 按需交付一次性高风险审批](tasks/T27-optional-approval-broker.md) | 按需 ADR、鉴权 IPC 与一次性批准 | T26 | 待排期 |
| [R4 验收 v0.2 资源扩展](tasks/R4-resource-extension-review.md) | 新连接器与兼容回归 | T27 | 待排期 |

### Phase 5 · v1.0 与 v1.0+ 企业扩展

| 任务卡 | 交付重点 | 前置 | 状态 |
|---|---|---|---|
| [T28 交付非敏感项目模板与策略导入导出](tasks/T28-shareable-manifests.md) | 非敏感模板与策略交换 | R4 | 待排期 |
| [T29 按需稳定扩展连接器 SDK](tasks/T29-connector-sdk.md) | 按需 SDK 与契约套件 | T28 | 待排期 |
| [T30 按需接入企业凭据与受限代理](tasks/T30-enterprise-credentials-network.md) | 按需企业凭据及受限代理 | T29 | 待排期 |
| [T31 按需交付受限隔离执行](tasks/T31-isolated-execution.md) | 按需原生隔离模式 | T30 | 待排期 |
| [R5 验收 v1.0 及企业能力边界](tasks/R5-enterprise-release-review.md) | 按实测范围交付 v1.0 | T31 | 待排期 |

## 需求与验收追踪

下列编号沿用架构 §2.2、§14.3 和 §19。每行给出主责或联验任务，验收记录回填到对应任务卡，R3 复核完整首发矩阵。

| 用户故事 | 首次交付能力 | 责任任务 |
|---|---|---|
| US-01 | 导入 Git 项目、识别技术栈与独立 worktree | T05、T19 |
| US-02、US-03 | 多版本工具选择、缺失/冲突诊断与修复操作 | T06、T08、T19、R1 |
| US-04、US-05 | 环境与资源归属、OS 凭据引用、连接诊断 | T09–T11、T20 |
| US-06 | 经授权的 MySQL 结构和只读查询 | T11–T13、R2 |
| US-07、US-09 | Maven/Node 具名任务、结果、审计与证据复查 | T07、T08、T12、T13、T20、T22 |
| US-08 | 三客户端接入、验证、卸载和回滚 | T14–T18、T20、T22 |
| US-10、US-11 | Redis、Elasticsearch 只读与 SSH 日志任务 | T23–T25、R4 |
| US-12 | 按需的一次性本地审批 | T27、R4 |
| US-13 | 可分享的非敏感模板和策略导入导出 | T28、R5 |
| US-14 | 连接器 SDK、企业凭据及受限网络代理 | T29、T30、R5，按需求进入 v1.0+ |

### v0.1 功能与安全用例

| 功能用例 | 主责验收 | 安全用例 | 主责验收 |
|---|---|---|---|
| E2E-01 · Apple Silicon 安装/卸载/重装 | V01 | SEC-001 · 未绑定项目拒绝 | T05 |
| E2E-02 · Intel 安装与 CLI/MCP | V02 | SEC-002 · 已撤销绑定拒绝 | T05 |
| E2E-03 · Windows NSIS / WebView2 / CLI | V03 | SEC-003 · 非授权目标拒绝 | T09 |
| E2E-04 · 真实 Java 8/Spring Boot 与 Maven 测试 | T08、R1 | SEC-004 · PROD 写入拒绝 | T09、R2 |
| E2E-05 · 指定 JDK 8/17/21，系统环境不变 | T06–T08、R1 | SEC-005 · MCP 无 secret getter | T13 |
| E2E-06 · DEV/TEST 连接与账号隔离 | T11、R2 | SEC-006 · SQL 多语句与危险函数拒绝 | T12 |
| E2E-07 · 只读账号查结构/SELECT，写入失败 | T11、T12、R2 | SEC-007 · 跨库/越界表拒绝 | T12 |
| E2E-08 · Claude Code 接入、卸载、回滚 | T16 | SEC-008 · 路径穿越/链接替换失败 | T05、V03 |
| E2E-09 · Codex 接入、卸载、回滚 | T17 | SEC-009 · 输出、审计及日志无密码 | T08、T10、T22 |
| E2E-10 · Cursor 接入、卸载、回滚 | T18 | SEC-010 · STDIO stdout 零污染 | T04、T13 |
| E2E-11 · 关闭桌面，CLI/MCP 独立运行 | T14、V01–V03 | SEC-011 · 配置回滚保留第三方条目 | T15–T18 |
| E2E-12 · 退出码、耗时、脱敏日志与 evidenceId | T08、T22 | SEC-012 · 未信任仓库不能执行任务 | T07、R1 |

### 跨任务交付责任

| 基线 | 主责及最终证据 |
|---|---|
| §1.3 第 1、3、8 项 · 绑定、默认拒绝、调用时鉴权 | T05、T09、T13 的负面用例和 R2 的跨项目实测 |
| §1.3 第 2、5 项 · 远端目标与本机工具链分离，子进程环境隔离 | T06–T09 的双项目、多 JDK 和环境快照 |
| §1.3 第 4、9 项 · 凭据保密，首版基础设施只读 | T10–T13 的原生 provider、只读账号与脱敏测试，T22 全链路扫描 |
| §1.3 第 6、7 项 · 桌面可关闭，Rust 共用业务逻辑 | R0 同库双入口、T13 契约对照及 V01–V03 安装版独立运行 |
| §1.3 第 10、11、12 项 · 证据、网关治理边界、真实兼容性 | T08/T12 报告，T21 对宿主 Shell/可信构建/VPN 限制的发布说明，T16–T18 及 T22 的真实客户端记录 |
| §8、§12 · 接口兼容 | T02 冻结样例，T04 验证协议传输，T13 验证八项 MCP 和 CLI，T15–T18 补齐 integration 子命令与实测 |
| §13 · 八个桌面页面、引导、可访问性和 IPC | T19、T20 本地交互与原生验证，T22 从安装版复跑主流程 |
| §17、§19.2 · 离线、性能、并发、取消、假秘密扫描 | T07、T10–T13 实现限制，T22 在候选包测量并记录样本、结果和剩余差异 |
| §10.5、§19.3 · 多进程、崩溃、升级、回滚、凭据修复 | T03、T10、T14、T15、T21 制备旧数据与故障夹具，T22 及 V01–V03 复验 |
| §16、§19.3 · 平台、签名、许可与发行制品 | T21 生成候选包，V01–V03 分别验收，R3 按同一候选修订及制品哈希放行 |

## 验收环境准备

| 最晚使用点 | 需要的环境或输入 | 记录什么 |
|---|---|---|
| R0 | macOS arm64、macOS x64、Windows x64 构建作业 | 各自目标、工具版本、构建结果。原生运行在相应验收任务单独取证 |
| R1 | 有 JDK 8/17/21、Maven、Node 及可运行 Java 8/Spring Boot 项目的隔离环境 | 版本、选中工具、系统环境前后对照、成功与失败的证据 |
| R2 | DEV/TEST 两套可区分的真实 MySQL 测试资源、专用只读账号、测试证书及两个系统的凭据存储 | 版本、账号权限摘要、表标记、拒绝/恢复记录，证据移除内部地址与凭据 |
| T16–T18 | 三个真实客户端、隔离测试项目/配置和必要的测试账号 | 实际 OS、版本、作用域、协商结果、安装/验证/卸载与回滚 |
| T21–R3 | 发行签名与公证环境、三目标干净测试用户、兼容升级基线 | 候选修订、包哈希、签名验证、安装/升级结果。Intel 采用原生 CI 并按需补实机，Windows 11 用 x64 原生环境 |

## 当前检查点

| 当前任务 | 已验证 | 待做或阻塞 | 续做入口 |
|---|---|---|---|
| T18 待开始 | [T17-A01/A02](tasks/T17-codex-integration.md#三目标原生验收) 已在 Apple Silicon、Intel macOS 和 Windows x64 验证 Codex 可信项目接入、工具调用、撤销及配置恢复 | 验证 Cursor 项目接入及切换项目后的绑定隔离 | [T18 交付 Cursor 项目接入](tasks/T18-cursor-integration.md) |

任务中断时，用一行替换当前检查点，写下最后通过的验收编号、修订或文件、复现命令和下一个动作。一次任务完成后保留证据在原卡，再将此处指向下一张卡。

## 计划变更记录

| 日期 | 变更 | 影响 |
|---|---|---|
| 2026-11-23 | 按架构 v1.0.0 建立初始顺序与阶段验收 | Phase 0–3 交付 MVP，Phase 4–5 独立排期 |
