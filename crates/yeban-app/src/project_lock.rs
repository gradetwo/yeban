//! 工程文件的 `.yeban.lock` —— **与 `yeban-mcp` 字面上同一份源码**
//! （`[ARCH-SEC-001]` / `[MUST-GATE-008]`；`ROAD-M4-008` 选项 (a) 第三片）。
//!
//! ## 为什么是 `#[path]` 而不是"再写一份"
//!
//! 本仓库对"同一件事有两份实现"的态度是明令禁止的：`src/undo.rs` 用
//! `#[path = "../../yeban-mcp/src/undo_session.rs"]` 把撤销会话装进来，
//! 理由正是"与 MCP 侧**字面上同一份源码**"。锁要走同一条路：
//!
//! ```text
//! crates/yeban-mcp/src/domain/lock.rs   ←── 唯一实现（原子创建 + OS 建议锁）
//!         │  #[path]
//!         └────────────► crates/yeban-app/src/project_lock.rs   （本模块）
//! ```
//!
//! 因此 [`lock`] 里的 `acquire` / `LockMode` / `LockGuard` / `LockError` 与
//! `yeban_mcp::domain::lock` 是**同一份代码**（在 `yeban-app` 里各实例化一次 ⇒
//! 两侧类型不可互换，这与 `undo_session.rs` 的情形完全一样，也正是"两边不可能各写
//! 一套协议"的机械保证）。
//!
//! ## 为什么不加一条 `yeban-mcp` 依赖边
//!
//! `MUST-GATE-009` 的第一道开关是**编译期**的：默认构建里不许有 `yeban-mcp`
//! （核验命令 `cargo tree -p yeban-app -e normal --locked | grep -c yeban-mcp` ⇒ **0**）。
//! 若为了拿锁而依赖 `yeban-mcp`，那道开关就没了 —— 而 `--save-as` 是**默认构建**里
//! 就存在的写路径。⇒ 唯一既"不写第二份锁"又"不动依赖边"的做法就是共享源码。
//!
//! 代价是一条**直接依赖边**：这份源码的锁元数据用 `serde_json`（它早就在默认依赖图里，
//! 见 `Cargo.toml` 的注）。包集合、feature 集合、`yeban-mcp` 命中数**都不变**。
//!
//! ## 谁在什么模式下用它
//!
//! | 写路径 | 目标 | 模式 | 落点 |
//! | :--- | :--- | :--- | :--- |
//! | `ui/force_save` | 工程容器 | [`LockMode::ExclusiveWrite`] | [`crate::save::save_project_file`] |
//! | `--save-as` | 工程容器 | [`LockMode::ExclusiveWrite`] | [`crate::save::save_archive_file`] |
//! | `--export-elements` / `--export-midi` / `--export-als` | **不是工程**（清单 / `.mid` / `.als`） | **不取锁** | [`crate::save::write_file_atomically`] |
//!
//! 导出目标是**别的产物**，对它取 `.yeban.lock` 会凭空在 `song.mid` 旁边造一个
//! `song.mid.lock`，而且与任何工程的锁都不互斥 —— 那是假保护，不是保护。
//!
//! ## 与控制面（形态 A）的关系：**同一把锁，同一个内核仲裁**
//!
//! 控制面会话在 `crates/yeban-app/src/mcp_mount.rs` 里用
//! `yeban_mcp::domain::store::acquire_lock(path, read_only)` 持锁（只读 ⇒
//! [`LockMode::SharedRead`]）；本模块的 [`lock::acquire`] 与它**是同一个函数体**，
//! 因此"GUI 保存"与"控制面会话"争的是**同一个 inode 上的同一把建议锁**。
//! 后果（两条都如实登记，不是巧合）：
//!
//! 1. 别的进程/形态以排他写持锁时，GUI 的保存**拒绝写入**（`SaveError::Locked`），
//!    而不是"绕过锁直接覆盖"——这正是 `ROAD-M4-008` §6.4 拒绝翻 `read_only` 的原因；
//! 2. **本进程**的只读控制面会话正持 [`LockMode::SharedRead`] 时，GUI 的排他写同样
//!    拿不到锁（`flock` 在 fd 粒度上仲裁，同进程另一个 fd 也冲突）⇒ 保存被**拒绝**。
//!    这是 fail-closed：宁可拒绝，也不产生第二个写者。要把"控制面存活期间 GUI 也能存"
//!    做出来，前提是让**一个**持有者（会话）成为唯一写者，那需要新的宿主保存动作 ——
//!    本片不做，逐条登记在 `docs/ledger/m4-008-authority-notes.md` §7.4。

#[path = "../../yeban-mcp/src/domain/lock.rs"]
pub mod lock;
