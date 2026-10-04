# 夜半 (Yeban) 开源贡献指南 (Contributing Guide)

感谢您对 **夜半 (Yeban) / Yeban DAW** 开源项目的关注与支持！本项目致力于基于 Slint + 纯 Rust 构建下一代专业级开源数字音频工作站。

---

## 1. 许可证与贡献者协议 (Developer Certificate of Origin - DCO)

本项目基于 **GNU General Public License v3.0 (GPLv3)**（带 CLAP 插件加载附加许可）开源。
为了维护开源社区的合规性与所有贡献者的合法权益，本项目采用 Linux 基金会的 **开发者原创证书 (Developer Certificate of Origin - DCO 1.1)** 机制：

- 您的每一次代码提交（Git Commit）必须包含 Signed-off-by 签名行（通过 `git commit -s` 自动生成）；
- 签名即代表您声明：该代码由您原创创作，或您有权依据 GPLv3 许可证将其贡献给本项目；
- 提交合并至本项目主分支后，即表示您同意该贡献永久以 GPLv3 许可证向全世界公开发布。

---

## 2. GPLv3 源代码分发规范与依赖管理 (Source Code Distribution)

依据 GPLv3 第 6 条的严格法律要求：
1. **源码完整性**：发布任何二进制发行版（Binary Releases）时，必须同步提供对应版本的完整源代码 Tarball，包含所有构建脚本、`Cargo.lock` 以及 `slint-build` 所需的原始 `.slint` 声明式源文件（Slint 编译生成的 Rust 中间结构体不构成合法源码替代）；
2. **`Cargo.lock` 必须纳入版本控制**：夜半 (Yeban) Workspace 包含最终的二进制执行程序（`crates/yeban-app` 与 `crates/yeban-mcp`），`Cargo.lock` 必须强制纳入 Git 跟踪管理，确保 100% 可重现构建；
3. **第三方依赖打包 (`cargo vendor`)**：CI 发布流程在构建发布包时，必须执行 `cargo vendor` 并归档第三方 crate 源码包，确保满足 GPLv3 §6(d) 传递提供所有第三方依赖源码的法定合规义务。

---

## 3. 开发环境与构建指令 (Development Setup)

### 3.1 基础依赖要求
- Rust 1.80+ (Stable toolchain)
- CMake, pkg-config (部分底层库构建所需)
- 操作系统底层音频库头文件（Linux: `libasound2-dev`, `libpipewire-0.3-dev` 等）

### 3.2 常用开发指令
```bash
# 1. 检出代码
git clone https://github.com/yeban-daw/yeban.git
cd yeban

# 2. 运行完整工作区单元测试与属性测试
cargo test --workspace

# 3. 运行代码格式化与 Clippy 静态检查
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings

# 4. 运行无头模式 (Headless) 与 UI 自动化测试
SLINT_BACKEND=headless \
SLINT_MCP_PORT=9315 \
cargo test -p yeban-app --features "slint/mcp,slint/renderer-skia"

# 5. 启动开发版 GUI
cargo run -p yeban-app
```

---

## 4. 代码质量与实时音频约束 (Real-Time Safety Rules)

在向 `crates/yeban-engine`、`crates/yeban-dsp`、`crates/yeban-sfz` 提交 PR 时，必须严格遵循实时声学铁律：
1. **音频回调零分配 (Zero Allocation)**：音频渲染热路径（Render Callback）内严禁调用 `malloc`、`free`、`Box::new`、`Vec::push` 或任何隐式分配；
2. **零互斥锁与零阻塞调用 (Zero Locks / I/O)**：音频线程严禁持有 `std::sync::Mutex` 或执行文件读写、网络操作，所有主线程通信必须走无锁 SPSC（`rtrb` bulk API）；
3. **确定性浮点计算**：DSP 核心严禁引入平台不确定性数学函数，统一使用固定的采样块大小（128 点）与可重现浮点运算逻辑。
