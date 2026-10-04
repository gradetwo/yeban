//! **真实界面上的 UI 控制面** —— 端到端判据（`cargo test -p yeban-app --all-targets` 会跑它）。
//!
//! 这一条工作线要证明的事只有一句：**AI 能在活的 `MainWindow` 上看见界面**。
//! 本文件把这句话拆成可失败的判据：
//!
//! | 判据 | 证明 |
//! | :--- | :--- |
//! | `live_control_plane_reads_the_project_backed_window_end_to_end` | 建立真实执行面 → `ui/tree` → `ui/node`（按**语义 ID**）→ `ui/screenshot` 四步全通；节点标签里是**工程**（`filled_project()`）的轨道名；截图尺寸非零、非全黑，且与"直接从窗口抓的那一帧"**逐字节同一份** |
//! | `live_control_plane_coverage_matches_the_projected_registry` | 运行时树 ⊆ 投影注册表（`visible: false` 的分支不在树里 ⇒ 缺失非空），两个方向的差异都被 `ui/coverage` 如实报出来 |
//! | `production_plane_hard_denies_injection_on_the_live_window` | §12.3 / `ARCH-SEC-002`：生产模式下 `ui:inject` 在**真实执行面**上也是 403 硬禁（并且之后的只读调用照常可用） |
//! | `interactive_test_mode_plane_injects_into_the_live_window` | §12.4：测试模式 + `Interactive` 权限下，事件注入真的落进窗口；`ui/methods` 如实报出 `injectAllowed` |
//!
//! ## 为什么判据住在两个文件里而不是这里
//!
//! 判据的**逻辑**住在 `crates/yeban-app/src/live_surface.rs`（接线）与
//! `crates/yeban-ui-mcp/src/live.rs`（`ControlPlane::probe`：树 → 节点 → 截图那条链）。
//! 本文件只做"装配 + 断言 + 留痕"，因此同一段 `probe` 逻辑在**本机**（假执行面，零 Slint）
//! 与 **CI**（真实 `MainWindow`）上跑的是同一份代码 —— 差别只有执行面。
//!
//! ## 依赖从哪来（红线 6 没有被削弱）
//!
//! `yeban-ui-mcp` / `yeban-ui-test-port` 都是 `[dev-dependencies]`：**不进入** release 构建。
//! 核验命令（零编译）：`cargo tree -p yeban-app -e normal --locked`。
//!
//! ## 每个 `#[test]` 只装一次 Tier-1 平台
//!
//! `slint::platform::set_platform` 是**线程局部**的（每个测试一个线程，libtest 默认如此），
//! 因此每个测试各建一个 `LiveUi`；同一个测试里**不能**建第二个。

#[path = "../src/live_surface.rs"]
mod live;

use live::{
    LiveControlPlane, LiveWiringError, LiveWiringOptions, build_live_ui, build_live_ui_with,
};

use yeban_app::elements::is_model_driven_family;
use yeban_app::scene::TRACK_NAMES;
use yeban_ui_mcp::live::{ProbeOptions, family_member_count};
use yeban_ui_mcp::surface::{DEFAULT_MAX_PNG_BYTES, encode_with_evidence};
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::report_line;

/// 装配一条真实控制面（所有判据的唯一入口）。
fn assemble(permission: Permission) -> Result<LiveControlPlane, LiveWiringError> {
    let project = yeban_model::samples::filled_project();
    Ok(build_live_ui(&project, permission)?.into_control_plane(permission))
}

/// 把过程产物写进 `target/ui-test-port/`（`.gitignore` 已忽略；由 CI 的 artifact 步骤上传）。
fn write_artifact_bytes(name: &str, bytes: &[u8]) -> String {
    let dir = yeban_ui_test_port::artifact_dir();
    if let Err(error) = std::fs::create_dir_all(&dir) {
        report_line(&format!("artifact 目录创建失败(不影响判据): {error}"));
        return String::new();
    }
    let path = dir.join(name);
    match std::fs::write(&path, bytes) {
        Ok(()) => path.display().to_string(),
        Err(error) => {
            report_line(&format!("artifact 写入失败(不影响判据): {error}"));
            String::new()
        }
    }
}

/// 判据 1（**本工作线的核心**）：真实窗口上的四步端到端读数，且数据来自工程。
#[test]
fn live_control_plane_reads_the_project_backed_window_end_to_end() {
    let mut plane = assemble(Permission::ReadOnly).expect("真实界面 + Tier-1 执行面");
    // 先只读地把"工程里是什么"抄下来（`ViewState` 不要求 `Clone`，而且下面要用 `&mut plane`）。
    let viewport = plane.viewport();
    let lead_index = plane
        .view()
        .tracks
        .first()
        .expect("filled_project 有轨道")
        .index;
    let lead_name = plane.view().tracks[0].name.clone();
    let track_count = plane.view().tracks.len();
    let section_count = plane.view().sections.len();
    let clip_ids: Vec<String> = plane
        .view()
        .clips
        .iter()
        .map(|clip| clip.placement_id.clone())
        .collect();
    let note_ulids: Vec<String> = plane.view().note_ulids.clone();
    let element_id = format!("track-{lead_index}-header");

    let options = ProbeOptions::new(element_id.clone(), lead_name.clone())
        .expecting_size(viewport.width, viewport.height);
    let probe = plane
        .plane()
        .probe(&options)
        .unwrap_or_else(|error| panic!("端到端 probe 失败: {error}"));
    report_line(&probe.summary());

    // ---- 方向 1：控件树里是**工程**的数据（不是演示常量） ----
    assert_eq!(probe.surface, "tier1-live-port");
    assert!(
        probe.tree.count >= 40,
        "运行时控件树只有 {} 个节点, 布局或 debug info 疑似没生效",
        probe.tree.count
    );
    assert_eq!(probe.node.id, element_id);
    assert_eq!(probe.node.role, "list-item", "轨道包头角色漂移");
    assert_eq!(
        probe.node.label,
        format!("轨道 {lead_name}"),
        "`accessible-label` 必须由工程里的轨道名拼出（`arrangement_view.slint` 的 `\"轨道 \" + track_name`）"
    );
    assert!(
        probe.node.visible == Some(true),
        "运行时树里的节点都是几何可见的: {:?}",
        probe.node.visible
    );
    assert!(
        probe
            .node
            .bounds
            .is_some_and(|rect| rect.width > 0 && rect.height > 0),
        "几何必须来自真实布局: {:?}",
        probe.node.bounds
    );

    // ---- 方向 2：族规模 == 工程规模（3 条非主总线轨道 / 2 段落 / 2 剪辑摆放） ----
    assert_eq!(
        family_member_count(&probe.tree, "track-", "-header"),
        track_count,
        "轨道包头数必须等于工程的轨道数"
    );
    assert_eq!(
        family_member_count(&probe.tree, "section-", "-card"),
        section_count,
        "章节卡片数必须等于工程的段落数"
    );
    assert_eq!(
        family_member_count(&probe.tree, "clip-", "-header"),
        clip_ids.len(),
        "剪辑包头数必须等于工程的剪辑摆放数"
    );
    for placement_id in &clip_ids {
        assert!(
            probe
                .tree
                .find(&format!("clip-{placement_id}-header"))
                .is_some(),
            "[UI-TEST-001] 工程的摆放身份 `{placement_id}` 必须能在控件树里寻址"
        );
    }
    for ulid in &note_ulids {
        assert!(
            probe.tree.find(&format!("note-{ulid}-rect")).is_some(),
            "工程的音符身份 `{ulid}` 必须出现在卷帘控件树里"
        );
    }
    // 负向：由投影驱动的族里**一个演示夹具的名字都不许出现**。
    for node in &probe.tree.nodes {
        if !is_model_driven_family(&node.id) {
            continue;
        }
        for demo in TRACK_NAMES {
            assert!(
                !node.label.contains(demo),
                "工程驱动的 `{}` 里出现了演示夹具的名字 `{demo}`: {:?}",
                node.id,
                node.label
            );
        }
    }

    // ---- 方向 3：截图的实测数字（`[MUST-GATE-015]`） ----
    let shot = &probe.screenshot;
    report_line(&format!(
        "[live-port] 截图实测: {}x{} (IHDR {}x{}) 非黑 {}/{} ({}%) 颜色 {} 种 PNG {} 字节 指纹 {} 遮罩 {} 区 effective={}",
        shot.width,
        shot.height,
        shot.ihdr_width,
        shot.ihdr_height,
        shot.non_black_pixels,
        u64::from(viewport.width) * u64::from(viewport.height),
        shot.non_black_percent(),
        shot.distinct_colors,
        shot.png_bytes,
        shot.fingerprint,
        shot.masked_regions,
        shot.mask_effective
    ));
    assert_eq!((shot.width, shot.height), (viewport.width, viewport.height));
    assert_eq!(
        (shot.ihdr_width, shot.ihdr_height),
        (viewport.width, viewport.height)
    );
    assert!(shot.non_black_pixels > 0, "非全黑");
    assert!(
        shot.distinct_colors >= 8,
        "颜色过少, 疑似只画了背景: {} 种",
        shot.distinct_colors
    );
    // 两条分开写：`a > 0 && a <= N` 会被 `clippy::manual_range_contains` 盯上
    // （CI 的 `-D warnings` 不接受"善意提示"），而这两个文件本机跑不了 clippy。
    assert!(shot.png_bytes > 0, "PNG 不能是 0 字节");
    assert!(
        shot.png_bytes <= DEFAULT_MAX_PNG_BYTES,
        "PNG {} 字节超过单文件上限 {DEFAULT_MAX_PNG_BYTES}",
        shot.png_bytes
    );
    assert_eq!(shot.png_bytes, shot.png.len(), "证据与字节必须是同一份");
    assert!(shot.mask_dynamic, "默认必须遮罩 (§12.5 的 MUST)");
    assert!(
        shot.masked_regions > 0 && shot.mask_effective,
        "动态区必须真的被置黑: {} 区 / effective={}",
        shot.masked_regions,
        shot.mask_effective
    );

    // ---- 方向 4（**最强的一条**）：控制面发出去的 PNG == 直接从窗口抓的那一帧 ----
    //
    // `ui/screenshot` 默认遮罩，所以这里显式要原始帧（`maskDynamic: false`），
    // 再与装配时**直接** `port.window().capture()` 的那一帧各自编码一次，
    // 比较 FNV-1a 指纹 —— 相同就说明"AI 看到的像素"就是窗口产出的那一份，
    // 中间没有第二个渲染器、没有第二份像素来源。
    let raw_probe = plane
        .plane()
        .probe(&ProbeOptions::new(element_id.clone(), lead_name.clone()).without_masking())
        .unwrap_or_else(|error| panic!("原始帧 probe 失败: {error}"));
    let (_reference_png, reference_evidence) =
        encode_with_evidence(plane.reference(), DEFAULT_MAX_PNG_BYTES).expect("对照帧编码");
    assert_eq!(
        format!("{:016x}", reference_evidence.fingerprint),
        raw_probe.screenshot.fingerprint,
        "控制面发出去的原始帧必须与直接从窗口抓的那一帧逐字节相同"
    );
    assert_eq!(
        raw_probe.screenshot.non_black_pixels, reference_evidence.non_black_pixels,
        "同一帧的非黑像素数必须一致"
    );
    assert_ne!(
        raw_probe.screenshot.fingerprint, shot.fingerprint,
        "遮罩必须真的改变发出去的字节（否则 [UI-MCP-002] 的 MUST 是空转）"
    );
    report_line(&format!(
        "[live-port] 未遮罩帧: 指纹 {} 非黑 {} ({}%) 颜色 {} 种; 与窗口直抓帧一致",
        raw_probe.screenshot.fingerprint,
        raw_probe.screenshot.non_black_pixels,
        raw_probe.screenshot.non_black_percent(),
        raw_probe.screenshot.distinct_colors
    ));

    // ---- 方向 5：两次读数逐字节稳定（线格式与证据链都是确定的） ----
    let again = plane.plane().probe(&options).expect("第二次读数必须成功");
    assert_eq!(
        again.tree_json, probe.tree_json,
        "同一执行面两次 `ui/tree` 必须逐字节相同"
    );
    assert_eq!(again.node_json, probe.node_json);
    assert_eq!(again.screenshot.fingerprint, probe.screenshot.fingerprint);

    // ---- 过程产物（人眼复核 + artifact） ----
    report_line(&format!(
        "[live-port] 截图产物: {}",
        write_artifact_bytes(
            "live-port-filled-project-unmasked-1920x1080.png",
            &raw_probe.screenshot.png
        )
    ));
    report_line(&format!(
        "[live-port] 遮罩帧产物: {}",
        write_artifact_bytes("live-port-filled-project-masked-1920x1080.png", &shot.png)
    ));
    report_line(&format!(
        "[live-port] 控件树产物: {}",
        write_artifact_bytes("live-port-control-tree.json", probe.tree_json.as_bytes())
    ));
}

/// 判据 2：运行时树与**投影注册表**的双向覆盖（`[UI-TEST-001]` / `[ARCH-UI-005]`）。
#[test]
fn live_control_plane_coverage_matches_the_projected_registry() {
    let mut plane = assemble(Permission::ReadOnly).expect("真实界面 + Tier-1 执行面");
    let registry_ids: Vec<String> = plane.registry().ids().map(str::to_owned).collect();
    let (tree, _json) = plane.plane().tree().expect("ui/tree");
    let coverage = plane.plane().coverage(&registry_ids).expect("ui/coverage");
    report_line(&format!(
        "[live-port] ui/coverage: 注册表 {} / 运行时 {} / 注册表有而运行时无 {} / 运行时有而注册表无 {}",
        coverage.registry_count,
        coverage.runtime_count,
        coverage.missing_at_runtime.len(),
        coverage.unknown_at_runtime.len()
    ));

    assert_eq!(coverage.registry_count, registry_ids.len());
    assert_eq!(coverage.runtime_count, tree.count);
    assert!(
        coverage.unknown_at_runtime.is_empty(),
        "运行时树里出现了注册表未登记的语义 ID（双向契约破裂）: {:?}",
        coverage.unknown_at_runtime
    );
    assert!(
        !coverage.missing_at_runtime.is_empty(),
        "运行时树包含了注册表全部 {} 条 —— `visible: false` 的过滤没生效",
        registry_ids.len()
    );
    assert!(
        coverage.missing_at_runtime.len() < registry_ids.len(),
        "运行时树一条都没抓到（缺 {}/{}）",
        coverage.missing_at_runtime.len(),
        registry_ids.len()
    );
}

/// 判据 3：生产模式下 `ui:inject` 在**真实执行面**上也被硬禁（403），且只读能力不受影响。
#[test]
fn production_plane_hard_denies_injection_on_the_live_window() {
    let mut plane = assemble(Permission::ReadOnly).expect("真实界面 + Tier-1 执行面");
    let denied = plane.plane().try_line(
        r#"{"jsonrpc":"2.0","id":11,"method":"ui/dispatch_key_press","params":{"keyCode":"Tab"}}"#,
    );
    assert!(denied.is_error(), "生产模式下注入必须被拒: {denied:?}");
    assert_eq!(denied.status, 403);
    assert_eq!(
        denied.kind.as_deref(),
        Some("forbidden-in-production"),
        "拒的理由必须是**生产硬禁**（而不是「实现侧拒绝」）: {denied:?}"
    );

    // 硬禁之后，只读那三件事照常可用（界面没有被这次拒绝弄坏）。
    let probe = plane
        .plane()
        .probe(&ProbeOptions::new("track-0-header", "Lead").without_masking());
    assert!(probe.is_ok(), "硬禁不该影响只读路径: {probe:?}");
}

/// 判据 4：测试模式 + `Interactive` ⇒ 事件注入真的落进活窗口（§12.4）。
#[test]
fn interactive_test_mode_plane_injects_into_the_live_window() {
    let mut plane = assemble(Permission::Interactive).expect("真实界面 + Tier-1 执行面");
    let methods = plane.plane().methods().expect("ui/methods");
    report_line(&format!(
        "[live-port] ui/methods: service={} surface={} mode={} injectAllowed={}",
        methods["service"], methods["surface"], methods["mode"], methods["injectAllowed"]
    ));
    assert_eq!(methods["surface"], "tier1-live-port");
    assert_eq!(methods["mode"], "test");
    assert_eq!(methods["injectAllowed"].as_bool(), Some(true));

    let moved = plane.plane().try_line(
        r#"{"jsonrpc":"2.0","id":12,"method":"ui/dispatch_pointer_move","params":{"x":640.0,"y":360.0}}"#,
    );
    assert!(
        !moved.is_error(),
        "测试模式 + Interactive 下指针注入必须落到真实窗口: {moved:?}"
    );
    assert_eq!(moved.status, 200);
    let result = moved.result.expect("有 result");
    assert_eq!(result["dispatched"], "pointer_move");
    assert_eq!(result["x"], 640.0);

    // 注入之后界面仍然可以被截（尺寸不变、非全黑）。
    let shot = plane
        .plane()
        .probe(&ProbeOptions::new("track-0-header", "Lead"))
        .expect("注入后仍需能读界面")
        .screenshot;
    assert_eq!(
        (shot.width, shot.height),
        (plane.viewport().width, plane.viewport().height)
    );
    assert!(shot.non_black_pixels > 0);
}

// ===========================================================================
// app-mixer 工作线：混音台通道条（投影 + 真实电平）与三个管理动作
// ===========================================================================
//
// 这一半的判据回答两个问题：
//   1. **混音台的通道条真的由工程与引擎驱动吗？**（数量 / 名字 / 数值 / 像素 / 控件树属性）
//   2. **`[UI-MCP-001]` §12.3 的 Administrative 三件事真的发生了吗？**（切视图 / 写盘 / 重建引擎）
//
// 全部判据都住在**一个**测试目标里（本文件）：`src/live_surface.rs` 用 `#[path]` 被本目标
// 装进来编译，而 `-D warnings` 下"没人用的公开方法"是硬错误 —— 一个目标 = 一份确定的使用
// 集合，多目标会让某个目标看不到某个方法（live-port 线 CI 第 1 轮就死在这条上）。

use std::path::PathBuf;

use yeban_app::bridge::{ViewState, demo_project};
use yeban_app::open::open_project_file;
use yeban_engine::meter::{MeterFrame, meter_channel};
use yeban_model::ids::EntityId;
use yeban_model::project::YebanProjectV1;
use yeban_ui_mcp::tree::UiTree;
use yeban_ui_test_port::tree::ControlTree;

/// 装配选项：判据只调三个旋钮（权限 / 控制台 Tab / 每个引擎代的量子数）。
fn options(permission: Permission, console_tab: i32) -> LiveWiringOptions {
    LiveWiringOptions {
        permission,
        console_tab,
        save_path: None,
        engine_quanta: 4,
    }
}

/// 通道条族规模（`track-{i}-channel-strip`）。
fn channel_strips(tree: &ControlTree) -> usize {
    tree.with_prefix("track-")
        .filter(|node| node.id.ends_with("-channel-strip"))
        .count()
}

/// 取某个语义 ID 在**执行面**控件树（`ControlTree`）里的标签。
///
/// 两个助手对应两条**不同的**树类型：`ControlTree` 是 `LivePort` 内省出来的那棵树
/// （`LiveUi::tree_snapshot`），`UiTree` 是 `ui/tree` 把同一棵树投影成线格式的结果。
/// 判据两边都用，所以类型必须分开写 —— 混用是编译错误（CI 第一次就把这一条抓住了）。
fn label_of(tree: &ControlTree, id: &str) -> String {
    tree.find_by_id(id)
        .unwrap_or_else(|| panic!("运行时控件树里没有 `{id}`"))
        .label
        .clone()
}

/// 取某个语义 ID 在**控制面**投影（`ui/tree` 的 `UiTree`）里的标签。
fn label_in(tree: &UiTree, id: &str) -> String {
    tree.find(id)
        .unwrap_or_else(|| panic!("`ui/tree` 的投影里没有 `{id}`"))
        .label
        .clone()
}

/// 一帧像素的 FNV-1a 指纹（与 `ui/screenshot` 的指纹同算法）。
fn frame_fingerprint(image: &yeban_ui_test_port::Rgb8Image) -> String {
    let (_bytes, evidence) =
        encode_with_evidence(image, DEFAULT_MAX_PNG_BYTES).expect("Tier-1 帧必须非零且非全黑");
    format!("{:016x}", evidence.fingerprint)
}

/// 工程里的**非主总线**轨道身份（`BTreeMap` 升序 = 视图顺序）。
fn track_ids(project: &YebanProjectV1) -> Vec<EntityId> {
    project
        .tracks
        .keys()
        .copied()
        .filter(|id| *id != project.master_bus_track_id)
        .collect()
}

/// 一个一次性的临时目录（不引入 `tempfile`：它不是本 crate 的依赖）。
fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-app-live-{tag}-{}-{}",
        std::process::id(),
        EntityId::new().to_canonical_string()
    ));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

/// 判据 5（**混音台**）：通道条数 == 工程轨道数；**同一个活窗口**换工程 ⇒ 名字/数值/像素全变。
#[test]
fn mixer_channel_strips_follow_the_projected_project_on_the_same_window() {
    let demo = demo_project();
    let filled = yeban_model::samples::filled_project();
    let demo_view = ViewState::from_project(&demo).expect("演示投影");
    let filled_view = ViewState::from_project(&filled).expect("样本投影");
    assert_ne!(
        demo_view.tracks.len(),
        filled_view.tracks.len(),
        "两个工程的轨道数必须不同, 否则这条判据没有分辨力"
    );

    // 混音台（控制台 Tab 1）必须是**可见**的，否则它不在运行时控件树里。
    let mut live = build_live_ui_with(&demo, &options(Permission::ReadOnly, 1)).expect("装配");
    let before = live.tree_snapshot();
    assert_eq!(
        channel_strips(&before),
        demo_view.tracks.len(),
        "通道条数必须等于工程的轨道数（而不是 .slint 里写死的 6）"
    );
    assert_eq!(label_of(&before, "track-0-channel-strip"), "通道条 鼓");
    assert!(before.find_by_id("track-5-channel-strip").is_some());
    // 静音 / 独奏 / 色标 / 声相 / 推子都来自投影（演示工程：0 号独奏、5 号静音）。
    assert_eq!(
        label_of(&before, "track-5-mixer-mute-button"),
        "轨道 打击 静音"
    );
    assert_eq!(
        label_of(&before, "track-0-mixer-solo-button"),
        "轨道 鼓 独奏"
    );
    assert_eq!(
        label_of(&before, "track-0-mixer-color-swatch"),
        format!("轨道色标 {}", demo_view.track_color_labels()[0])
    );
    assert_eq!(label_of(&before, "track-0-fader"), "轨道 鼓 推子");
    let frame_before = frame_fingerprint(&live.capture().expect("换工程前抓帧"));

    // ---- 同一个活窗口上换工程 ----
    live.apply_project(&filled).expect("换工程");
    let after = live.tree_snapshot();
    assert_eq!(
        channel_strips(&after),
        filled_view.tracks.len(),
        "换工程之后通道条数必须跟着变"
    );
    assert_eq!(label_of(&after, "track-0-channel-strip"), "通道条 Lead");
    assert!(
        after.find_by_id("track-3-channel-strip").is_none(),
        "filled_project 只有 {} 条轨道, 不该有第 4 条通道条",
        filled_view.tracks.len()
    );
    assert_eq!(
        label_of(&after, "track-0-mixer-color-swatch"),
        format!("轨道色标 {}", filled_view.track_color_labels()[0])
    );
    let frame_after = frame_fingerprint(&live.capture().expect("换工程后抓帧"));
    assert_ne!(
        frame_before, frame_after,
        "换工程必须换像素（否则说明界面读的不是工程）"
    );
    report_line(&format!(
        "[app-mixer] 换工程: 通道条 {} -> {}, 像素指纹 {} -> {}",
        channel_strips(&before),
        channel_strips(&after),
        frame_before,
        frame_after
    ));

    // ---- 端到端：控制面服务的树就是刚看的那棵树，且运行时 ⊆ 注册表 ----
    let mut plane = live.into_control_plane(Permission::ReadOnly);
    let (tree, _json) = plane.plane().tree().expect("ui/tree");
    assert_eq!(tree.count, after.len(), "控制面读到的就是同一个控件树");
    let registry_ids: Vec<String> = plane.registry().ids().map(str::to_owned).collect();
    let coverage = plane.plane().coverage(&registry_ids).expect("ui/coverage");
    report_line(&format!(
        "[app-mixer] ui/coverage(混音台可见): 注册表 {} / 运行时 {} / 未登记 {} / 缺失 {}",
        coverage.registry_count,
        coverage.runtime_count,
        coverage.unknown_at_runtime.len(),
        coverage.missing_at_runtime.len()
    ));
    assert!(
        coverage.unknown_at_runtime.is_empty(),
        "混音台的语义 ID 必须全部在注册表里（未登记 == 契约破裂）: {:?}",
        coverage.unknown_at_runtime
    );
    let probe = plane
        .plane()
        .probe(&ProbeOptions::new("track-0-channel-strip", "Lead").without_masking())
        .expect("按语义 ID 读通道条");
    assert_eq!(probe.node.label, "通道条 Lead");
    assert_eq!(probe.node.role, "groupbox");
    assert_eq!(
        family_member_count(&probe.tree, "track-", "-channel-strip"),
        filled_view.tracks.len()
    );
}

/// 判据 6（**电平真的被消费**）：注入一组已知 `MeterFrame` ⇒ 控件树里的 dBFS 与之一致。
///
/// 容差与理由：`MeterFrame` 的幅度是线性的，dBFS = `20·log10(a)`；界面上显示的是
/// `{:.1}` 的文本，因此"显示值 vs 真值"的偏差上界是 **0.05 dB**（半个显示步长）。
/// 判据同时钉住两边：文本**逐字**相等（`"0.0"` / `"-6.0"` / `"-120.0"`），以及
/// 未取整的浮点值与解析式相差 < 1e-3 dB。只断言其中一个都会漏掉一类错。
#[test]
fn mixer_meter_labels_match_the_injected_frames() {
    let project = demo_project();
    let ids = track_ids(&project);
    assert!(ids.len() >= 3, "演示工程至少 3 条非主总线轨道");

    let mut live = build_live_ui_with(&project, &options(Permission::ReadOnly, 1)).expect("装配");
    let (mut publisher, collector) = meter_channel(64);
    live.adopt_meter_collector(collector);
    publisher.publish(&[
        // 满幅 ⇒ 0.0 dBFS
        MeterFrame::new(ids[0], 5, 1.0, 1.0),
        // 半幅 ⇒ 20·log10(0.5) = -6.0206 dBFS ⇒ 显示 "-6.0"
        MeterFrame::new(ids[1], 5, 0.5, 0.25),
        // 静音 ⇒ 负无穷 ⇒ 显示下限 "-120.0"
        MeterFrame::new(ids[2], 5, 0.0, 0.0),
        // 母线单独一条（立体声联动的读数）
        MeterFrame::new(project.master_bus_track_id, 5, 1.0, 1.0),
    ]);
    let snapshot = live.pump_meters();
    assert_eq!(snapshot.quantum, Some(5), "抽到的是最新的那个量子");
    assert_eq!(snapshot.nodes_seen, 4);
    assert_eq!(snapshot.peak_labels()[0], "0.0");
    assert_eq!(snapshot.peak_labels()[1], "-6.0");
    assert_eq!(snapshot.peak_labels()[2], "-120.0");
    assert_eq!(snapshot.master_peak_label(), "0.0");
    assert!(
        (snapshot.tracks[1].peak_dbfs + 6.020_6).abs() < 1e-3,
        "未取整的峰值 dBFS 必须就是 20·log10(0.5): {}",
        snapshot.tracks[1].peak_dbfs
    );
    assert!(
        (snapshot.tracks[1].rms_dbfs + 12.041_2).abs() < 1e-3,
        "未取整的 RMS dBFS 必须就是 20·log10(0.25): {}",
        snapshot.tracks[1].rms_dbfs
    );
    assert_eq!(snapshot.tracks[0].level, 1.0, "满幅 ⇒ 柱高 1.0");
    assert_eq!(snapshot.tracks[2].level, 0.0, "静音 ⇒ 柱高 0.0");

    // ---- 控件树：同一个数必须能被 AI 读到（`accessible-label` 是同一条链的出口） ----
    let mut plane = live.into_control_plane(Permission::ReadOnly);
    let (tree, tree_json) = plane.plane().tree().expect("ui/tree");
    // 注意 RMS 与峰值是**两个不同的口径**：`MeterFrame::new` 让 `rms_smoothed = rms`，
    // 因此 `ids[1]` 的 0.25 ⇒ 20·log10(0.25) = -12.04 ⇒ 显示 "-12.0"（不是 "-6.0"）。
    for (id, expected) in [
        ("track-0-meter", "峰值 0.0 RMS 0.0 dBFS"),
        ("track-1-meter", "峰值 -6.0 RMS -12.0 dBFS"),
        ("track-2-meter", "峰值 -120.0 RMS -120.0 dBFS"),
        ("mixer-master-meter", "峰值 0.0 RMS 0.0 dBFS"),
    ] {
        let label = label_in(&tree, id);
        assert!(
            label.ends_with(expected),
            "`{id}` 的标签 {label:?} 必须以 {expected:?} 结尾（dBFS 必须进控件树）"
        );
    }
    report_line(&format!(
        "[app-mixer] 注入电平: track-0-meter={:?} track-1-meter={:?} track-2-meter={:?}",
        label_in(&tree, "track-0-meter"),
        label_in(&tree, "track-1-meter"),
        label_in(&tree, "track-2-meter")
    ));

    // `ui/node` 走的是同一条入口（找不到 / 标签不含期望文本都会显式报错）。
    let probe = plane
        .plane()
        .probe(&ProbeOptions::new("track-1-meter", "峰值 -6.0 RMS -12.0 dBFS").without_masking())
        .expect("ui/node 必须给同一个数");
    assert_eq!(probe.node.id, "track-1-meter");
    assert!(
        tree_json.contains("-6.0"),
        "线上文本里必须真的有那个 dBFS 值"
    );
}

/// 判据 7（**静音下限与 NaN**）：敌意电平输入在控件树里**不许**出现 `NaN`，只许出现下限。
///
/// 这里故意绕过引擎的生产侧钳位（直接构造 `NaN` 帧），因为消费侧必须自己也是安全的：
/// 引擎的防线挡不住"测试注入 / 上游回归"这两类输入。
#[test]
fn hostile_levels_never_render_as_nan_in_the_control_tree() {
    let project = demo_project();
    let ids = track_ids(&project);
    let mut live = build_live_ui_with(&project, &options(Permission::ReadOnly, 1)).expect("装配");
    let (mut publisher, collector) = meter_channel(16);
    live.adopt_meter_collector(collector);
    publisher.publish(&[
        MeterFrame {
            node: ids[0],
            quantum: 9,
            peak: f32::NAN,
            peak_hold: f32::NAN,
            rms: f32::NAN,
            rms_smoothed: f32::NAN,
        },
        MeterFrame {
            node: ids[1],
            quantum: 9,
            peak: f32::INFINITY,
            peak_hold: f32::INFINITY,
            rms: f32::NEG_INFINITY,
            rms_smoothed: f32::NEG_INFINITY,
        },
    ]);
    let snapshot = live.pump_meters();
    assert!(
        snapshot.peak_labels()[0]
            .parse::<f32>()
            .expect("可解析")
            .is_finite()
    );
    assert_eq!(snapshot.peak_labels()[0], "-120.0", "NaN 幅度 ⇒ 静音下限");

    let mut plane = live.into_control_plane(Permission::ReadOnly);
    let (tree, tree_json) = plane.plane().tree().expect("ui/tree");
    for id in ["track-0-meter", "track-1-meter", "mixer-master-meter"] {
        let label = label_in(&tree, id);
        assert!(
            !label.contains("NaN"),
            "`{id}` 的标签里出现了 NaN: {label:?}"
        );
        assert!(
            !label.contains("inf"),
            "`{id}` 的标签里出现了 inf: {label:?}"
        );
    }
    assert_eq!(
        label_in(&tree, "track-0-meter"),
        "轨道 鼓 电平表 峰值 -120.0 RMS -120.0 dBFS"
    );
    assert!(!tree_json.contains("NaN"), "整棵树的线上文本里都不许有 NaN");
}

/// 判据 8（**管理动作 1/3**）：`ui/switch_main_view` 真的切了主视图，树跟着换。
#[test]
fn admin_switch_main_view_really_switches_the_live_view() {
    let project = demo_project();
    let mut plane = build_live_ui_with(&project, &options(Permission::Administrative, 0))
        .expect("装配")
        .into_control_plane(Permission::Administrative);

    // 起点：默认是 Arrangement（`host::build_main_window` 写 `arrangement_by_default`）。
    let (before, _) = plane.plane().tree().expect("ui/tree");
    assert!(before.find("workspace-arrangement-canvas").is_some());
    assert!(before.find("workspace-session-canvas").is_none());

    // 切到 Session：结果里必须带回**回读值**（不是回显入参）。
    let switched = plane.plane().try_line(
        r#"{"jsonrpc":"2.0","id":21,"method":"ui/switch_main_view","params":{"view":"session"}}"#,
    );
    assert!(
        !switched.is_error(),
        "ui/switch_main_view 必须成功: {switched:?}"
    );
    assert_eq!(switched.status, 200);
    let result = switched.result.expect("有 result");
    assert_eq!(result["accepted"], true);
    assert_eq!(result["view"], "session");
    assert_eq!(result["report"]["operation"], "switch_main_view");
    assert_eq!(
        result["report"]["arrangementView"], false,
        "回执必须报告**回读**到的属性值"
    );
    assert!(
        result["report"]["treeNodes"].as_u64().expect("计数") > 0,
        "回执必须报告重抓之后的节点数"
    );

    // 界面真的换了：两条查询路径都换。
    let (after, _) = plane.plane().tree().expect("ui/tree");
    assert!(
        after.find("workspace-session-canvas").is_some(),
        "切到 session 之后运行时树里必须有 Session 画布"
    );
    assert!(
        after.find("workspace-arrangement-canvas").is_none(),
        "切到 session 之后 Arrangement 画布必须离开运行时树"
    );
    let node = plane
        .plane()
        .probe(&ProbeOptions::new("workspace-session-canvas", "Session").without_masking())
        .expect("ui/node 必须看到同一个切换");
    assert_eq!(node.node.role, "main");

    // 切回 Arrangement：同一棵树上再翻一次。
    let back = plane.plane().try_line(
        r#"{"jsonrpc":"2.0","id":22,"method":"ui/switch_main_view","params":{"view":"arrangement"}}"#,
    );
    assert!(!back.is_error(), "切回 Arrangement 必须成功: {back:?}");
    assert_eq!(
        back.result.expect("有 result")["report"]["arrangementView"],
        true
    );
    let (restored, _) = plane.plane().tree().expect("ui/tree");
    assert!(restored.find("workspace-arrangement-canvas").is_some());
    assert!(restored.find("workspace-session-canvas").is_none());

    // 非法视图名 ⇒ 参数错（`-32602`），而且**没有**副作用（视图仍是 Arrangement）。
    let denied = plane.plane().try_line(
        r#"{"jsonrpc":"2.0","id":21,"method":"ui/switch_main_view","params":{"view":"mixer"}}"#,
    );
    assert!(denied.is_error());
    assert_eq!(
        denied.code,
        Some(-32602),
        "非法视图名是**参数问题**, 不是「能力没接线」(-32005): {denied:?}"
    );
    let (still, _) = plane.plane().tree().expect("ui/tree");
    assert!(still.find("workspace-arrangement-canvas").is_some());
}

/// 判据 9（**管理动作 2/3**）：`ui/force_save` 真的写了盘，写出来的能被容器读回。
#[test]
fn admin_force_save_really_writes_a_readable_container() {
    let project = demo_project();
    let dir = scratch_dir("force-save");
    let path = dir.join("live-admin.yeban");
    let mut opts = options(Permission::Administrative, 0);
    opts.save_path = Some(path.clone());
    let mut plane = build_live_ui_with(&project, &opts)
        .expect("装配")
        .into_control_plane(Permission::Administrative);

    assert!(!path.exists(), "保存之前文件不该存在");
    let first = plane
        .plane()
        .try_line(r#"{"jsonrpc":"2.0","id":51,"method":"ui/force_save"}"#);
    assert!(!first.is_error(), "ui/force_save 必须成功: {first:?}");
    let first = first.result.expect("有 result");
    assert_eq!(first["accepted"], true);
    assert_eq!(first["report"]["saveEpoch"], 1);
    let bytes = first["report"]["bytes"].as_u64().expect("字节数");
    assert!(bytes > 0);
    assert!(path.is_file(), "保存之后文件必须真的在磁盘上");
    assert_eq!(
        std::fs::metadata(&path).expect("元数据").len(),
        bytes,
        "回执里的字节数必须等于磁盘上的字节数"
    );
    let read_back = open_project_file(&path).expect("写出来的必须是可读的 .yeban 容器");
    assert_eq!(read_back, project, "读回来的工程必须与写出的逐字段相同");
    report_line(&format!(
        "[app-mixer] ui/force_save: {} 字节 -> {}（可被 open_project_file 读回）",
        bytes,
        path.display()
    ));

    // 第二次保存 ⇒ 世代 +1；目录里不留临时文件。
    let second = plane
        .plane()
        .try_line(r#"{"jsonrpc":"2.0","id":52,"method":"ui/force_save"}"#);
    assert!(!second.is_error(), "第二次保存: {second:?}");
    assert_eq!(second.result.expect("有 result")["report"]["saveEpoch"], 2);
    let leftovers: Vec<String> = std::fs::read_dir(&dir)
        .expect("列目录")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.contains(".tmp-"))
        .collect();
    assert!(leftovers.is_empty(), "残留临时文件: {leftovers:?}");
}

/// 判据 10（**管理动作 3/3**）：`ui/reload_engine` 真的重建引擎，并把电平接回下限。
///
/// 副作用链（每一步都可观测）：注入 0.0 dBFS 的电平 ⇒ `ui/tree` 读得到 ⇒ 重建引擎 ⇒
/// **新的**电平队列被采纳、旧读数作废 ⇒ `ui/tree` 读回下限。
#[test]
fn admin_reload_engine_rebuilds_and_resets_the_meter_tap() {
    let project = demo_project();
    let view = ViewState::from_project(&project).expect("投影");
    let ids = track_ids(&project);
    let mut opts = options(Permission::Administrative, 1);
    opts.engine_quanta = 4;
    let mut live = build_live_ui_with(&project, &opts).expect("装配");
    let (mut publisher, collector) = meter_channel(64);
    live.adopt_meter_collector(collector);
    publisher.publish(&[MeterFrame::new(ids[0], 3, 1.0, 1.0)]);
    live.pump_meters();
    let mut plane = live.into_control_plane(Permission::Administrative);
    let (before, _) = plane.plane().tree().expect("ui/tree");
    assert_eq!(
        label_in(&before, "track-0-meter"),
        "轨道 鼓 电平表 峰值 0.0 RMS 0.0 dBFS",
        "重建之前 UI 显示的是注入的那一帧"
    );

    let reloaded = plane
        .plane()
        .try_line(r#"{"jsonrpc":"2.0","id":61,"method":"ui/reload_engine"}"#);
    assert!(
        !reloaded.is_error(),
        "ui/reload_engine 必须成功: {reloaded:?}"
    );
    let reloaded = reloaded.result.expect("有 result");
    let report = &reloaded["report"];
    assert_eq!(report["operation"], "reload_engine");
    assert_eq!(report["generation"], 1, "第一次重建 = 第 1 代");
    assert_eq!(report["revision"], 1);
    assert_eq!(
        report["tracks"].as_u64().expect("轨道数"),
        (view.tracks.len() + 1) as u64,
        "快照里的节点数 = 非母线轨 + 母线"
    );
    assert_eq!(report["quanta"], 4);
    assert_eq!(
        report["meterBulkPublishes"], 4,
        "每量子恰好一次批量发布（引擎的结构性契约）"
    );
    assert_eq!(
        report["meterFrames"].as_u64().expect("帧数"),
        4 * (view.tracks.len() as u64 + 1),
        "帧数 = 量子数 × (非母线轨数 + 1)"
    );
    report_line(&format!(
        "[app-mixer] ui/reload_engine: gen={} rev={} tracks={} quanta={} publishes={} frames={} visibleQuantum={}",
        report["generation"],
        report["revision"],
        report["tracks"],
        report["quanta"],
        report["meterBulkPublishes"],
        report["meterFrames"],
        report["visibleQuantum"]
    ));

    // ---- 副作用：电平回到下限（新引擎的读数取代了注入的那一帧） ----
    let (after, _) = plane.plane().tree().expect("ui/tree");
    assert_eq!(
        label_in(&after, "track-0-meter"),
        "轨道 鼓 电平表 峰值 -120.0 RMS -120.0 dBFS",
        "引擎换代之后旧读数必须作废（电平回到下限）"
    );
    assert_eq!(
        label_in(&after, "mixer-master-meter"),
        "主控电平表 峰值 -120.0 RMS -120.0 dBFS"
    );

    // 再重建一次 ⇒ 代数递增（不是把同一个数字报两遍）。
    let again = plane
        .plane()
        .try_line(r#"{"jsonrpc":"2.0","id":62,"method":"ui/reload_engine"}"#);
    assert!(!again.is_error(), "第二次重建: {again:?}");
    let again = again.result.expect("有 result");
    assert_eq!(again["report"]["generation"], 2);
    assert_eq!(again["report"]["revision"], 2);
}

/// 判据 11（**管理动作的失败路径**）：引擎重建失败必须**如实报错**，不许假成功。
#[test]
fn admin_reload_engine_reports_a_real_failure_honestly() {
    // `YebanProjectV1::default()` 的 `master_bus_track_id` 是 nil ⇒ 引擎侧 `NoMasterBus`。
    let empty = YebanProjectV1::default();
    let mut plane = build_live_ui_with(&empty, &options(Permission::Administrative, 0))
        .expect("空工程也能装配界面")
        .into_control_plane(Permission::Administrative);

    let denied = plane
        .plane()
        .try_line(r#"{"jsonrpc":"2.0","id":31,"method":"ui/reload_engine"}"#);
    assert!(denied.is_error(), "没有主总线的工程不可能重建出引擎");
    let message = denied.message.clone().unwrap_or_default();
    assert!(
        message.contains("引擎") || message.contains("主总线") || message.contains("master"),
        "失败消息必须说清是引擎/主总线的问题: {message:?}"
    );
    // 再调一次仍然失败（没有"第一次失败之后变成成功"这种状态残留）。
    let again = plane
        .plane()
        .try_line(r#"{"jsonrpc":"2.0","id":32,"method":"ui/reload_engine"}"#);
    assert!(again.is_error());
    // 只读能力不受影响。
    assert!(plane.plane().tree().is_ok());
}

/// 判据 12（**生产模式**）：三个管理动作在**生产模式**下按 scope 被拒，且没有副作用。
#[test]
fn production_mode_rejects_the_three_admin_actions_by_scope() {
    let project = demo_project();
    let dir = scratch_dir("production");
    let path = dir.join("must-not-exist.yeban");
    let mut opts = options(Permission::ReadOnly, 1);
    opts.save_path = Some(path.clone());
    let mut plane = build_live_ui_with(&project, &opts)
        .expect("装配")
        .into_control_plane(Permission::ReadOnly);
    let (before, _) = plane.plane().tree().expect("ui/tree");

    for (method, line, required) in [
        (
            "ui/switch_main_view",
            r#"{"jsonrpc":"2.0","id":41,"method":"ui/switch_main_view","params":{"view":"session"}}"#,
            "app:admin",
        ),
        (
            "ui/force_save",
            r#"{"jsonrpc":"2.0","id":42,"method":"ui/force_save"}"#,
            "app:save",
        ),
        (
            "ui/reload_engine",
            r#"{"jsonrpc":"2.0","id":43,"method":"ui/reload_engine"}"#,
            "app:reload-engine",
        ),
    ] {
        let denied = plane.plane().try_line(line);
        assert!(
            denied.is_error(),
            "生产模式下 `{method}` 必须被拒: {denied:?}"
        );
        assert_eq!(denied.status, 403, "`{method}`: {denied:?}");
        assert_eq!(denied.code, Some(-32003), "`{method}`: {denied:?}");
        report_line(&format!(
            "[app-mixer] 生产模式拒绝 `{method}`: 需要 {required}, 当前 ui:read,ui:screenshot"
        ));
    }

    // 副作用一个都没有：文件没被写、视图没被切、树还是那棵树。
    assert!(!path.exists(), "生产模式下 `ui/force_save` 不得写任何文件");
    let (after, _) = plane.plane().tree().expect("ui/tree");
    assert_eq!(after.count, before.count, "拒绝前后运行时树规模不变");
    assert!(after.find("workspace-arrangement-canvas").is_some());
    assert!(after.find("workspace-session-canvas").is_none());
}
