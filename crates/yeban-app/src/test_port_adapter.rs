//! `yeban-app` ↔ `yeban-ui-test-port` 的**单向适配器**（app 侧）。
//!
//! 规范来源 (Normative)：
//! - `[UI-TEST-001]` UI/UX §12.2 语义元素寻址：控件树断言必须建立在稳定语义 ID 上；
//! - `[UI-MCP-002]` §12.5 动态区域遮罩：遮罩矩形必须来自元素树元数据；
//! - `[MUST-GATE-015]` 路线图 §5：Golden 图必须由 Tier-1 软件光栅化产出，尺寸非零且非全黑。
//!
//! ## 为什么本文件是一个 `[[test]]` 目标（而不是 `lib` 模块）
//!
//! 依赖方向是 `yeban-app`（feature `ui-test-port`，默认关闭）→ `yeban-ui-test-port`，
//! 所以适配器必须住在 app 侧。而把 `src/test_port_adapter.rs` 变成 `lib` 模块需要在
//! `crates/yeban-app/src/lib.rs` 里加一行
//! `#[cfg(feature = "ui-test-port")] pub mod test_port_adapter;` ——
//! **`lib.rs` 不在本工作线的授权范围内**（本线只被授权新增本文件 + 修改 `Cargo.toml`）。
//! 因此这里用 Cargo 的标准做法：在 `Cargo.toml` 里声明一个 `[[test]]` 目标指向本文件，
//! 并用 `required-features = ["ui-test-port"]` 把它锁在开关后面（默认构建不编译它）。
//!
//! 代价（已登记为 needs，见 `docs/ledger/ui-test-port-notes.md`）：适配器目前对其它代码不可见。
//! 若要让它成为可复用的 `lib` 模块，需要集成者在 `lib.rs` 加那一行 —— **本线不代签**。
//!
//! ## 本文件证明什么 / 不证明什么
//!
//! 证 明（在启用 feature 的前提下，`cargo test -p yeban-app --features ui-test-port`）：
//! - 184 条注册表条目能被**无损**适配成语义控件树（ID / 角色 / 标签 / 动态标记逐条对齐）；
//! - 13 个 `.slint` 真的能被 **Tier-1 软件光栅化**渲染出**非零、非全黑**的像素
//!   （`live_main_window_renders_tier1_pixels_and_enforces_permissions`，**不依赖**控件树）；
//! - `[UI-MCP-001]` 的 `ReadOnly` 闸门在真实端口上拒绝事件注入。
//!
//! ## 控件树的事实源与它的前置条件（按集成者裁决）
//!
//! - **权威事实源是注册表**（[`registry_to_tree`]）：它是纯 Rust、确定性、无窗口也能工作的。
//!   `dump_json()` 的默认输出与产物 `app-registry-control-tree.json` 都来自它。
//! - **运行时交叉核对是独立的能力**（`runtime_control_tree_cross_check_against_the_registry`），
//!   它的前置条件是被内省的 `.slint` 在**编译期**打开了 debug info ——
//!   而上游默认关闭（`i-slint-compiler-1.18.1/lib.rs:282`），本仓库
//!   `crates/yeban-app/build.rs` 也**没有**打开。因此这条判据现在会：
//!   ① 打一行机器可读的 `RUNTIME-TREE-CAPABILITY: unavailable: …`（写到进程 fd 2，绕过
//!   libtest 捕获）；② **红**，并在消息里给出根因与两种修法。
//!   "能力缺失时静默跳过"在本仓库等于假绿（守卫 G13 对 PyYAML 缺失就是这么处理的），
//!   而这里的能力缺口还会连带挡住 §12.3 的属性读取、§12.5 的动态遮罩与事件注入所需的
//!   绝对坐标 —— 所以它必须红到有人修 `build.rs`，而不是被降级成 SKIP。
//!
//! 不证明：像素**长什么样**（没有基准图，`[UI-MCP-003]` 的分平台 Golden 需要人类先提交基准）；
//! 也不证明"UI 没有视觉缺陷" —— 它证明的是"UI 真的被渲染过"。

use yeban_app::elements::ElementRegistry;
use yeban_app::scene::DemoScene;
use yeban_app::ui::MainWindow;
use yeban_ui_test_port::image::Size;
use yeban_ui_test_port::port::{Permission, PortError, UiTestPort};
use yeban_ui_test_port::tree::{ControlNode, ControlTree, Role, TreeError};
use yeban_ui_test_port::{
    GoldenEvidence, LivePort, compare_with_dynamic_masking, golden_evidence, mask_rects_from_tree,
};

/// 把 `yeban-app` 的静态语义元素注册表适配成 test-port 的控件树。
///
/// 这是一个**纯函数**：注册表 → 树，不做 I/O、不碰 Slint、没有几何
/// （静态注册表里没有包围盒，`bounds` 一律 `None` —— 不编造数字）。
///
/// 与 `yeban-app` 注册表的**不匹配之处**（只适配，不改 app 侧代码）：
/// `ElementMeta::component`（定义该 ID 的 `.slint` 文件）在 test-port 的节点模型里**没有对应字段**，
/// 因此断言失败时无法从 JSON 直接映射回源码文件。已登记为 needs。
fn registry_to_tree(registry: &ElementRegistry) -> Result<ControlTree, TreeError> {
    let mut tree = ControlTree::new();
    for meta in registry.iter() {
        let role = Role::parse(meta.kind.accessible_role())?;
        let node = if meta.dynamic_region {
            ControlNode::new(meta.id.clone(), role, meta.label.clone()).as_dynamic()
        } else {
            ControlNode::new(meta.id.clone(), role, meta.label.clone())
        };
        tree.insert(node)?;
    }
    Ok(tree)
}

/// 默认演示场景下**必然可见**的语义 ID 最小集合（`arrangement_view = true`、`compact = false`）。
///
/// 上游 Testing Backend 的 `visit_descendants` 只访问**几何可见**的元素
/// （`ItemRc::is_visible()` = 绝对裁剪矩形与元素几何相交，见 `i-slint-core-1.18.1/item_tree.rs:410`），
/// 所以运行时树是"当前可见元素"的集合，而注册表是"UI 声明过的全部元素"的集合。
/// 这两个集合本来就不该相等 —— 本判据只钉住"默认视图的关键部件真的在树里"。
///
/// **刻意只收非重复元素的 ID**：`ElementHandle::accessible_id()` 在上游有一句
/// `if self.element_index != 0 { return None; }`，而 `for` 循环展开的元素是否走
/// `element_count > 1` 这条路径、从而只有第一个实例能被语义 ID 寻址，**本机无法核验**
/// （不能编译 Slint）。因此 `track-{i}-header` / `note-{ulid}-rect` / `clip-{ulid}-header`
/// 这些重复族**不在这里断言**，改为在下面打印观察值 —— 见
/// `docs/ledger/ui-test-port-notes.md` 的 pending（这是必须在 CI 上跑一次才能定的问题）。
const DEFAULT_VIEW_MUST_HAVE: [&str; 9] = [
    "ai-rail",
    "arrangement-ruler",
    "status-bar",
    "status-bar-selection",
    "transport-bpm-field",
    "transport-play-button",
    "transport-record-button",
    "transport-stop-button",
    "transport-timecode",
];

/// 默认视图下**不可见**的语义 ID 样本（会话视图的画布）。
/// 它们不得出现在运行时树里 —— 否则说明"可见性"根本没被尊重。
const DEFAULT_VIEW_MUST_NOT_HAVE: [&str; 1] = ["workspace-session-canvas"];

fn demo_registry() -> ElementRegistry {
    ElementRegistry::demo()
}

/// 判据 1: 适配必须是**无损的双射** —— 条目数、ID、角色、标签、动态标记逐条对齐。
#[test]
fn adapter_mirrors_the_registry_one_to_one() {
    let registry = demo_registry();
    let tree = registry_to_tree(&registry).expect("注册表必须能适配");

    assert_eq!(tree.len(), registry.len(), "适配不得丢条目/造条目");
    assert!(
        registry.len() >= 100,
        "注册表条目数异常偏少: {}",
        registry.len()
    );

    for meta in registry.iter() {
        let node = tree
            .find_by_id(&meta.id)
            .unwrap_or_else(|| panic!("丢失 ID {}", meta.id));
        assert_eq!(
            node.role.as_str(),
            meta.kind.accessible_role(),
            "角色漂移: {}",
            meta.id
        );
        assert_eq!(node.label, meta.label, "标签漂移: {}", meta.id);
        assert_eq!(
            node.dynamic_region, meta.dynamic_region,
            "动态标记漂移: {}",
            meta.id
        );
        assert_eq!(node.bounds, None, "静态注册表不该有几何: {}", meta.id);
        assert_eq!(node.parent, None, "静态注册表没有层级信息: {}", meta.id);
    }

    let dynamic_in_tree = tree
        .dynamic_regions()
        .map(|node| node.id.clone())
        .collect::<Vec<_>>();
    let dynamic_in_registry = registry
        .dynamic_regions()
        .map(|meta| meta.id.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        dynamic_in_tree, dynamic_in_registry,
        "动态区清单必须逐条一致"
    );
    assert!(
        dynamic_in_tree.len() >= 10,
        "动态区只有 {} 条, 疑似漏登记",
        dynamic_in_tree.len()
    );
}

/// 判据 2: 语义 ID 的格式规则在两侧是**同一条规则**（依赖方向不允许复用实现，因此逐条对账）。
#[test]
fn id_format_rule_matches_yeban_app() {
    let registry = demo_registry();
    for meta in registry.iter() {
        assert_eq!(
            yeban_app::elements::is_well_formed_id(&meta.id),
            yeban_ui_test_port::tree::is_well_formed_id(&meta.id),
            "两份 ID 格式实现出现分歧: {}",
            meta.id
        );
        assert!(
            yeban_app::elements::is_well_formed_id(&meta.id),
            "注册表 ID 必须格式良好"
        );
    }
    for probe in [
        "",
        "-x",
        "x-",
        "a--b",
        "track_0",
        "track 0",
        "音符-0",
        "note-01J8Z-rect",
    ] {
        assert_eq!(
            yeban_app::elements::is_well_formed_id(probe),
            yeban_ui_test_port::tree::is_well_formed_id(probe),
            "两份实现分歧: `{probe}`"
        );
    }
}

/// 判据 3: 注册表用到的**每一个** `ElementKind` 都必须落在 Slint `AccessibleRole` 的全集里
/// （拼错一个角色就会让运行时控件树构建整条失败）。
#[test]
fn every_registry_role_is_a_slint_accessible_role() {
    let registry = demo_registry();
    let mut seen = std::collections::BTreeSet::new();
    for meta in registry.iter() {
        let raw = meta.kind.accessible_role();
        Role::parse(raw).unwrap_or_else(|err| panic!("`{raw}` 不是合法角色: {err}"));
        seen.insert(raw);
    }
    assert!(
        seen.len() >= 15,
        "注册表只用到 {} 种角色, 覆盖异常",
        seen.len()
    );
    assert!(
        seen.contains("progress-indicator"),
        "动态区必须有 progress-indicator 一类的角色"
    );
}

/// 判据 4: 静态树**没有几何**，因此 `[UI-MCP-002]` 的遮罩在它上面必须**显式报错**，
/// 而不是"遮 0 个矩形"然后静默通过。
///
/// 这条判据是"动态标记 + 几何必须合并"的证明：几何只能来自运行时树（`LivePort`），
/// 动态标记只能来自注册表（本文件），任何一侧单独用都不够。
#[test]
fn static_tree_cannot_mask_without_geometry() {
    let tree = registry_to_tree(&demo_registry()).expect("适配");
    let err = mask_rects_from_tree(&tree).expect_err("静态树没有几何, 必须报错");
    assert!(
        matches!(err, TreeError::DynamicRegionWithoutBounds { .. }),
        "错误类型必须指明原因, 实际 {err:?}"
    );
    assert!(tree.mask_rects().is_err(), "静态树的 mask_rects 必须报错");
}

/// 判据 5: 控件树 JSON 必须逐字节稳定、可被反序列化回等价模型（CI 断言脚本读的就是它）。
#[test]
fn control_tree_json_is_stable_and_round_trips() {
    let registry = demo_registry();
    let first = registry_to_tree(&registry).expect("适配");
    let second = registry_to_tree(&registry).expect("适配");
    let json = first.dump_json();
    assert_eq!(json, second.dump_json(), "同一注册表两次导出必须逐字节相同");
    assert_eq!(ControlTree::from_json(&json).expect("往返"), first);
    assert!(json.ends_with("}\n"));
    assert!(json.contains("\"track-0-fader\"") || json.contains("track-0-"));
}

/// **本文件的核心里程碑（像素那一半）**：让 13 个 `.slint` 真的被 Tier-1 软件光栅化渲染一次，
/// 并断言 `[MUST-GATE-015]` 的两条门槛（尺寸非零、非全黑）+ `[UI-MCP-001]` 的权限闸门。
///
/// 这一条**不依赖运行时控件树**（因此也不依赖编译期 debug info），所以它总是可执行的 ——
/// 它就是 ADR-0001 D18 里那句"当前界面只是编译通过，从未被渲染器或人眼看过"的关闭动作。
/// 控件树那一半是 [`runtime_control_tree_cross_check_against_the_registry`]，它有前置条件。
#[test]
fn live_main_window_renders_tier1_pixels_and_enforces_permissions() {
    let scene = DemoScene::demo();
    let registry = demo_registry();
    let static_tree = registry_to_tree(&registry).expect("注册表适配");
    let size = Size::new(scene.viewport_width, scene.viewport_height);

    // 与 src/main.rs 的 GUI 路径注入同一组演示数据（全部是标量属性）。
    let mut port = LivePort::new(size, Permission::ReadOnly, Some(&static_tree), || {
        let ui = MainWindow::new()?;
        ui.set_timecode(scene.timecode.into());
        ui.set_bpm_display(scene.bpm_display.into());
        ui.set_branch_name(scene.branch_name.into());
        ui.set_arrangement_view(scene.arrangement_by_default);
        ui.set_playing(false);
        ui.set_console_tab(0);
        ui.set_compact(scene.compact());
        Ok(ui)
    })
    .expect("Tier-1 平台 + MainWindow + 运行时控件树");

    // ---- `[MUST-GATE-015]` Tier-1 像素证据 ----
    let image = port.window().capture().expect("Tier-1 截图");
    let evidence: GoldenEvidence =
        golden_evidence(&image).expect("[MUST-GATE-015] 尺寸非零且非全黑");
    assert_eq!(evidence.size, size, "截图尺寸必须等于窗口尺寸");
    assert!(evidence.non_black_pixels > 0, "全黑截图: {evidence:?}");
    assert!(
        evidence.distinct_colors >= 8,
        "颜色过少, 疑似只画了背景: {evidence:?}"
    );
    report_evidence(&evidence);

    // 过程产物（落在 target/ 下, 不提交）。
    let png_path =
        yeban_ui_test_port::write_artifact("app-main-window-arrangement-1920x1080", &image);
    match png_path {
        Ok(path) => eprintln!("yeban-app 主窗口截图: {}", path.display()),
        Err(err) => eprintln!("截图落盘失败（不影响判据）: {err}"),
    }
    // 控件树的**权威事实源是注册表**（无窗口也能工作、确定性、由构造方注入）：
    // 这一份才是 `dump_json()` 的默认输出。运行时抠出来的那一份另存为观察值 ——
    // 它在没有编译期 debug info 时会是 `{}`（见下面那条判据）。
    let json_path = yeban_ui_test_port::artifact_dir().join("app-registry-control-tree.json");
    std::fs::write(&json_path, static_tree.dump_json())
        .unwrap_or_else(|err| panic!("写注册表控件树 JSON 失败 {}: {err}", json_path.display()));
    let runtime_json = yeban_ui_test_port::artifact_dir().join("app-runtime-control-tree.json");
    std::fs::write(&runtime_json, port.tree().dump_json())
        .unwrap_or_else(|err| panic!("写运行时控件树 JSON 失败 {}: {err}", runtime_json.display()));
    eprintln!(
        "控件树 JSON: 注册表 {} 条 -> {}; 运行时 {} 条 -> {}",
        static_tree.len(),
        json_path.display(),
        port.tree().len(),
        runtime_json.display()
    );

    // ---- `[UI-MCP-001]` 真实端口上的权限行为 ----
    assert_eq!(port.permission(), Permission::ReadOnly);
    assert!(matches!(
        port.dispatch_pointer_down(
            "transport-play-button",
            1.0,
            1.0,
            yeban_ui_test_port::PointerButton::Left
        ),
        Err(PortError::PermissionDenied { .. })
    ));
    assert!(matches!(
        port.dispatch_key_press(yeban_ui_test_port::KeyCode::Tab),
        Err(PortError::PermissionDenied { .. })
    ));
    // ReadOnly 允许的截图（不依赖运行时控件树）。
    let png = port.capture_png().expect("只读允许截图");
    assert!(
        png.starts_with(&yeban_ui_test_port::png::PNG_SIGNATURE),
        "capture_png 必须返回真 PNG"
    );
    // 属性读取依赖运行时元素（`read_property` 先查运行时树），因此它的正向判据在
    // 下面那条"运行时控件树"判据里 —— 这里不做会假绿的降级。
}

#[test]
fn runtime_control_tree_cross_check_against_the_registry() {
    // 这条判据只在"运行时控件树**确实可用**"时才有意义 ⇒ 它的前置条件是
    // `crates/yeban-app` 的 `.slint` 在编译期打开了 debug info（见 notes §2 第 27 条）。
    // 前置条件不满足时它**必须红**（而不是静默跳过）：语义寻址 / 属性读取 / 动态遮罩
    // 在真实界面上都依赖运行时几何。
    let scene = DemoScene::demo();
    let registry = demo_registry();
    let static_tree = registry_to_tree(&registry).expect("注册表适配");
    let size = Size::new(scene.viewport_width, scene.viewport_height);
    let mut port = LivePort::new(size, Permission::ReadOnly, Some(&static_tree), || {
        let ui = MainWindow::new()?;
        ui.set_timecode(scene.timecode.into());
        ui.set_bpm_display(scene.bpm_display.into());
        ui.set_branch_name(scene.branch_name.into());
        ui.set_arrangement_view(scene.arrangement_by_default);
        ui.set_playing(false);
        ui.set_console_tab(0);
        ui.set_compact(scene.compact());
        Ok(ui)
    })
    .expect("Tier-1 平台 + MainWindow");

    // ---- `[UI-TEST-001]` 运行时控件树 vs 静态注册表 ----
    //
    // ⚠️ 前置条件：`ElementHandle` 的遍历依赖**编译期 debug info**。上游默认是关闭的
    // （`i-slint-compiler-1.18.1/lib.rs:282`：`debug_info = env::var_os("SLINT_EMIT_DEBUG_INFO").is_some()`），
    // 而 `crates/yeban-app/build.rs` 用的是 `slint_build::compile(...)` —— 默认关闭。
    // 因此在这里，`yeban-app` 的控件树会是**空的**：这不是本 crate 的缺陷，而是
    // `[ARCH-UI-005]` / `[UI-TEST-001]` 在这条链路上**尚未闭环**的确切位置。
    // 本判据故意在此失败，并把根因与两种修法写在断言消息里。
    if port.tree().is_empty() {
        // 能力缺失必须**出声**且**不得算作通过**（静默跳过 = 假绿）。
        // 机器可读前缀便于在 CI 日志里 grep：`RUNTIME-TREE-CAPABILITY:`。
        report_capability(
            "unavailable: crates/yeban-app 的 .slint 没有编译期 debug info ⇒ ElementHandle \
             遍历拿到一棵空树 (见 docs/ledger/ui-test-port-notes.md §2 #27 / §10 needs 0)",
        );
        panic!(
            "[UI-TEST-001] 运行时控件树为空 ⇒ 前置条件不满足: `crates/yeban-app` 的 .slint 没有编译期 debug info。\n\
             根因: i-slint-compiler-1.18.1/lib.rs:282 `debug_info = env::var_os(\"SLINT_EMIT_DEBUG_INFO\").is_some()` 默认关闭,\n\
                   而 crates/yeban-app/build.rs 用的是 `slint_build::compile(...)`;\n\
                   `i-slint-backend-testing` 的 ElementHandle 遍历需要它 (item.element_count() 返回 None ⇒ 空树)。\n\
             影响: §12.2 语义寻址 / §12.3 属性读取 / §12.5 动态遮罩在**真实界面**上都还无法执行 ——\n\
                   注入事件需要元素的绝对坐标, 而坐标只能来自运行时几何。\n\
             修法 (二选一, 都不在 yeban-ui-test-port 的授权文件范围内):\n\
               (a) crates/yeban-app/build.rs 改用 `compile_with_config(\"ui/app.slint\",\n\
                   CompilerConfiguration::new().with_debug_info(true))`;\n\
               (b) CI 的构建步骤加环境变量 `SLINT_EMIT_DEBUG_INFO=1`。\n\
             本 crate 自己的 Tier-1 夹具已经这么做 (build.rs + ui/fixture.slint), 并在 CI 上真的\n\
             抠出了非空控件树 —— 所以这不是"无头与窗口实例互相矛盾", 而是**一个编译期开关没打开**。\n\
             详见 docs/ledger/ui-test-port-notes.md §2 第 27/28 条与 §10 needs 第 0 条。"
        );
    }
    let coverage = port.tree().coverage_against(&static_tree);
    assert!(
        coverage.unknown_at_runtime.is_empty(),
        "UI 里出现了没登记的 accessible-id（双向契约被破坏）: {}",
        coverage.unknown_at_runtime.join(", ")
    );
    assert!(
        !coverage.missing_at_runtime.is_empty(),
        "运行时树包含了注册表全部条目 —— 说明可见性/裁剪没有被尊重, `visible: false` 的分支也在树里"
    );
    assert!(
        coverage.missing_at_runtime.len() < static_tree.len(),
        "运行时树一条都没抓到 ({}/{})",
        coverage.missing_at_runtime.len(),
        static_tree.len()
    );
    assert!(
        port.tree().len() >= 30,
        "运行时树只有 {} 个节点, 布局疑似没有生效",
        port.tree().len()
    );
    // 观察值（**不是**断言）：重复元素族是否可被语义 ID 寻址。若这里打印出 0，
    // 就说明上游 `accessible_id()` 对 `element_index != 0` 返回 None，
    // `[UI-TEST-001]` 的 `for` 循环寻址需要另找机制（`find_by_element_id` + 索引）。
    let track_headers = port
        .tree()
        .with_prefix("track-")
        .filter(|node| node.id.ends_with("-header"))
        .count();
    let note_rects = port
        .tree()
        .with_prefix("note-")
        .filter(|node| node.id.ends_with("-rect"))
        .count();
    eprintln!(
        "观察值: 运行时树里 track-*-header = {track_headers}, note-*-rect = {note_rects} \
         (为 0 即上游 element_index != 0 不暴露 accessible-id, 见 notes pending)"
    );
    for id in DEFAULT_VIEW_MUST_HAVE {
        assert!(
            port.tree().contains(id),
            "默认视图缺少必然可见的 `{id}` ({coverage})"
        );
    }
    for id in DEFAULT_VIEW_MUST_NOT_HAVE {
        assert!(!port.tree().contains(id), "不可见的 `{id}` 混进了运行时树");
    }

    // ---- `[UI-MCP-002]` + `[UI-MCP-003]` 遮罩与 SSIM ----
    let rects = mask_rects_from_tree(port.tree()).expect("运行时动态区必须有几何");
    assert!(
        !rects.is_empty(),
        "运行时树里没有任何动态区 —— [UI-MCP-002] 无从执行"
    );
    let targets: Vec<_> = rects
        .iter()
        .copied()
        .filter(|rect| !rect.is_empty() && rect.intersect(size).is_some())
        .collect();
    assert!(!targets.is_empty(), "所有动态区都被裁到屏幕外: {rects:?}");

    // 抖动只发生在动态区内 ⇒ 遮罩后必须逐字节相同（SSIM 恒等于 1.0）。
    let mut jittered = image.clone();
    for rect in &targets {
        jittered.fill_rect(*rect, [0x00, 0xff, 0x00]);
    }
    assert_ne!(image.pixels(), jittered.pixels(), "抖动必须真的改到像素");
    // "未遮罩时应当被检出"只在动态区**占画面足够大**时成立：SSIM 是逐窗口均值，
    // 1px 宽的走带光标改 100 来个像素时均值仍然是 1.000000 —— 那不是判据失效，是口径。
    // 因此这里按面积分流：够大就断言变红，太小就打印实测值并说明由 test-port 的夹具判据覆盖。
    let jitter_area: u64 = targets.iter().map(|rect| rect.area()).sum();
    let unmasked = yeban_ui_test_port::ssim::ssim(&image, &jittered).expect("同尺寸");
    if jitter_area * 100 >= size.pixel_count() {
        assert!(
            !yeban_ui_test_port::ssim::Verdict::with_default_threshold(unmasked).passed,
            "动态区占画面 ≥1% 时未遮罩必须被检出, 实测 SSIM {unmasked:.6}"
        );
    } else {
        eprintln!(
            "动态区总面积仅 {jitter_area} px (<1% 画面), 未遮罩 SSIM={unmasked:.6};              该方向由 yeban-ui-test-port 的夹具判据 (masking_absorbs_dynamic_jitter...) 覆盖"
        );
    }
    let masked = compare_with_dynamic_masking(&image, &jittered, port.tree()).expect("遮罩比对");
    assert!(masked.passed, "遮罩后必须通过: {masked}");
    assert!(
        (masked.score - 1.0).abs() <= f64::EPSILON,
        "遮罩区内的差异必须被完全吸收, 实测 {}",
        masked.score
    );

    // 静态区被整块抹掉 ⇒ 遮罩之后仍然必须低于阈值（遮罩不能把界面变成盲区）。
    let mut regressed = image.clone();
    regressed.fill_rect(
        yeban_ui_test_port::image::Rect::new(400, 300, 400, 300),
        [0, 0, 0],
    );
    let verdict = compare_with_dynamic_masking(&image, &regressed, port.tree()).expect("遮罩比对");
    assert!(!verdict.passed, "静态区大改必须被检出: {verdict}");

    // ---- `[UI-MCP-001]` ReadOnly 的属性读取（需要运行时元素） ----
    let role = port
        .read_property("transport-play-button", "role")
        .expect("role 可读");
    Role::parse(&role).unwrap_or_else(|err| panic!("运行时角色非法: {role} ({err})"));
    let width = port
        .read_property("transport-play-button", "width")
        .expect("width 可读");
    assert!(
        width.parse::<f64>().is_ok_and(|value| value > 0.0),
        "实测 width={width}"
    );
    assert!(matches!(
        port.read_property("transport-play-button", "no-such-property"),
        Err(PortError::Rejected { .. })
    ));
}
