# T03 建立 SQLite 存储、迁移与恢复

Phase 0 · 工程与契约　·　[返回交付总表](../PLAN.md)

| 开工条件 | 内容 |
|---|---|
| 顺序前置 | [T02 冻结领域模型与入口契约](T02-domain-contracts.md) |
| 架构与追踪 | [架构基线](../../architecture/ARCHITECTURE.md) §3.2、§4.5、§5、§10.1–10.5、§19.3 · US-01、US-09 |
| 工作范围 | `crates/storage/`、application 仓储端口、桌面及 CLI 读入口、迁移测试 |

## 交付目标

桌面和 CLI 通过同一应用服务读到同一份项目数据，三个入口共用 schema 兼容检查。

## 实施与产出

1. 在 storage 按 §10.4 建立十张表和迁移器，落实统一 ID、外键、唯一约束、长度校验、索引和 UTC。平台目录 API 分配数据库、日志、临时文件与备份，使用用户专属权限或 ACL。
2. 使用 WAL、busy_timeout、短事务。新旧 schema 不兼容时拒绝写并提供升级诊断。升级前备份，失败后恢复或转只读，处理恢复时的 WAL 状态。数据库保存非敏感摘要与凭据引用。
3. 实现最小项目读用例、CLI 查询和类型化 Tauri Command。用测试初始化项目数据。用户导入、信任与实际授权留给 T05。

## 验收

- **T03-A01**　从空库及上一版测试快照迁移后，桌面和独立 CLI 读到同一个项目 ID。无效外键和重复业务键写入失败。
- **T03-A02**　两个独立进程竞争写锁、迁移中断及重启后，数据仍可读、事务一致。用旧入口打开较新 schema 时原库内容未变。备份可还原并核对资源引用。
- **T03-A03**　分别检查 macOS 文件模式及 Windows 用户 ACL。扫描数据库、WAL、备份和迁移错误输出，确认无假秘密标记。

## 交接

移交可重放的新旧库夹具、恢复演练和双入口读数，供 R0 与最终升级演练复用。

## 执行记录

### 2026-11-24 · SQLite 数据格式与复跑

实现起点为 T02 的 `d6feb3f`。迁移 1 建立十张表，迁移 2 增加项目描述。上一版数据使用合成的 v1 夹具。数据库以 16 字节保存 UUID，入口以标准 UUID 文本返回，时间使用 UTC Unix 毫秒。[迁移 SQL](../../../crates/storage/migrations/001_baseline.sql) · [上一版夹具](../../../crates/storage/tests/fixtures/previous.sql)

数据库位于用户应用目录的 `Envlume/data/envlume.db`。macOS 的应用目录由系统解析到 Application Support，Windows 使用 LocalAppData。日志、临时文件及备份分别放在 `logs`、`tmp`、`backups`。备份文件采用 `schema-<版本>-<UUID>.db` 命名，只保存元数据与凭据引用。

| 验收 | 操作与预期 |
|---|---|
| T03-A01 | 从空库建表，再从 v1 夹具升级。无效外键、重复资源别名、无效 UUID 长度及超长名称均被 SQLite 拒绝。正常 UI 进程的 Rust 后端和独立 CLI 共用 `list_projects` 用例，原生 WebView 冒烟样例调用主窗口权限下的同一个 Tauri Command，返回测试项目 `orders`，ID 为 `11111111-1111-4111-8111-111111111111`。项目导入与授信由 T05 实现。 |
| T03-A02 | 持有写锁时，另一进程在约 5 秒后收到忙错误；释放写锁后两个进程竞争升级，只发布一份升级前备份。在更新版本、提交事务之前终止迁移子进程，重新打开仍读到完整 v1，再升级成功。保存于 WAL 的最新凭据引用在备份还原后可复查。旧入口读取较新 schema 会收到升级诊断，主库和 WAL 字节保持一致。 |
| T03-A03 | 对实际数据库目录、备份、数据库、WAL、SHM 与迁移锁检查原生权限。用合成密码标记触发 SQLite 错误，核对应用诊断已脱敏，并直接扫描数据库文件、WAL 和备份。测试在临时目录运行，使用合成端点与凭据引用。 |

**恢复演练。** 将升级前备份交给 `Store::restore_into(backup, fresh_paths)`。恢复器校验数据库标识、版本、完整性及外键，通过 SQLite API 复制，在新位置重新打开并应用后续迁移。目标已存在主库或日志时拒绝覆盖。当前版本的快照在还原后也重新启用 WAL。迁移失败会回滚事务，原库可用原版本继续读取。[可重放的故障与权限测试](../../../crates/storage/src/tests.rs)

从仓库根目录运行以下命令。本机原生验证使用 macOS 26.5.2 arm64、Rust 1.99.1。第一条覆盖 11 项存储测试，桌面样例使用临时应用目录和真实 WebView。CI 在一次性 runner 的空白用户配置中另外启动独立 CLI，逐字段比对两个入口返回的 JSON。

```sh
cargo test --locked -p envlume-storage
pnpm install --frozen-lockfile
pnpm build
cargo run --locked -p envlume-desktop --example shared_catalogue --features tauri/custom-protocol
```

**原生证据。** 验收修订 `ca81655`。Apple Silicon、Intel macOS 和 Windows x64 原生作业均通过新旧库、并发、恢复及双入口读数检查。Windows Server 2025 x64 的 DACL 实查覆盖数据库、WAL、SHM、备份和目录；两种入口均返回同一测试项目。初版 Windows 验收示例缺少正式程序使用的 Common Controls v6 manifest，修复后通过同一原生测试。[Apple Silicon](https://github.com/Refinex-Space/envlume/actions/runs/45222768882/job/137861180244) · [Intel macOS](https://github.com/Refinex-Space/envlume/actions/runs/45222768882/job/137861179471) · [Windows x64](https://github.com/Refinex-Space/envlume/actions/runs/45222768882/job/137861180084)

**质量门。** `cargo fmt --all --check`、`cargo clippy --workspace --all-targets --locked -- -D warnings`、`cargo test --workspace --locked`、`cargo build --workspace --locked`、`pnpm desktop build --debug --no-bundle -- --locked` 通过。前端锁定安装、lint、typecheck、unit 和 build 通过。许可证和 Git 历史秘密扫描通过，RustSec 保留 T01 已登记的三条依赖警告。[安全作业](https://github.com/Refinex-Space/envlume/actions/runs/45222768882/job/137861180207)

**交接。** T04 可复用 `Store::open` 的版本检查及绑定快照仓储，继续验证独立 STDIO 模拟调用。Windows 11 安装、升级和卸载的发行验收仍由 V03 负责。
