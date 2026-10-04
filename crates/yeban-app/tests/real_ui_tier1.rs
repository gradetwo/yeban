//! 真实界面的 Tier-1 判据 —— **同一个判据源码的第二个 cargo 目标**。
//!
//! 判据本身全部住在 `crates/yeban-app/src/test_port_adapter.rs`（本文件用 `#[path]` 原样引入，
//! 不存在第二份实现，也就不会漂移）。本目标存在的唯一理由是**让它真的被执行**：
//!
//! - 那个文件同时是 `[[test]] name = "test_port_adapter"`，挂在
//!   `required-features = ["ui-test-port"]` 后面。该 feature 默认关闭，而 CI 目前没有任何
//!   一步启用它（`.github/workflows/ci.yml` 里那一步被集成者按纪律暂时移除），
//!   于是它**从未被编译过** —— commit `66b002c` / run 37223586792 的实测就是 8 处编译错误。
//! - 本目标是 `tests/` 下的**自动发现**集成测试，`cargo test -p yeban-app --all-targets`
//!   一定会编译并运行它；依赖通过 `[dev-dependencies] yeban-ui-test-port` 提供。
//! - dev-dependencies **不进入** release 构建：`cargo build -p yeban-app`（含 `--release`）
//!   的依赖图不变，因此 `AGENTS.md` §2 红线 6 与 `Cargo.toml` 里"内省能力不进默认构建"
//!   的契约没有被削弱。核验命令（零编译，本机可跑）：
//!
//!   ```text
//!   cargo tree -p yeban-app -e normal --locked    # 不应出现 yeban-ui-test-port
//!   ```
//!
//! 两个目标同时启用 feature 时会把同一批判据跑两遍（两个测试二进制各一遍）——
//! 这是刻意的：集成者的 `cargo test -p yeban-app --features ui-test-port --locked`
//! 必须继续可用，而"判据在 CI 上真的执行"不能依赖任何 feature 开关。

#[path = "../src/test_port_adapter.rs"]
mod criteria;
