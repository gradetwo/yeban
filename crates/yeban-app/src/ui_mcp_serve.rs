//! `--enable-ui-mcp-http`：把 **UI 控制面**（`ui/*`，`[ARCH-UI-004]`）挂成一个
//! **真的能 `curl` 的环回入口**，并让一个人在**正在跑的界面**上驱动它。
//! 规范来源 (Normative)：`[ARCH-UI-004]`（JSON-RPC 内省协议）、`[UI-MCP-001]`（三级权限）、
//! `[MUST-GATE-009]`（网络监听默认关 / 只绑环回 / 必须令牌）、`[ARCH-SEC-002]`（`0600` 令牌）、
//! `[MUST-GATE-015]`（像素只能来自 Tier-1 软件光栅化）。
//!
//! ## 这个模块补的是哪个洞（一句话）
//!
//! 在这一条之前，14 条 `ui/*` 方法**只能**从 `yeban-app` 的测试目标进程内调用
//! （`tests/live_ui_mcp.rs` 用 `#[path]` 把 `src/live_surface.rs` 装进去）。
//! `crates/yeban-ui-mcp` 早就有一个 `ui-mcp-http` feature 与
//! `UiHttpServer::bind_loopback`，但**没有任何产品入口把它们接起来** —— 于是"AI/人
//! 能在活的界面上看见并操作界面"这句话在**产品形态**上是不可验证的。
//! 本模块把那条链接上：装配执行面 → 绑定环回 → 落 `0600` 令牌 → 一拍一拍地服务。
//!
//! ## 为什么是**这一档**进程形态（不是 GUI，也不是无头的 `--headless`）
//!
//! 这是本模块最需要被读懂的一段，因为"为什么不在 GUI 进程里挂"有一个**类型层面**的答案：
//!
//! | 形态 | 有没有 `ui/*` 的执行面 | 为什么 |
//! | :--- | :--- | :--- |
//! | `--headless`（`cli::run_batch`） | **没有** | 它一个 Slint 对象都不构造（这是它的契约），而 `ui/*` 的每一条都长在活控件树上 |
//! | GUI（`run_gui`，默认形态） | **装不下** | `ui/*` 的像素必须来自 Tier-1 软件光栅化（`[MUST-GATE-015]` 允许的唯一 Golden 路径）。`Tier1Window::install` 调 `slint::platform::set_platform`，而它是**线程局部且每线程一次**、且**必须先于任何 Slint 组件构造**；GUI 路径先 `MainWindow::new()` 就把 winit 后端装上了 ⇒ 那里 `set_platform` 只会返回 `Err` |
//! | **本档**（软件平台上的活控件树） | **有** | `build_live_ui_with` 先装 Tier-1 平台、再建 `MainWindow`（唯一注入点 `host::build_main_window*`，D28）⇒ 真的有一棵被布局、被光栅化、能被事件注入的控件树 |
//!
//! 所以本档**不是**"降级成一个服务器"，而是"`[MUST-GATE-015]` 指定的那台渲染器
//! 正好只有这一档进程形态"。它仍然是**真界面**：换主视图会真的改窗口属性并回读，
//! 截图是真的由 `SoftwareRenderer` 光栅化出来的 PNG，`ui/tree` 是运行时控件树。
//!
//! ## 与领域 MCP（形态 A）的关系：**同一套安全模型，两条依赖边**
//!
//! | 关心的事 | 领域 MCP（`--enable-mcp-http`） | 本档（`--enable-ui-mcp-http`） |
//! | :--- | :--- | :--- |
//! | 编译期开关 | `in-process-mcp`（非默认） | `ui-mcp-http`（非默认） |
//! | 运行期开关 | `--enable-mcp-http` / `YEBAN_MCP_HTTP=1` | `--enable-ui-mcp-http` |
//! | 绑定 | `HttpServer::bind_loopback` → `127.0.0.1:0` + 回读断言 | `UiHttpServer::bind_loopback`（**同一份** `assert_loopback`） |
//! | 令牌 | `BearerToken::generate`（256 bit） | **同一个**生成函数 |
//! | 落盘 | `~/.yeban/session.token`，`0600` | **同一个** `TokenFile` |
//! | 鉴权 | `security::authenticate`，缺/错/形状非法 ⇒ `401` + `WWW-Authenticate` | **同一个**函数（判定在 `UiService`，不在传输层） |
//! | 默认关（红线 6） | `[features] default` 里没有它 | 同款（`ui-mcp-http` 不在 `default`） |
//!
//! **两者不会同时在同一个进程里**：领域控制面要求"正在跑的 GUI 事件循环"（`cli.rs` 的
//! `McpHttpNeedsGui`），而本档要的是软件光栅化平台 —— 同一个进程里两者互斥。
//! 于是"一个端口 / 一个令牌 / 一条生命周期"在这里是**字面成立**的：
//! 本进程里只有**一个**控制面。这也是 `parse()` 拒绝 `--enable-mcp-http` +
//! `--enable-ui-mcp-http` 的原因（见 `ParseError::UiMcpHttpConflict`）。
//!
//! ## 安全边界（逐条可被外部命令验证）
//!
//! 1. **只绑环回**：地址由 `UiHttpServer::bind_loopback` 决定（`127.0.0.1:0`），
//!    本模块**不接受**任何地址参数 ⇒ 没有"改成非环回地址"的入口；端口由系统分配；
//! 2. **必须带 Bearer 令牌**：缺失 / 形状非法 / 不匹配 ⇒ `401` + `WWW-Authenticate`，
//!    **不回退放开**（判定在 `yeban_mcp::security::authenticate`，本模块不碰判定）；
//! 3. **令牌落 `0600`，内容不打印**：报告行只有路径；
//! 4. **fail closed**：令牌写不进去 ⇒ **停机并释放监听口**，然后才退出（非零）；
//! 5. **`ui:inject` 硬禁**：控制面一律以 `RunMode::Production` 构造 ⇒ 注入族被
//!    `security::authorize` 硬拒，而 `ui/switch_main_view` 等按其 `app:*` scope 授权。
//!
//! ## 怎么跑（下面这条命令已在本机真二进制上跑过，逐条响应见本次交付的报告）
//!
//! ```text
//! cargo build -p yeban-app --features ui-mcp-http
//! HOME=<可写目录> ./target/debug/yeban-app --enable-ui-mcp-http --headless-idle --idle-seconds 30
//! TOKEN=$(cat "$HOME/.yeban/session.token")
//! curl -s -X POST http://127.0.0.1:<port>/ui-mcp -H "Authorization: Bearer $TOKEN" \
//!      -d '{"jsonrpc":"2.0","id":1,"method":"ui/tree"}'
//! ```

use std::thread;
use std::time::{Duration, Instant};

use yeban_ui_mcp::transport::mount::UiHttpMount;
use yeban_ui_mcp::transport::{HttpStartup, plan_http_startup};
use yeban_ui_test_port::port::Permission;

use crate::cli::{self, CliError, Options};
use crate::live_surface::{LiveWiringOptions, build_live_ui_with};

/// 本档的握手行（与 `headless ok` / `headless-idle ok` **刻意不同**）。
///
/// 三个字面值说的是三件不同的事：`headless ok` = 一个 Slint 对象都没构造；
/// `headless-idle ok` = 建了控件树 + 光栅化一帧后空闲；本行 = 建了控件树**并且**
/// 已经在环回 socket 上应答 `ui/*`。因此本行**只在**"绑定 + 令牌落盘都成功"之后才打
/// （见 [`run`] 的顺序）。
pub const HANDSHAKE: &str = "ui-mcp-http ok";

/// 本档的边界声明行（说清它**做**了什么、**没做**什么）。
pub const BOUNDARY: &str = "ui-mcp-http: 已构造真 MainWindow (平台 = MinimalSoftwareWindow/SoftwareRenderer) \
并挂上生产模式 UI 控制面 (只绑环回 + Bearer 令牌 + ui:inject 硬禁); 未创建 OS 窗口、未进阻塞事件循环、\
未连声卡/未建引擎、未跑领域 MCP (两条控制面互斥, 见 src/ui_mcp_serve.rs 的模块文档)";

/// 空闲轮询的间隔（毫秒）。
///
/// 5 ms 是个折中：一次 `curl` 的等待里不会被它拖出可感知的延迟（本地自动化），
/// 而空转时每秒只醒 200 次 —— 与 `--headless-idle` 的 10 ms 同族，不占 CPU。
const POLL_IDLE: Duration = Duration::from_millis(5);

/// `accept` / 连接级失败后的退避（与 `crate::mcp_mount` 的工作线程同款）。
const ACCEPT_RETRY_BACKOFF: Duration = Duration::from_millis(10);

/// 跑一次 `--enable-ui-mcp-http`：装配 → 绑定 → 落令牌 → 服务 → 停机并释放。
///
/// ## 顺序是契约（每一段都有理由）
///
/// 1. **先装配执行面**（`build_live_ui_with`：装 Tier-1 平台 → 建 `MainWindow` →
///    `show()` → 抓运行时控件树 → 抓一帧对照）—— 装不出来就什么都不该绑；
/// 2. **再绑定环回**（`127.0.0.1:0` + 回读断言）；
/// 3. **再落令牌**，失败 ⇒ **先 `stop()` 释放监听口**，再报错退出（fail closed）。
///    顺序不能反：`ui-mcp-http ok` 这一行是**契约**，它出现必须蕴含"正在应答 `ui/*`"；
/// 4. **最后才打印**（握手行 / 工程读数 / 执行面证据 / 端点 / 令牌路径 / 服务时长），
///    然后进服务循环。返回值里只剩下"停机 + 边界"两行（由 `cli::finish` 打到最后）。
///
/// # Errors
///
/// - 运行期要求开 HTTP 但本次构建没编译 `ui-mcp-http`（防御性第二道；`parse()` 已经拒了）；
/// - 工程打不开 / 投影失败（与其它命令同源）；
/// - 执行面装配失败（平台 / 组件 / Tier-1 抓帧 / 注册表适配）；
/// - 绑定环回失败；
/// - 令牌落盘失败（**此时监听口已被释放**，退出码非零）。
pub fn run(options: &Options) -> Result<Vec<String>, CliError> {
    // 第二道开关的判定**复用** `yeban-ui-mcp` 的纯函数（与 `mcp_mount` 复用
    // `yeban-mcp` 的 `plan_http_startup` 同款）：本模块不自己判 feature 有没有进来。
    // 两个 `Err`/`Disabled` 分支在实践中都不可达（`parse()` 已经判过并给退出码 2），
    // 保留它们是为了**不吞掉**底层判定 —— 而不是把"编译期没进来"静默成一个空操作。
    match plan_http_startup(options.ui_mcp_http) {
        Ok(HttpStartup::Enabled) => {}
        Ok(HttpStartup::Disabled) => {
            return Err(CliError::Ui {
                detail: format!(
                    "内部错误: `{}` 没被置位却走到了 UI 控制面入口 (parse 已保证不会发生)",
                    cli::UI_MCP_HTTP_SWITCH
                ),
            });
        }
        Err(error) => {
            return Err(CliError::Ui {
                detail: format!("{}", error),
            });
        }
    }

    let loaded = cli::load_project(options)?;
    let view = cli::project_view(&loaded.archive.project)?;

    // §12.3 的最高一级：`ui/switch_main_view`（`app:admin`）/ `ui/force_save`
    // （`app:save`）/ `ui/reload_engine`（`app:reload-engine`）要它。`ui:inject` 也在
    // 这一级的 scope 集合里，但**生产模式**会让它在服务端被硬拒 —— 这正是我们要的：
    // 作用域给全、能力由模式闸住（与领域控制面 `ScopeSet::all()` + `RunMode::Production` 同款）。
    let permission = Permission::Administrative;
    let wiring = LiveWiringOptions {
        permission,
        // 底部控制台 Tab 0 = 卷帘（与 `--headless-idle` / 默认 GUI 一致）。
        console_tab: 0,
        // `ui/force_save` 的落点：`--open` 的那个文件（样本形态没有磁盘对应物 ⇒
        // 该动作会**如实**报错，而不是猜一个文件名）。
        save_path: loaded.source.save_target(),
        // 装配阶段不推量子（`ui/reload_engine` 才建引擎）。
        engine_quanta: 0,
    };
    let live =
        build_live_ui_with(&loaded.archive.project, &wiring).map_err(|error| CliError::Ui {
            detail: format!(
                "[ARCH-UI-004] UI 控制面执行面装配失败: {error} \
             (本档需要能装上 Tier-1 软件光栅化平台: `slint::platform::set_platform` \
             每线程只能装一次, 且必须先于任何 Slint 组件构造)"
            ),
        })?;

    // 证据（**在绑 socket 之前**取，而且走的是**真的** JSON-RPC 管线）：
    // - `ui/tree` 的一次进程内调用：证明这棵运行时控件树真的能被服务（而不是"装好了"）；
    // - 装配时**直接**抓的那一帧（`[MUST-GATE-015]` 的 Tier-1 路径）：证明真的光栅化过。
    // 这些都不是回显参数。
    let mut plane = live.into_production_control_plane(permission);
    let viewport = plane.viewport();
    let reference = plane.reference().clone();
    let (tree, _) = plane.plane().tree().map_err(|error| CliError::Ui {
        detail: format!("[ARCH-UI-004] 绑定之前的进程内 `ui/tree` 探针失败: {error}"),
    })?;
    let tree_nodes = tree.count;

    let mount = UiHttpMount::bind_loopback(plane.into_service()).map_err(|error| CliError::Ui {
        detail: format!("[ARCH-SEC-002] UI 控制面环回绑定失败: {error}"),
    })?;
    let endpoint = mount.endpoint();

    // 令牌落盘 —— **fail closed**：写不进去就没有任何人能鉴权，那么监听口不该存在。
    // 先把监听口释放掉，再报错（而不是让 `mount` 随着 `?` 静默 drop 之后再报错 ——
    // 那样输出里就没有"已停止并释放"这句话，用户无法从报告里分辨两种结局）。
    let token_path = match mount.publish_token() {
        Ok(path) => path,
        Err(error) => {
            mount.stop();
            cli::emit(&[format!(
                "ui-mcp-http: 会话令牌落盘失败 ({error}) ⇒ 监听口已停止并释放 {endpoint}"
            )]);
            return Err(CliError::Ui {
                detail: format!(
                    "会话令牌无法写到 `~/.yeban/session.token` (0600): {error} —— \
                     fail closed: 没有令牌的监听口只是多出来的攻击面, 已释放 {endpoint}"
                ),
            });
        }
    };

    let mut lines = vec![HANDSHAKE.to_owned()];
    lines.extend(cli::project_report(&loaded));
    lines.extend(cli::view_report(&view));
    lines.push(format!(
        "ui-mcp-http-surface: tier1 {}x{} non-black-pixels={} distinct-colors={} tree-nodes={}",
        viewport.width,
        viewport.height,
        reference.non_black_pixels(),
        reference.distinct_color_count(),
        tree_nodes,
    ));
    lines.push(format!(
        "ui-mcp-http: UI 控制面已监听 {endpoint} \
         (只绑环回; 生产模式 ⇒ ui:inject 族硬禁 403 forbidden-in-production; \
          权限 = administrative; 端点路径与领域 MCP 的 /mcp 刻意不同; 令牌 = 256-bit)"
    ));
    lines.push(format!(
        "ui-mcp-http: 会话令牌已写 {} (权限 0600, 内容不打印)",
        token_path.display()
    ));
    match options.idle_seconds {
        Some(seconds) => lines.push(format!(
            "ui-mcp-http: 服务中 ({seconds} 秒后自动停机并释放; Ctrl-C 也可以)"
        )),
        None => lines.push(
            "ui-mcp-http: 服务中 (直到进程被终止: Ctrl-C / kill; 加 --idle-seconds <N> 可自动停机)"
                .to_owned(),
        ),
    }
    cli::emit(&lines);

    // 服务：一拍一拍地泵（执行面是 `!Send` 的，所以服务就在**这条线程**上，
    // 见 `yeban-ui-mcp::transport::mount` 的差别表）。
    let deadline = options
        .idle_seconds
        .map(|seconds| Instant::now() + Duration::from_secs(u64::from(seconds)));
    let served = serve_until(&mount, deadline);
    mount.stop();

    Ok(vec![
        format!(
            "ui-mcp-http: 已停机并释放 {endpoint} \
             (监听 socket 已 close; 本进程一共服务 {served} 个连接)"
        ),
        BOUNDARY.to_owned(),
    ])
}

/// 泵到时限为止，返回服务过的连接数。
///
/// `deadline` 为 `None` ⇒ 一直服务到进程被终止（Ctrl-C / `kill`）—— 这是产品形态；
/// 给了秒数就是自动化形态（跑完自己停机并释放）。
///
/// **连接级失败不结束服务**（与 `crate::mcp_mount` 的工作线程同款）：一次
/// `UnexpectedEof`（客户端写到一半就关）不是停机理由。退避一下继续，避免"accept 坏掉"
/// 变成把机器一个核打满的忙等。
fn serve_until(mount: &UiHttpMount, deadline: Option<Instant>) -> u64 {
    let mut served = 0_u64;
    loop {
        // `let-chains`（edition 2024）：与 `clippy::collapsible_if` 的要求一致。
        if let Some(deadline) = deadline
            && Instant::now() >= deadline
        {
            return served;
        }
        match mount.pump() {
            // 队列里可能还有 ⇒ 立刻再泵一次，不睡。
            Ok(true) => served = served.saturating_add(1),
            Ok(false) => thread::sleep(POLL_IDLE),
            Err(_) => thread::sleep(ACCEPT_RETRY_BACKOFF),
        }
    }
}
