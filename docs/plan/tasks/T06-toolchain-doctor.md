# T06 解析本机工具链并诊断版本冲突

Phase 1 · 项目与工具链　·　[返回交付总表](../PLAN.md)

| 开工条件 | 内容 |
|---|---|
| 顺序前置 | [T05 实现项目导入、绑定和信任撤销](T05-workspace-trust.md) |
| 架构与追踪 | [架构基线](../../architecture/ARCHITECTURE.md) §6.1–6.3、§12、§15.2、§17 · US-02、US-03 · E2E-05 |
| 工作范围 | `crates/toolchain/`、application、profile 仓储与 doctor 入口 |

## 交付目标

同机安装多套工具时，为每个项目选择可说明来源、版本和路径的工具链。

## 实施与产出

1. 实现显式路径/JDK Home、已锁定或获准的可选 mise 配置、项目原生约定、本机发现的解析优先级。macOS 兼顾无终端 PATH 的 GUI，Windows 使用原生路径。发现 wrapper 时仅解析信息。
2. 识别 JDK/java/javac 的版本、发行版和架构，Maven/JDK 配对、Gradle 基础信息、Node 和 npm/pnpm/yarn 的 lockfile、Python/uv/venv、Git/worktree、Rust/Cargo、Docker/mise 是否存在。
3. 提供可持久化的 ToolchainProfile、主机 doctor 和绑定级 toolchain_doctor 用例。报告来源、约束、实际版本、有效路径摘要与修复动作。Maven settings 只报告配置状态。

## 验收

- **T06-A01**　以不同显式路径、系统发现、缺失工具、版本冲突、带空格路径和 GUI 精简环境为输入，核对选择及原因。探测不安装工具、不改父进程环境。
- **T06-A02**　指定 Java 8 项目时优先选 JDK 8，与选用 17 的项目独立。识别 JDK 21 时仍服从指定版本。断网亦可诊断本机已有工具。
- **T06-A03**　对缺 JDK、包管理器与锁文件不匹配、失效解释器分别给出可执行的修复建议。测试报告和 doctor 输出均不含 settings 内容或环境凭据。

## 交接

移交跨平台工具发现夹具、两套项目配置和诊断样例，供 T07 与 R1 做实际执行对照。

## 执行记录

待执行。收尾时登记修订、平台/版本、实际命令或操作、对应验收编号、结果与脱敏证据位置。
