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
//! | `a_tree_read_sees_a_click_that_just_happened` | **新鲜度**：一次真实点击（`toggle-sidebar`）之后，`ui/tree` / `ui/node` / `ui/property` 三条读同时看见它（节点 **85→81**、`x` **210→6**、宽度 `"240.00"→"36.00"`），再点一次全部回去 |
//! | `a_tree_regrab_costs_one_introspection_not_one_full_render` | **代价**：一次重抓 = 一次全树内省（85 节点），不是一次 1920×1080 的 Tier-1 全量渲染（两者并排打出来） |
//! | `escape_closes_the_time_machine_modal_for_real` | **`Escape` 真的关得掉全屏时光机**（`[UI-A11Y-003]` §7.3 与 `.slint` 的 "Esc 关闭" 那句）：端口读数 `true→false`、运行时树里 `undo-tree-modal` 消失、最后一条动作记录是 `close-undo-tree`，而工程与撤销栈一位未动 |
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

// 判据 17（`ROAD-M4-008` 选项 (a)）：以进程内控制面会话为**唯一可变权威**的装配入口
// 与刷新点。它们只存在于非默认 feature `in-process-mcp` 下 —— 默认构建里没有
// `yeban-mcp`（`tree -p yeban-app -e normal` 命中 0），因此这里也必须 cfg。
#[cfg(feature = "in-process-mcp")]
use live::{AuthoritySync, build_live_ui_from_authority};

// 判据 17 的**同案差分像素**（`ADR-0005` S1）与横向缩放的**像素 A/B**：
// 帧差分读数在默认构建里也被用到（`ui_zoom_shortcuts_...`），因此 `Rect` / `Rgb8Image`
// 与 [`frame_diff`] **不再** cfg —— 只被 non-default feature 用到的那几个助手
// （`diff_pixels_outside` / `projected_lane_stack_box` / 容差常量）继续 cfg，否则默认构建
// 里它们是 dead_code（`-D warnings`）。
use yeban_ui_test_port::{Rect, Rgb8Image};

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
use yeban_app::host::{self, MeterPump};
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
        // 这条装配不接撤销端口（与既有判据 16 的"没有撤销会话"形态一致）。
        undo: None,
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

/// 取某个语义 ID 在**执行面**控件树（`ControlTree`）里的 `accessible-value` 原文。
///
/// `None` = 该元素**没有声明** `accessible-value`（`[ARCH-UI-004]` 的可选状态属性；
/// 不是空串）。判据既要读值，也要读"没有值"。
fn value_of(tree: &ControlTree, id: &str) -> Option<String> {
    tree.find_by_id(id)
        .unwrap_or_else(|| panic!("运行时控件树里没有 `{id}`"))
        .value
        .clone()
}

/// 取某个语义 ID 在**执行面**控件树（`ControlTree`）里的 `accessible-checked`。
fn checked_of(tree: &ControlTree, id: &str) -> Option<bool> {
    tree.find_by_id(id)
        .unwrap_or_else(|| panic!("运行时控件树里没有 `{id}`"))
        .checked
}

/// 取某个语义 ID 在**控制面**投影（`ui/tree` 的 `UiTree`）里的 `accessible-value`。
fn value_in(tree: &UiTree, id: &str) -> Option<String> {
    tree.find(id)
        .unwrap_or_else(|| panic!("`ui/tree` 的投影里没有 `{id}`"))
        .value
        .clone()
}

/// 取某个语义 ID 在**控制面**投影（`ui/tree` 的 `UiTree`）里的 `accessible-checked`。
fn checked_in(tree: &UiTree, id: &str) -> Option<bool> {
    tree.find(id)
        .unwrap_or_else(|| panic!("`ui/tree` 的投影里没有 `{id}`"))
        .checked
}

/// 从推子的 `accessible-value`（`"-3.2 dB"`）里取出 dB 数值。
///
/// 用于把**读回来的文本**与 `TrackV3::volume_db`（模型的权威数字）对账 ——
/// 而不是与"测试自己写的期望文本"对账。`{:.1}` 的显示精度 ⇒ 容差 0.05 dB。
fn db_of_value(value: &str) -> Option<f32> {
    value.strip_suffix(" dB")?.trim().parse::<f32>().ok()
}

/// 从电平标签里取出**峰值 dBFS**（`"… 峰值 -6.0 RMS -23.4 dBFS"` ⇒ `-6.0`）。
///
/// 为什么不用字符串相等去断言电平: 电平是**新引擎真实合成**的产物, 把它写成常量会让这条判据
/// 在合成器任何一次合理调参后变红 —— 那时红的是"数字变了", 而不是"接线坏了"。
/// 这条判据要守的是**接线**: 注入值被丢弃 + 读数是新队列的真实结果。
fn dbfs_value(label: &str) -> Option<f32> {
    let peak = label.split("峰值 ").nth(1)?;
    peak.split_whitespace().next()?.parse::<f32>().ok()
}

/// 一帧像素的 FNV-1a 指纹（与 `ui/screenshot` 的指纹同算法）。
fn frame_fingerprint(image: &yeban_ui_test_port::Rgb8Image) -> String {
    let (_bytes, evidence) =
        encode_with_evidence(image, DEFAULT_MAX_PNG_BYTES).expect("Tier-1 帧必须非零且非全黑");
    format!("{:016x}", evidence.fingerprint)
}

// ---------------------------------------------------------------------------
// 判据 17 的**同案差分像素**（`ADR-0005` S1）：同一次运行的两帧对比，**不碰 golden**
// ---------------------------------------------------------------------------

/// 差分包围盒允许的**命名容差**（逻辑像素）。
///
/// 它吸收两处不可避免的取整：① 画布 → 窗口的 y 平移由投影 `band_y` 的取整标定；
/// ② 字形边缘的抗锯齿会落到相邻像素。**它不允许大到"什么都容得下"** ——
/// 上界由 [`LANE_DIFF_TOLERANCE_MAX_PX`] 与判据 `the_lane_diff_tolerance_stays_bounded`
/// 钉住（把这里放大到全屏必须让那条判据变红）。
#[cfg(feature = "in-process-mcp")]
const LANE_DIFF_TOLERANCE_PX: i32 = 2;

/// 容差的**上界**：判据钉住它，防止"把容差放大到全屏"把断言变成空转。
#[cfg(feature = "in-process-mcp")]
const LANE_DIFF_TOLERANCE_MAX_PX: i32 = 4;

/// 两帧的**差异像素**读数（逐像素比较 RGB 三字节；相等即逐字节相同）。
struct FrameDiff {
    /// 差异像素数。
    count: u64,
    /// 差异像素的整数包围盒（含边界；单位 = 像素）。
    bbox: Rect,
}

/// 逐像素比较两帧；**逐字节相同** ⇒ `None`（"界面一位没动"）。
fn frame_diff(before: &Rgb8Image, after: &Rgb8Image) -> Option<FrameDiff> {
    assert_eq!(
        (before.width(), before.height()),
        (after.width(), after.height()),
        "两帧必须同尺寸（视口由装配时的 `DemoScene` 定下，重投影不改它）"
    );
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (u32::MAX, u32::MAX, 0_u32, 0_u32);
    let mut count = 0_u64;
    for y in 0..before.height() {
        for x in 0..before.width() {
            if before.pixel(x, y) != after.pixel(x, y) {
                count += 1;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }
    (count > 0).then(|| FrameDiff {
        count,
        // 边界含两端 ⇒ 宽高 +1（`Rect::right()`/`bottom()` 是**不含**的右/下界）。
        bbox: Rect::new(
            min_x as i32,
            min_y as i32,
            max_x - min_x + 1,
            max_y - min_y + 1,
        ),
    })
}

/// 落在 `area` **之外**的差异像素数。`0` = "期望区域外逐字节相同"。
#[cfg(feature = "in-process-mcp")]
fn diff_pixels_outside(before: &Rgb8Image, after: &Rgb8Image, area: Rect) -> u64 {
    let mut outside = 0_u64;
    for y in 0..before.height() {
        for x in 0..before.width() {
            let inside = (x as i32) >= area.x
                && (x as i32) < area.right()
                && (y as i32) >= area.y
                && (y as i32) < area.bottom();
            if !inside && before.pixel(x, y) != after.pixel(x, y) {
                outside += 1;
            }
        }
    }
    outside
}

/// 被改轨道上**泳道带栈**在窗口像素里的期望区域。
///
/// **几何来源是投影**：`ViewState::automation_lanes` 的 `band_y` / `band_height`
/// （`ADR-0001` `D28` 的整数口径，`automation.rs:750-753` 的等分）。画布 → 窗口的 y
/// 平移由 `anchor` 标定：起点那一条泳道在 `ui/tree` 里的真实几何（`anchor.y`）减去它的
/// 投影 `band_y` —— 这不是猜，`D28` 的**唯一**注入点把同一份 `band_y` 写进这**一个**
/// 窗口，因此平移是一个常量。x 范围直接取该泳道的窗口几何（所有泳道共用同一条 x 带）。
///
/// 为什么是"带**栈**"而不是"新泳道那一条带"：见判据 17 的步 5b（加一条泳道会重新等分
/// 整条带栈，**既有**泳道的带高也会变）。
#[cfg(feature = "in-process-mcp")]
fn projected_lane_stack_box(
    views: [&ViewState; 2],
    track_index: usize,
    anchor_band_y: f32,
    anchor_bounds: Rect,
    tolerance: i32,
) -> Rect {
    let origin_y = anchor_bounds.y - anchor_band_y.round() as i32;
    let mut bands: Vec<(i32, i32)> = Vec::new();
    for view in views {
        for lane in view
            .automation_lanes
            .iter()
            .filter(|lane| lane.track_index == track_index)
        {
            let y0 = origin_y + lane.band_y.round() as i32;
            bands.push((y0, y0 + lane.band_height.round() as i32));
        }
    }
    let top = bands
        .iter()
        .map(|(y0, _)| *y0)
        .min()
        .expect("该轨道至少一条泳道");
    let bottom = bands
        .iter()
        .map(|(_, y1)| *y1)
        .max()
        .expect("该轨道至少一条泳道");
    Rect::new(
        anchor_bounds.x - tolerance,
        top - tolerance,
        anchor_bounds.width + 2 * tolerance as u32,
        (bottom - top) as u32 + 2 * tolerance as u32,
    )
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

    // ---- [ARCH-UI-004] "自定义绑定状态（如推子电平）"：值 / 勾选态真的读得回来 ----
    //
    // 读的是活控件上的 `accessible-value` / `accessible-checked`（不是标签文本，
    // 也不是测试自己写进树里的常量）。期望值从**投影**（`ViewState`）派生，
    // 而"它跟着活控件走"由下面的换工程那一段证明（换工程 ⇒ 读回来的值必须变）。
    let demo_fader_value =
        value_of(&before, "track-0-fader").expect("推子必须声明 accessible-value");
    let demo_fader_db = db_of_value(&demo_fader_value)
        .unwrap_or_else(|| panic!("推子值必须形如 `X dB`，实测 {demo_fader_value:?}"));
    assert!(
        (demo_fader_db - demo_view.tracks[0].volume_db).abs() <= 0.05,
        "读回来的推子值 {demo_fader_db} dB 必须等于投影的 volume_db {} dB（`{{:.1}}` 的显示精度 ⇒ 容差 0.05）",
        demo_view.tracks[0].volume_db
    );
    assert_eq!(
        demo_fader_value,
        format!("{} dB", demo_view.track_volumes()[0]),
        "accessible-value 的文本必须就是投影的 `volume_display + \" dB\"`"
    );
    assert!(
        demo_view.track_solos()[0] && demo_view.track_mutes()[5],
        "演示工程必须 0 号轨独奏、5 号轨静音，否则下面两条断言没有分辨力"
    );
    assert_eq!(
        checked_of(&before, "track-0-mixer-solo-button"),
        Some(demo_view.track_solos()[0]),
        "独奏按钮的 checked 必须来自工程"
    );
    assert_eq!(
        checked_of(&before, "track-5-mixer-mute-button"),
        Some(demo_view.track_mutes()[5]),
        "静音按钮的 checked 必须来自工程"
    );
    // 没有声明可选状态的元素：如实 `None`（不是空串 / `false`）。
    assert_eq!(
        value_of(&before, "track-0-mixer-color-swatch"),
        None,
        "色标没有声明 `accessible-value` ⇒ None，不许用空串冒充"
    );
    assert_eq!(
        checked_of(&before, "track-0-fader"),
        None,
        "推子没有声明 `accessible-checked` ⇒ None，不许编造 false"
    );
    report_line(&format!(
        "[app-mixer] [ARCH-UI-004] 演示工程: track-0-fader value={demo_fader_value:?} \
         track-0-mixer-solo-button checked={:?} track-5-mixer-mute-button checked={:?}",
        checked_of(&before, "track-0-mixer-solo-button"),
        checked_of(&before, "track-5-mixer-mute-button")
    ));

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

    // ---- [ARCH-UI-004] 换工程 ⇒ 读回来的值 / 勾选态必须**跟着变**（常量抄本在这里变红） ----
    let filled_fader_value =
        value_of(&after, "track-0-fader").expect("推子必须声明 accessible-value");
    assert_eq!(
        filled_fader_value,
        format!("{} dB", filled_view.track_volumes()[0]),
        "换工程后推子值必须来自**新**投影"
    );
    assert!(
        (db_of_value(&filled_fader_value).expect("形如 X dB") - filled_view.tracks[0].volume_db)
            .abs()
            <= 0.05,
        "换工程后读回来的推子值必须等于新投影的 volume_db {}",
        filled_view.tracks[0].volume_db
    );
    assert_ne!(
        demo_fader_value, filled_fader_value,
        "换工程必须换推子值（演示 {} vs 样本 {}）—— 否则读的不是活控件",
        demo_view.tracks[0].volume_db, filled_view.tracks[0].volume_db
    );
    assert!(
        !filled_view.track_solos()[0],
        "样本工程 0 号轨必须不独奏，否则下面那条断言没有分辨力"
    );
    assert_eq!(
        checked_of(&after, "track-0-mixer-solo-button"),
        Some(filled_view.track_solos()[0]),
        "换工程后独奏勾选态必须来自新投影"
    );
    assert_ne!(
        checked_of(&before, "track-0-mixer-solo-button"),
        checked_of(&after, "track-0-mixer-solo-button"),
        "同一个活窗口换工程 ⇒ 独奏勾选态必须真的翻（演示 true / 样本 false）"
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

    // ---- `[ARCH-UI-004]` 端到端：`ui/property` 的两个状态属性走**活控件**，与 `ui/node` 一致 ----
    assert_eq!(
        value_in(&tree, "track-0-fader").as_deref(),
        Some(filled_fader_value.as_str()),
        "`ui/tree` 的 `value` 必须与执行面树上读到的是同一个值"
    );
    assert_eq!(
        checked_in(&tree, "track-0-mixer-solo-button"),
        Some(false),
        "`ui/tree` 的 `checked` 必须与执行面树一致"
    );
    let value_line = plane.plane().try_line(
        r#"{"jsonrpc":"2.0","id":71,"method":"ui/property","params":{"elementId":"track-0-fader","name":"value"}}"#,
    );
    assert!(
        !value_line.is_error(),
        "`ui/property` value: {value_line:?}"
    );
    assert_eq!(
        value_line.result.expect("有 result")["value"],
        filled_fader_value.as_str(),
        "`ui/property {{name:\"value\"}}` 必须从**活控件**读到当前值"
    );
    let checked_line = plane.plane().try_line(
        r#"{"jsonrpc":"2.0","id":72,"method":"ui/property","params":{"elementId":"track-0-mixer-solo-button","name":"checked"}}"#,
    );
    assert!(
        !checked_line.is_error(),
        "`ui/property` checked: {checked_line:?}"
    );
    assert_eq!(
        checked_line.result.expect("有 result")["value"],
        "false",
        "`ui/property {{name:\"checked\"}}` 必须从活控件读到勾选态"
    );
    // 缺席必须**如实报错**（`-32602`，D25 不发明新码）—— 不是空串，也不是"成功但没值"。
    //
    // 为什么不在这里断言那句话：`invalid_params` 把详细理由放在 `error.data.detail`，
    // 而 `message` 是稳定的种类说明（"参数非法"）。本仓库的口径是"判据断言拒的**理由**
    // （机器可读），不去比对一句人话"。因此"元素没声明"与"不支持这个属性名"这两条话术的
    // 区分由**端口层**判据钉住（`yeban-ui-test-port` 的 `read_property` 直接给
    // `PortError::Rejected { message }`），这里只钉线上错误码。
    let absent = plane.plane().try_line(
        r#"{"jsonrpc":"2.0","id":73,"method":"ui/property","params":{"elementId":"track-0-mixer-color-swatch","name":"value"}}"#,
    );
    assert!(
        absent.is_error(),
        "色标没有 `accessible-value` ⇒ 必须报错而不是空串: {absent:?}"
    );
    assert_eq!(absent.code, Some(-32602));
}

/// 判据 7（**混音台的四项写入能力**）：推子 / 声相 / 静音 / 独奏各走**自己的**语义元素
/// 与**自己的**轨道下标，并且各自满足"拖动期只写视图态 ＋ 松手恰一次提交 ＋ `Cmd+Z` 复原"。
///
/// ## 为什么四项用四个不同的轨道下标（序号化 / 多路径探针）
///
/// 上一轮的教训：判据用**固定探针 id** 时，单独破坏某一路径**仍然绿**。
/// 本判据因此给每一项能力分配一组**互不共享**的 `(语义元素, 轨道下标, 模型字段)`：
///
/// | # | 能力 | 语义元素 | 轨道下标 | 模型字段 |
/// | :--- | :--- | :--- | :--- | :--- |
/// | ① | 音量 | `track-0-fader` | 0 | `TrackV3::volume_db` |
/// | ② | 声相 | `track-1-pan` | 1 | `TrackV3::pan` |
/// | ③ | 静音 | `track-2-mixer-mute-button` | 2 | `TrackV3::mute` |
/// | ④ | 独奏 | `track-3-mixer-solo-button` | 3 | `TrackV3::solo` |
///
/// 每一段都额外断言**别的轨道一位没变** ⇒ "写错下标"这一类缺陷会被指名抓到。
///
/// ## 三条会变红的注入（都实测过，见交付报告）
///
/// | 注入 | 位置 | 现象 |
/// | :--- | :--- | :--- |
/// | 删掉推子的 `TouchArea`（"混音台回退成无输入面"） | `ui/console/mixer_console.slint` | ① 段：注入后 `track-volumes[0]` 与注入前逐字相同 ⇒ 红 |
/// | 把提交从 `release` 挪到 `drag`（"拖动不原子"） | `src/host.rs` 的 `on_mixer_fader_drag` | ① 段：`拖动之后 undoable == 0` 与 `工程字段仍是起点值` 两条同时红 |
/// | 删掉 `wire_mixer_edit(&window, undo)` | `src/live_surface.rs` | 四段一起红（没有任何 `.slint` 回调被接到端口上） |
#[test]
fn the_mixer_strip_writes_volume_pan_mute_and_solo_back_into_the_project() {
    use std::rc::Rc;

    use serde_json::json;
    use yeban_app::undo::{UndoPort, UndoSession};
    use yeban_ui_mcp::methods::{METHOD_DISPATCH_POINTER_DOWN, METHOD_DISPATCH_POINTER_MOVE};

    /// 会话打开时刻（与既有两个拖拽判据同一个夹具常量）。
    const MIXER_NOW: u64 = 1_760_000_000_000;

    let project = demo_project();
    let view = ViewState::from_project(&project).expect("演示投影");
    assert!(view.tracks.len() >= 4, "演示工程至少有 4 条非主总线轨道");
    // 四项能力各自的起点值（从**投影**读，与界面上的下标同源）。
    let before: Vec<(f32, f32, bool, bool)> = (0..4)
        .map(|index| mixer_readings(&project, &view, index))
        .collect();
    // 起点必须能分辨"改过"与"没改"：四项的起点都不是它们的终值。
    assert_eq!(before[0].0, -3.2, "0 号轨（鼓）的音量起点是 -3.2 dB");
    assert_eq!(before[1].1, 0.0, "1 号轨（贝斯）的声相起点居中");
    assert!(!before[2].2, "2 号轨（铺底）起点不静音");
    assert!(!before[3].3, "3 号轨（主音）起点不独奏");

    // 混音台（控制台 Tab 1）必须**可见**，否则它的元素不在运行时树里。
    //
    // 端口先于装配建好，并交给 [`LiveWiringOptions::undo`]：装配路径
    // （`build_live_ui_with`）拿它去接**生产形态**的两条线 —— `host::wire_keys`
    // （`Cmd+Z`）与 `host::wire_mixer_edit`（混音台写入面）。
    // 判据**自己不再接线**：于是"装配路径忘了接混音台"会被本判据直接抓到
    // （注入实测见本判据文档的表）。
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<判据>", "yeban-app", project.clone(), MIXER_NOW).expect("打开"),
    ));
    let live = build_live_ui_with(
        &project,
        &LiveWiringOptions {
            permission: Permission::Interactive,
            console_tab: 1,
            save_path: None,
            engine_quanta: 0,
            undo: Some(Rc::clone(&port)),
        },
    )
    .expect("装配");
    let window = slint::ComponentHandle::clone_strong(live.ui());
    // ---- 像素：**未注入任何拖动**的默认外观（`console_tab = 1` ⇒ 混音台可见）----
    //
    // 本切片给混音台加的全是**不画像素**的东西（`TouchArea` 与 `accessible-*`），
    // 因此默认帧必须逐字节不变。读数记为 PNG **字节**的 sha256（不是文件大小：
    // 本仓 PNG 是存储式 deflate，1920×1080 恒为 6,222,418 字节 ⇒ 尺寸证明不了内容）。
    // A/B 实测（同一条 `console_tab=1` 帧，改动前 / 改动后各构建一次）见交付报告。
    let default_frame = live.capture().expect("默认外观帧");
    let (default_png, _default_evidence) =
        encode_with_evidence(&default_frame, DEFAULT_MAX_PNG_BYTES).expect("默认帧必须可编码");
    let default_digest = yeban_model::ids::ContentHash::of_bytes(&default_png);
    let artifact = write_artifact_bytes("mixer-default-1920x1080.png", &default_png);
    report_line(&format!(
        "[mixer-pixel] 默认外观（未注入拖动）: {} 字节 / sha256={} / artifact={artifact}",
        default_png.len(),
        default_digest.as_str()
    ));
    let mut plane = live.into_control_plane(Permission::Interactive);

    assert_eq!(port.display().undoable, 0, "起点不该有可撤销的编辑");

    // ================================================================ ① 音量（推子竖直拖）
    let fader = element_bounds(&mut plane, 301, "track-0-fader");
    let fader_x = fader["x"].as_f64().expect("x") + 11.0;
    let fader_y = fader["y"].as_f64().expect("y") + 8.0;
    let before_text = injected_strings(&window.get_track_volumes())[0].clone();
    assert_eq!(before_text, "-3.2", "① 注入前的推子文本");
    let down = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        302,
        METHOD_DISPATCH_POINTER_DOWN,
        Some(json!({"elementId": "track-0-fader", "xOffset": 11.0, "yOffset": 8.0, "button": "left"})),
    ));
    assert!(!down.is_error(), "① 推子按下必须落到真实窗口: {down:?}");
    // 向下拖 40px：88px 走完 66 dB（`-60..=6`）⇒ 40 ÷ 88 × 66 = 30 dB ⇒ -3.2 − 30 = -33.2。
    let moved = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        303,
        METHOD_DISPATCH_POINTER_MOVE,
        Some(json!({"x": fader_x, "y": fader_y + 40.0})),
    ));
    assert!(!moved.is_error(), "① 推子拖动必须落到真实窗口: {moved:?}");
    let dragging_text = injected_strings(&window.get_track_volumes())[0].clone();
    assert_eq!(
        dragging_text, "-33.2",
        "① 拖动期数值文本必须**立即**跟着动（这是「数值始终可见且随拖动更新」）"
    );
    // ⭐ 拖动期**只**写视图态：工程一位没动，撤销栈也没涨。
    assert_eq!(
        port.display().undoable,
        0,
        "① 拖动期不许提交（每像素一次提交会把一步撤销碎成几百步）"
    );
    assert_eq!(
        mixer_readings(&port.project(), &view, 0).0,
        before[0].0,
        "① 拖动期工程里的 volume_db 必须一位没动（视图态 ≠ 工程）"
    );
    let up = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        304,
        "ui/dispatch_pointer_up",
        Some(json!({"button": "left"})),
    ));
    assert!(!up.is_error(), "① 推子松手必须落到真实窗口: {up:?}");
    let committed_volume = mixer_readings(&port.project(), &view, 0).0;
    assert_eq!(
        format!("{committed_volume:.1}"),
        "-33.2",
        "① 松手后工程里的 volume_db 必须真的是 -33.2 dB（读回来证明）"
    );
    assert_eq!(
        port.display().undoable,
        1,
        "① 一次拖动 = **恰一步**可撤销的编辑（原子）"
    );
    assert_eq!(
        injected_strings(&window.get_track_volumes())[0].clone(),
        "-33.2"
    );
    // 别的轨道一位没变（写错下标会被这里抓到）。
    for (index, expected) in before.iter().enumerate().skip(1) {
        assert_eq!(
            mixer_readings(&port.project(), &view, index),
            *expected,
            "① 第 {index} 轨不该被 0 号轨的手势波及"
        );
    }
    report_line(&format!(
        "[mixer-①volume] 注入前 {before_text} dB → 拖动期 {dragging_text} dB（工程仍 {} dB / undoable 0）\
         → 松手 {committed_volume:.4} dB / undoable {} → `ui/property` 读数 {:?}",
        before[0].0,
        port.display().undoable,
        read_property(&mut plane, 305, "track-0-fader", "value")
    ));

    // `Cmd+Z` 复原（走 `.slint` 的 `key-action` 那**同一格**：与平台事件源同一条链）。
    assert!(
        window.invoke_key_action("z".into(), false, false, false, true),
        "① `Cmd+Z` 必须被 DAW 消费"
    );
    assert_eq!(
        mixer_readings(&port.project(), &view, 0).0.to_bits(),
        before[0].0.to_bits(),
        "① `Cmd+Z` 必须把 volume_db **逐位**还原"
    );
    assert_eq!(port.display().undoable, 0);
    assert_eq!(
        injected_strings(&window.get_track_volumes())[0].clone(),
        before_text,
        "① `Cmd+Z` 之后界面上的数值文本也必须回到注入前"
    );
    report_line(&format!(
        "[mixer-①volume] `Cmd+Z` 之后: volume_db={} dB（起点 {}）/ 界面文本={:?} / undoable={}",
        mixer_readings(&port.project(), &view, 0).0,
        before[0].0,
        injected_strings(&window.get_track_volumes())[0].clone(),
        port.display().undoable
    ));

    // ================================================================ ② 声相（水平拖）
    let pan = element_bounds(&mut plane, 311, "track-1-pan");
    let pan_x = pan["x"].as_f64().expect("x") + 10.0;
    let pan_y = pan["y"].as_f64().expect("y") + 7.0;
    assert_eq!(
        injected_strings(&window.get_track_pans())[1].clone(),
        "C",
        "② 注入前的声相文本"
    );
    let down = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        312,
        METHOD_DISPATCH_POINTER_DOWN,
        Some(
            json!({"elementId": "track-1-pan", "xOffset": 10.0, "yOffset": 7.0, "button": "left"}),
        ),
    ));
    assert!(!down.is_error(), "② 声相按下必须落到真实窗口: {down:?}");
    // 向右拖 9px：声相面 36px 走完 -1.0..=+1.0（跨度 2.0）⇒ 9 ÷ 36 × 2.0 = +0.5 ⇒ `R50`。
    let moved = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        313,
        METHOD_DISPATCH_POINTER_MOVE,
        Some(json!({"x": pan_x + 9.0, "y": pan_y})),
    ));
    assert!(!moved.is_error(), "② 声相拖动必须落到真实窗口: {moved:?}");
    let dragging_pan = injected_strings(&window.get_track_pans())[1].clone();
    assert_eq!(dragging_pan, "R50", "② 拖动期声相文本必须立即更新");
    assert_eq!(
        mixer_readings(&port.project(), &view, 1).1,
        before[1].1,
        "② 拖动期工程里的 pan 必须一位没动"
    );
    assert_eq!(port.display().undoable, 0, "② 拖动期不许提交");
    let up = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        314,
        "ui/dispatch_pointer_up",
        Some(json!({"button": "left"})),
    ));
    assert!(!up.is_error(), "② 声相松手必须落到真实窗口: {up:?}");
    let committed_pan = mixer_readings(&port.project(), &view, 1).1;
    assert!(
        (committed_pan - 0.5).abs() < 1e-6,
        "② 松手后工程里的 pan 必须真的是 +0.5（读回来证明），实测 {committed_pan}"
    );
    assert_eq!(port.display().undoable, 1, "② 一次拖动 = 恰一步");
    assert_eq!(
        read_property(&mut plane, 315, "track-1-pan", "value"),
        json!("R50"),
        "② `ui/property` 的 `value` 必须把声相读回来（本切片之前它读不到）"
    );
    for index in [0_usize, 2, 3] {
        assert_eq!(
            mixer_readings(&port.project(), &view, index),
            before[index],
            "② 第 {index} 轨不该被 1 号轨的手势波及"
        );
    }
    report_line(&format!(
        "[mixer-②pan] 注入前 {:?} → 拖动期 {:?}（工程仍 {} / undoable 0）→ 松手 {committed_pan} / \
         `ui/property value`={:?}",
        "C",
        dragging_pan,
        before[1].1,
        read_property(&mut plane, 316, "track-1-pan", "value")
    ));
    assert!(
        window.invoke_key_action("z".into(), false, false, false, true),
        "② `Cmd+Z` 必须被 DAW 消费"
    );
    assert_eq!(
        mixer_readings(&port.project(), &view, 1).1.to_bits(),
        before[1].1.to_bits(),
        "② `Cmd+Z` 必须把 pan **逐位**还原"
    );
    assert_eq!(injected_strings(&window.get_track_pans())[1].clone(), "C");

    // ================================================================ ③ 静音（点击开关）
    let opened = port.project().tracks.iter().filter(|(_, t)| t.solo).count();
    assert!(!before[2].2, "③ 起点不静音");
    click_element(&mut plane, 321, "track-2-mixer-mute-button");
    assert!(
        mixer_readings(&port.project(), &view, 2).2,
        "③ 点击之后工程里的 mute 必须真的是 true（读回来证明）"
    );
    assert_eq!(
        port.display().undoable,
        1,
        "③ 一次点击 = **恰一步**可撤销的编辑"
    );
    assert_eq!(
        read_property(&mut plane, 325, "track-2-mixer-mute-button", "checked"),
        json!("true"),
        "③ 界面上的勾选态必须跟着工程（`ui/property` 的 `value` 是**文本**，与既有判据同口径）"
    );
    for index in [0_usize, 1, 3] {
        assert_eq!(
            mixer_readings(&port.project(), &view, index),
            before[index],
            "③ 第 {index} 轨不该被 2 号轨的点击波及"
        );
    }
    report_line(&format!(
        "[mixer-③mute] `track-2-mixer-mute-button` 点击: mute {} → {} / undoable {} / \
         `ui/property checked`={:?}（solo 轨数仍 {opened}）",
        before[2].2,
        mixer_readings(&port.project(), &view, 2).2,
        port.display().undoable,
        read_property(&mut plane, 326, "track-2-mixer-mute-button", "checked")
    ));
    assert!(
        window.invoke_key_action("z".into(), false, false, false, true),
        "③ `Cmd+Z` 必须被 DAW 消费"
    );
    assert!(
        !mixer_readings(&port.project(), &view, 2).2,
        "③ `Cmd+Z` 必须把 mute 还原"
    );
    assert_eq!(port.display().undoable, 0);

    // ================================================================ ④ 独奏（点击开关）
    assert!(!before[3].3, "④ 起点不独奏");
    click_element(&mut plane, 331, "track-3-mixer-solo-button");
    assert!(
        mixer_readings(&port.project(), &view, 3).3,
        "④ 点击之后工程里的 solo 必须真的是 true（读回来证明）"
    );
    assert_eq!(port.display().undoable, 1, "④ 一次点击 = 恰一步");
    assert_eq!(
        read_property(&mut plane, 335, "track-3-mixer-solo-button", "checked"),
        json!("true")
    );
    for index in [0_usize, 1, 2] {
        assert_eq!(
            mixer_readings(&port.project(), &view, index),
            before[index],
            "④ 第 {index} 轨不该被 3 号轨的点击波及"
        );
    }
    report_line(&format!(
        "[mixer-④solo] `track-3-mixer-solo-button` 点击: solo {} → {} / undoable {} / \
         `ui/property checked`={:?}",
        before[3].3,
        mixer_readings(&port.project(), &view, 3).3,
        port.display().undoable,
        read_property(&mut plane, 336, "track-3-mixer-solo-button", "checked")
    ));
    assert!(
        window.invoke_key_action("z".into(), false, false, false, true),
        "④ `Cmd+Z` 必须被 DAW 消费"
    );
    assert!(
        !mixer_readings(&port.project(), &view, 3).3,
        "④ `Cmd+Z` 必须把 solo 还原"
    );
    assert_eq!(port.display().undoable, 0);

    // ---- 收尾：四项都复原之后，工程必须逐位回到起点（四条能力互不残留） ----
    for (index, expected) in before.iter().enumerate() {
        assert_eq!(
            mixer_readings(&port.project(), &view, index),
            *expected,
            "第 {index} 轨在四条能力都 `Cmd+Z` 之后必须逐位回到起点"
        );
    }
    // 撤销日志证明"四条能力各自走到了端口"（不是只改了界面属性）。
    let names: Vec<String> = port
        .records()
        .iter()
        .map(|record| record.action.to_owned())
        .collect();
    report_line(&format!(
        "[mixer] 四项能力的撤销日志（共 {} 条 undo）: {names:?}；最后 undoable={}",
        names.len(),
        port.display().undoable
    ));
}

/// 判据 7b（**主控通道条的写入能力**，本切片补的那一格）：主控**音量**与**声相**各自满足
/// "拖动期只写视图态 ＋ 松手恰一次提交 ＋ `Cmd+Z` 逐位回退 ＋ `Escape` 写回起点不提交 ＋
/// 孤立 `move`／迟到松手不再改值"。
///
/// ## 为什么主控要用**另一组无下标的回调**（本判据同时钉住这个方案）
///
/// 主总线**不在** `track-names` / `track-ids` 里（投影刻意排除它，它单独走
/// `ViewState::master: Option<TrackView>`）⇒ 通道条那八个回调的 `int` 参数里**没有**任何
/// 合法值能指到主控。本切片给主控一组**不带下标**的回调
/// （`mixer-master-fader-grab(length)` 等），身份由宿主在按下那一刻从权威工程取
/// （`host::master_track_id` ← `UndoPort::try_project().master_bus_track_id`）。
/// 于是 `.slint` 里没有身份→下标的算术，也没有哨兵下标。
///
/// ## 五行字面读数（每一项能力各走一遍，元素与模型字段互不共享）
///
/// | # | 情形 | 主控音量（`mixer-master-fader`） | 主控声相（`mixer-master-pan`） |
/// | :--- | :--- | :--- | :--- |
/// | ① | 按下 ＋ 拖动 | 文本 `0.0 → -33.0`、工程逐位未动、`undoable` 0、标志 `true` | 文本 `C → R50`、工程逐位未动、`undoable` 0、标志 `true` |
/// | ② | 松手 | 工程 `volume_db = -33.0`、`undoable` **恰 1** | 工程 `pan = +0.5`、`undoable` **恰 1** |
/// | ③ | `Cmd+Z` | `volume_db` **逐位**回退、`undoable` 0 | `pan` **逐位**回退、`undoable` 0 |
/// | ④ | `Escape` | 文本写回 `0.0`、工程逐位未动、`undoable` 0、标志 `false` | 文本写回 `C`、工程逐位未动、`undoable` 0、标志 `false` |
/// | ⑤ | 孤立 `move` ＋ 迟到松手 | 文本仍是起点、`undoable` 仍 0 | 文本仍是起点、`undoable` 仍 0 |
///
/// ④ 的两条路径**故意不同**：音量走**端口**（`ui/dispatch_key_press`，真事件源），
/// 声相走 `MainWindow::invoke_key_action`（`.slint` 的 `key-action` 那一格）——
/// 两条都坏才会绿。
///
/// ## 怎么变红（注入的字面红行见交付报告）
///
/// | 注入 | 位置 | 现象 |
/// | :--- | :--- | :--- |
/// | 摘掉主控推子的 `TouchArea`（"主控回退成不可操作"） | `ui/console/mixer_console.slint` | ① 步：注入后 `master-volume-display` 与注入前逐字相同 ⇒ 红 |
/// | 让主控的按下不接线（摘掉 `ui.on_mixer_master_fader_grab`） | `src/host.rs` 的 `wire_mixer_edit` | ① 步：标志仍 `false`、文本不动 ⇒ 红 |
/// | 把主控的提交从 `release` 挪到 `drag`（"主控拖动不原子"） | `src/host.rs` 的 `on_mixer_master_fader_drag` | ① 步：`undoable` 不再是 0 ⇒ 红 |
/// | 让主控 `Escape` 走提交 | `src/host.rs` 的 `cancel_mixer_gesture` | ④ 步：`undoable` 涨到 1、工程字段被改 ⇒ 红 |
#[test]
fn the_master_strip_writes_volume_and_pan_back_into_the_project() {
    use std::rc::Rc;

    use serde_json::json;
    use yeban_app::undo::{UndoPort, UndoSession};
    use yeban_ui_mcp::methods::{
        METHOD_DISPATCH_KEY_PRESS, METHOD_DISPATCH_POINTER_DOWN, METHOD_DISPATCH_POINTER_MOVE,
        METHOD_DISPATCH_POINTER_UP,
    };

    /// 会话打开时刻（与既有混音台判据同一个夹具常量）。
    const NOW: u64 = 1_760_000_000_000;

    let project = demo_project();
    let view = ViewState::from_project(&project).expect("演示投影");
    // 起点读数从**模型**读（不是从界面读）：这样"改了没有"不依赖被测的写入面。
    let before = master_readings(&project);
    assert_eq!(
        before,
        (0.0_f32, 0.0_f32),
        "演示工程的主总线起点是 0.0 dB / 居中"
    );
    assert!(view.master.is_some(), "演示工程必须有主总线投影");
    // 通道条的起点读数（收尾对账用：主控手势**不许**波及任何一条通道条）。
    let tracks_before: Vec<(f32, f32, bool, bool)> = (0..view.tracks.len())
        .map(|index| mixer_readings(&project, &view, index))
        .collect();

    let port = Rc::new(UndoPort::new(
        UndoSession::open("<判据>", "yeban-app", project.clone(), NOW).expect("打开"),
    ));
    let live = build_live_ui_with(
        &project,
        &LiveWiringOptions {
            permission: Permission::Interactive,
            console_tab: 1,
            save_path: None,
            engine_quanta: 0,
            undo: Some(Rc::clone(&port)),
        },
    )
    .expect("装配");
    let window = slint::ComponentHandle::clone_strong(live.ui());
    let mut plane = live.into_control_plane(Permission::Interactive);

    assert_eq!(port.display().undoable, 0, "起点不该有可撤销的编辑");
    let volume_before_text = window.get_master_volume_display().to_string();
    let pan_before_text = window.get_master_pan().to_string();
    assert_eq!(volume_before_text, "0.0", "① 注入前的主控音量文本");
    assert_eq!(pan_before_text, "C", "④ 注入前的主控声相文本");
    // 主控声相**进控件树**（`ui/property` 的 `value` = `accessible-value` 原文）。
    assert_eq!(
        read_property(&mut plane, 299, "mixer-master-pan", "value"),
        json!("C"),
        "主控声相必须能从控件树读回来（本切片新加的 `accessible-value`）"
    );

    // ================================================================ ① 音量：拖动期
    let fader = element_bounds(&mut plane, 301, "mixer-master-fader");
    let fader_x = fader["x"].as_f64().expect("x") + 11.0;
    let fader_y = fader["y"].as_f64().expect("y") + 8.0;
    let down = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        302,
        METHOD_DISPATCH_POINTER_DOWN,
        Some(json!({
            "elementId": "mixer-master-fader",
            "xOffset": 11.0,
            "yOffset": 8.0,
            "button": "left"
        })),
    ));
    assert!(!down.is_error(), "① 主控推子按下必须落到真实窗口: {down:?}");
    assert!(
        window.get_mixer_drag_active(),
        "① 按下之后手势标志必须是 true（主控与通道条共用同一份手势读数）"
    );
    // 向下拖 44px：88px 走完 66 dB ⇒ 0.0 − 33.0 = −33.0 dB。
    let moved = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        303,
        METHOD_DISPATCH_POINTER_MOVE,
        Some(json!({"x": fader_x, "y": fader_y + 44.0})),
    ));
    assert!(
        !moved.is_error(),
        "① 主控推子拖动必须落到真实窗口: {moved:?}"
    );
    let volume_dragging_text = window.get_master_volume_display().to_string();
    assert_eq!(
        volume_dragging_text, "-33.0",
        "① 拖动期主控数值文本必须**立即**跟着动"
    );
    assert_eq!(
        port.display().undoable,
        0,
        "① 拖动期不许提交（一次拖动 = 一次提交）"
    );
    assert_eq!(
        master_readings(&port.project()).0.to_bits(),
        before.0.to_bits(),
        "① 拖动期工程里的主总线 volume_db 必须**逐位**没动（视图态 ≠ 工程）"
    );
    assert!(window.get_mixer_drag_active(), "① 拖动期手势标志仍是 true");
    assert_eq!(
        read_property(&mut plane, 305, "mixer-master-fader", "value"),
        json!("-33.0 dB"),
        "① 拖动期 `ui/property` 读到的推子值也必须跟着动"
    );

    // ================================================================ ② 音量：松手提交
    let up = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        306,
        METHOD_DISPATCH_POINTER_UP,
        Some(json!({"button": "left"})),
    ));
    assert!(!up.is_error(), "② 主控推子松手必须落到真实窗口: {up:?}");
    let committed_volume = master_readings(&port.project()).0;
    assert_eq!(
        format!("{committed_volume:.1}"),
        "-33.0",
        "② 松手后工程里的主总线 volume_db 必须真的是 −33.0 dB（读回来证明既有 `Op` 够用）"
    );
    assert_eq!(
        port.display().undoable,
        1,
        "② 一次主控拖动 = **恰一步**可撤销的编辑（原子）"
    );
    assert!(!window.get_mixer_drag_active(), "② 松手之后标志归位");

    // ================================================================ ③ 音量：`Cmd+Z`
    assert!(
        window.invoke_key_action("z".into(), false, false, false, true),
        "③ `Cmd+Z` 必须被 DAW 消费"
    );
    assert_eq!(
        master_readings(&port.project()).0.to_bits(),
        before.0.to_bits(),
        "③ `Cmd+Z` 必须把主总线 volume_db **逐位**还原"
    );
    assert_eq!(port.display().undoable, 0);
    assert_eq!(
        window.get_master_volume_display().to_string(),
        volume_before_text,
        "③ `Cmd+Z` 之后界面上的主控数值文本也必须回到注入前"
    );

    // ================================================================ ④ 音量：`Escape`（端口那一格）
    //
    // ⚠ **每一次按下之前都要重新读一次几何**：`ui/dispatch_pointer_down` 用的是**端口里
    // 上一次抓的**控件树几何，而推子帽的 y 随数值走（`60px + 88px × (1 − fraction)`）——
    // 上一段手势把值改成 −33.0 之后，端口里那份几何的推子帽已经下移 44px。本判据第一版
    // 就是踩在这里：第二次按下落在**旧**帽位上，`mouse-y` 与拖动目标只差 0 ⇒ 观察不到任何
    // 移动（这不是产品缺陷，是注入面的测量口径）。重读 `ui/node` 把几何刷新到当前状态。
    let fader = element_bounds(&mut plane, 310, "mixer-master-fader");
    let fader_x = fader["x"].as_f64().expect("x") + 11.0;
    let fader_y = fader["y"].as_f64().expect("y") + 8.0;
    let down = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        311,
        METHOD_DISPATCH_POINTER_DOWN,
        Some(json!({
            "elementId": "mixer-master-fader",
            "xOffset": 11.0,
            "yOffset": 8.0,
            "button": "left"
        })),
    ));
    assert!(!down.is_error(), "④ 主控推子按下必须落到真实窗口: {down:?}");
    let moved = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        312,
        METHOD_DISPATCH_POINTER_MOVE,
        Some(json!({"x": fader_x, "y": fader_y + 44.0})),
    ));
    assert!(
        !moved.is_error(),
        "④ 主控推子拖动必须落到真实窗口: {moved:?}"
    );
    assert_eq!(window.get_master_volume_display().to_string(), "-33.0");
    let pressed = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        313,
        METHOD_DISPATCH_KEY_PRESS,
        Some(json!({"keyCode": "Escape"})),
    ));
    assert!(!pressed.is_error(), "④ `Escape` 注入必须成功: {pressed:?}");
    assert!(
        !window.get_mixer_drag_active(),
        "④ `Escape` 必须让主控手势标志归位（与通道条同一个 `mixer-drag-active`）"
    );
    assert_eq!(
        port.display().undoable,
        0,
        "④ 取消**不是**提交：撤销栈一位不许涨"
    );
    assert_eq!(
        master_readings(&port.project()).0.to_bits(),
        before.0.to_bits(),
        "④ 取消之后工程里的主控 volume_db 必须**逐位**等于起点"
    );
    let volume_after_escape_text = window.get_master_volume_display().to_string();
    assert_eq!(
        volume_after_escape_text, volume_before_text,
        "④ `Escape` 必须把主控数值文本写回**起点**"
    );

    // ================================================================ ⑤ 音量：孤立 move ＋ 迟到松手
    let stray = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        314,
        METHOD_DISPATCH_POINTER_MOVE,
        Some(json!({"x": fader_x, "y": fader_y + 70.0})),
    ));
    assert!(!stray.is_error(), "⑤ 孤立 move 必须落到真实窗口: {stray:?}");
    assert_eq!(
        window.get_master_volume_display().to_string(),
        volume_before_text,
        "⑤ 手势已经结束 ⇒ 孤立的 `move` 不许再改主控音量"
    );
    let late_up = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        315,
        METHOD_DISPATCH_POINTER_UP,
        Some(json!({"button": "left"})),
    ));
    assert!(
        !late_up.is_error(),
        "⑤ 迟到松手必须落到真实窗口: {late_up:?}"
    );
    assert_eq!(port.display().undoable, 0, "⑤ 迟到松手不许提交");
    assert_eq!(
        master_readings(&port.project()).0.to_bits(),
        before.0.to_bits(),
        "⑤ 迟到松手之后主控 volume_db 仍必须逐位等于起点"
    );
    report_line(&format!(
        "[mixer-master-①volume] `mixer-master-fader` 起点 {volume_before_text} dB → 拖动期 \
         {volume_dragging_text} dB（工程逐位仍 {} / undoable 0 / 标志 true）→ 松手 \
         {committed_volume:.4} dB / undoable 1 → `Cmd+Z` 后 {} dB → `Escape` 后文本 \
         {volume_after_escape_text:?}（工程逐位仍 {} / undoable {}）→ 孤立 move ＋ 迟到松手后 \
         undoable {}",
        before.0,
        master_readings(&port.project()).0,
        master_readings(&port.project()).0,
        port.display().undoable,
        port.display().undoable
    ));

    // ================================================================ ⑥ 声相：拖动期
    let pan = element_bounds(&mut plane, 321, "mixer-master-pan");
    let pan_x = pan["x"].as_f64().expect("x") + 10.0;
    let pan_y = pan["y"].as_f64().expect("y") + 7.0;
    let down = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        322,
        METHOD_DISPATCH_POINTER_DOWN,
        Some(json!({
            "elementId": "mixer-master-pan",
            "xOffset": 10.0,
            "yOffset": 7.0,
            "button": "left"
        })),
    ));
    assert!(!down.is_error(), "⑥ 主控声相按下必须落到真实窗口: {down:?}");
    assert!(window.get_mixer_drag_active(), "⑥ 主控声相手势在手");
    // 向右拖 9px：声相面 36px 走完 -1.0..=+1.0（跨度 2.0）⇒ +0.5 ⇒ `R50`。
    let moved = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        323,
        METHOD_DISPATCH_POINTER_MOVE,
        Some(json!({"x": pan_x + 9.0, "y": pan_y})),
    ));
    assert!(
        !moved.is_error(),
        "⑥ 主控声相拖动必须落到真实窗口: {moved:?}"
    );
    let pan_dragging_text = window.get_master_pan().to_string();
    assert_eq!(pan_dragging_text, "R50", "⑥ 拖动期主控声相文本必须立即更新");
    assert_eq!(port.display().undoable, 0, "⑥ 拖动期不许提交");
    assert_eq!(
        master_readings(&port.project()).1.to_bits(),
        before.1.to_bits(),
        "⑥ 拖动期工程里的主总线 pan 必须逐位没动"
    );
    assert_eq!(
        read_property(&mut plane, 324, "mixer-master-pan", "value"),
        json!("R50"),
        "⑥ 拖动期 `ui/property` 的 `value` 必须把主控声相读回来"
    );

    // ================================================================ ⑦ 声相：松手提交
    let up = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        325,
        METHOD_DISPATCH_POINTER_UP,
        Some(json!({"button": "left"})),
    ));
    assert!(!up.is_error(), "⑦ 主控声相松手必须落到真实窗口: {up:?}");
    let committed_pan = master_readings(&port.project()).1;
    assert!(
        (committed_pan - 0.5).abs() < 1e-6,
        "⑦ 松手后工程里的主总线 pan 必须真的是 +0.5（读回来证明），实测 {committed_pan}"
    );
    assert_eq!(port.display().undoable, 1, "⑦ 一次主控声相拖动 = 恰一步");

    // ================================================================ ⑧ 声相：`Cmd+Z`
    assert!(
        window.invoke_key_action("z".into(), false, false, false, true),
        "⑧ `Cmd+Z` 必须被 DAW 消费"
    );
    assert_eq!(
        master_readings(&port.project()).1.to_bits(),
        before.1.to_bits(),
        "⑧ `Cmd+Z` 必须把主总线 pan **逐位**还原"
    );
    assert_eq!(port.display().undoable, 0);
    assert_eq!(window.get_master_pan().to_string(), "C");

    // ================================================================ ⑨ 声相：`Escape`（回调那一格）
    let down = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        331,
        METHOD_DISPATCH_POINTER_DOWN,
        Some(json!({
            "elementId": "mixer-master-pan",
            "xOffset": 10.0,
            "yOffset": 7.0,
            "button": "left"
        })),
    ));
    assert!(!down.is_error(), "⑨ 主控声相按下必须落到真实窗口: {down:?}");
    // （声相文本的位置不随数值走，但按下之前重读几何是同一条纪律，见 ④ 的注。）
    let pan_bounds_again = element_bounds(&mut plane, 330, "mixer-master-pan");
    assert_eq!(
        (
            pan_bounds_again["x"].as_f64(),
            pan_bounds_again["y"].as_f64()
        ),
        (Some(pan_x - 10.0), Some(pan_y - 7.0)),
        "主控声相元素的几何不随数值动（与推子帽不同）"
    );
    let moved = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        332,
        METHOD_DISPATCH_POINTER_MOVE,
        Some(json!({"x": pan_x + 9.0, "y": pan_y})),
    ));
    assert!(
        !moved.is_error(),
        "⑨ 主控声相拖动必须落到真实窗口: {moved:?}"
    );
    assert_eq!(window.get_master_pan().to_string(), "R50");
    assert!(
        window.invoke_key_action("\u{1b}".into(), false, false, false, false),
        "⑨ 主控声相手势在手时 `Escape` 必须被 DAW 消费"
    );
    let pan_after_escape_text = window.get_master_pan().to_string();
    assert_eq!(
        pan_after_escape_text, pan_before_text,
        "⑨ `Escape` 写回起点声相"
    );
    assert!(!window.get_mixer_drag_active(), "⑨ `Escape` 让手势标志归位");
    assert_eq!(port.display().undoable, 0, "⑨ 取消不是提交");
    assert_eq!(
        master_readings(&port.project()).1.to_bits(),
        before.1.to_bits(),
        "⑨ 取消之后工程里的主总线 pan 必须逐位等于起点"
    );

    // ================================================================ ⑩ 声相：孤立 move ＋ 迟到松手
    let stray = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        333,
        METHOD_DISPATCH_POINTER_MOVE,
        Some(json!({"x": pan_x + 30.0, "y": pan_y})),
    ));
    assert!(!stray.is_error(), "⑩ 孤立 move 必须落到真实窗口: {stray:?}");
    assert_eq!(
        window.get_master_pan().to_string(),
        pan_before_text,
        "⑩ 手势已经结束 ⇒ 孤立的 `move` 不许再改主控声相"
    );
    let late_up = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        334,
        METHOD_DISPATCH_POINTER_UP,
        Some(json!({"button": "left"})),
    ));
    assert!(
        !late_up.is_error(),
        "⑩ 迟到松手必须落到真实窗口: {late_up:?}"
    );
    assert_eq!(port.display().undoable, 0, "⑩ 迟到松手不许提交");
    assert_eq!(
        master_readings(&port.project()).1.to_bits(),
        before.1.to_bits(),
        "⑩ 迟到松手之后主控 pan 仍必须逐位等于起点"
    );
    report_line(&format!(
        "[mixer-master-②pan] `mixer-master-pan` 起点 {pan_before_text:?} → 拖动期 \
         {pan_dragging_text:?}（工程逐位仍 {} / undoable 0）→ 松手 {committed_pan} / undoable 1 \
         → `Cmd+Z` 后 {:?} → `Escape` 后 {pan_after_escape_text:?}（工程逐位仍 {} / undoable {}）\
         → 孤立 move ＋ 迟到松手后 undoable {}",
        before.1,
        window.get_master_pan(),
        master_readings(&port.project()).1,
        port.display().undoable,
        port.display().undoable
    ));

    // ---- 收尾：主控与**所有通道条**都必须逐位回到起点（两类目标互不残留） ----
    assert_eq!(master_readings(&port.project()), before);
    for (index, expected) in tracks_before.iter().enumerate() {
        assert_eq!(
            mixer_readings(&port.project(), &view, index),
            *expected,
            "第 {index} 轨不该被主控手势波及"
        );
    }
    // 撤销日志证明"主控的两项能力各自走到了端口"（不是只改了界面属性）。
    let names: Vec<String> = port
        .records()
        .iter()
        .map(|record| record.action.to_owned())
        .collect();
    report_line(&format!(
        "[mixer-master] 主控两项能力的撤销日志（共 {} 条 undo）: {names:?}；最后 undoable={}",
        names.len(),
        port.display().undoable
    ));
}

/// 判据（`ADR-0005` S1 的**第三条收尾路径**）：`Escape` **取消**混音手势（推子 / 声相），
/// 写回起点值、**不提交**，并且手势标志归位 —— 与 `cancel_track_height_drag` 同口径。
///
/// ## 这条判据为什么必须存在
///
/// 在本次改动之前，混音手势只有"松手"这一条收尾路径：`.slint` 的 `pointer-event` 把
/// `up` / `cancel` 都送到 `mixer-*-release`，宿主在 `release` 里提交。拖动中途按 `Escape`
/// **什么都不发生** —— 用户按了取消，界面上数值仍停在拖到的位置，随后松手还会提交。
///
/// ## 四条路径各自的字面读数（都在本判据里当场读出来）
///
/// | # | 情形 | 语义元素 | 轨道下标 | 模型字段 | 期望 |
/// | :--- | :--- | :--- | :--- | :--- | :--- |
/// | ① | 拖动期 | `track-0-fader` | 0 | `TrackV3::volume_db` | 文本已动、工程逐位未动、`undoable` 0、标志 `true` |
/// | ② | `Escape`（**真事件源**） | 同上 | 0 | 同上 | 文本写回起点、工程逐位未动、`undoable` 0、标志 `false` |
/// | ③ | `Escape` 后的孤立 `move` ＋ 迟到的松手 | 同上 | 0 | 同上 | 文本仍是起点、`undoable` 仍 0（手势真的结束了） |
/// | ④ | `Escape`（**回调那一格**）＋ 孤立 `move` | `track-1-pan` | 1 | `TrackV3::pan` | 文本 `R50 → C`、工程逐位未动、`undoable` 0 |
/// | ⑤ | 只松手（对照） | `track-2-fader` | 2 | `TrackV3::volume_db` | **恰一步** `undoable`、`Cmd+Z` 逐位回退 |
///
/// ## 怎么变红（注入的字面红行见交付报告）
///
/// | 注入 | 位置 | 现象 |
/// | :--- | :--- | :--- |
/// | 摘掉 `ui.on_mixer_cancel_gesture(..)` | `src/host.rs` 的 `wire_mixer_edit` | ② 步：标志仍是 `true`、文本仍是 `-33.2` ⇒ 红 |
/// | 摘掉 `cancel_mixer_gesture` 里的 `preview_target_volume` 写回 | `src/host.rs` | ② 步：文本仍是 `-33.2` ⇒ 红 |
/// | 让取消走提交（`cancel_mixer_gesture` 里 `commit_ops`） | `src/host.rs` | ② 步：`undoable` 涨到 1、工程字段被改 ⇒ 红 |
#[test]
fn escape_cancels_the_mixer_fader_and_pan_drag_without_committing() {
    use std::rc::Rc;

    use serde_json::json;
    use yeban_app::undo::{UndoPort, UndoSession};
    use yeban_ui_mcp::methods::{
        METHOD_DISPATCH_KEY_PRESS, METHOD_DISPATCH_POINTER_DOWN, METHOD_DISPATCH_POINTER_MOVE,
        METHOD_DISPATCH_POINTER_UP,
    };

    /// 会话打开时刻（与既有两个混音台判据同一个夹具常量）。
    const NOW: u64 = 1_760_000_000_000;

    let project = demo_project();
    let view = ViewState::from_project(&project).expect("演示投影");
    assert!(view.tracks.len() >= 3, "本判据需要至少 3 条非主总线轨道");
    // 起点读数（从**投影**读，与界面上的下标同源）。每一项能力用**互不共享**的
    // `(轨道下标, 模型字段)`：破坏推子那一条不会让声相那一条跟着绿（本会话的教训）。
    let before: Vec<(f32, f32, bool, bool)> = (0..3)
        .map(|index| mixer_readings(&project, &view, index))
        .collect();

    let port = Rc::new(UndoPort::new(
        UndoSession::open("<判据>", "yeban-app", project.clone(), NOW).expect("打开"),
    ));
    let live = build_live_ui_with(
        &project,
        &LiveWiringOptions {
            permission: Permission::Interactive,
            console_tab: 1,
            save_path: None,
            engine_quanta: 0,
            undo: Some(Rc::clone(&port)),
        },
    )
    .expect("装配");
    let window = slint::ComponentHandle::clone_strong(live.ui());

    // ---- 像素命题 ①：「默认帧可复现」= **确定性**（同一条装配、同一次运行抓两帧）----
    //
    // 这里**不碰 golden**（比的是同一次运行的两帧），也**不**用尺寸当证据：本仓 PNG 是
    // 存储式 deflate，1920×1080 恒为 6,222,418 字节。读数记为 PNG 字节的 sha256。
    // 命题 ②（"手势之后画面变了"）另有一条判据，见
    // `a_committed_mixer_fader_gesture_changes_the_frame`。
    let frame_default = live.capture().expect("默认外观帧");
    let frame_default_again = live.capture().expect("默认外观帧（第二次）");
    let (default_png, _default_evidence) =
        encode_with_evidence(&frame_default, DEFAULT_MAX_PNG_BYTES).expect("默认帧必须可编码");
    let (default_png_again, _) = encode_with_evidence(&frame_default_again, DEFAULT_MAX_PNG_BYTES)
        .expect("默认帧必须可编码");
    assert_eq!(
        default_png, default_png_again,
        "同一条装配、同一次运行里两次抓帧编码出的 PNG 必须逐字节相同（确定性）"
    );
    let default_digest = yeban_model::ids::ContentHash::of_bytes(&default_png);
    let artifact = write_artifact_bytes("mixer-escape-default-1920x1080.png", &default_png);
    report_line(&format!(
        "[mixer-escape-pixel] 默认外观（未注入任何手势）: {} 字节 / sha256={} / artifact={artifact}",
        default_png.len(),
        default_digest.as_str()
    ));

    let mut plane = live.into_control_plane(Permission::Interactive);
    assert_eq!(port.display().undoable, 0, "起点不该有可撤销的编辑");
    assert!(
        !window.get_mixer_drag_active(),
        "起点不该有混音手势在手（标志是「手势是否还没结束」的唯一窗口读数）"
    );

    // ================================================================ ① 推子：拖动期
    let fader = element_bounds(&mut plane, 601, "track-0-fader");
    let fader_x = fader["x"].as_f64().expect("x") + 11.0;
    let fader_y = fader["y"].as_f64().expect("y") + 8.0;
    let volume_before_text = injected_strings(&window.get_track_volumes())[0].clone();
    assert_eq!(volume_before_text, "-3.2", "① 注入前的推子文本");

    let down = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        602,
        METHOD_DISPATCH_POINTER_DOWN,
        Some(
            json!({"elementId": "track-0-fader", "xOffset": 11.0, "yOffset": 8.0, "button": "left"}),
        ),
    ));
    assert!(!down.is_error(), "① 推子按下必须落到真实窗口: {down:?}");
    assert!(
        window.get_mixer_drag_active(),
        "① 按下之后手势标志必须是 true（否则没有可取消的对象）"
    );
    // 向下拖 40px：88px 走完 66 dB（`-60..=6`）⇒ 40 ÷ 88 × 66 = 30 dB ⇒ -3.2 − 30 = -33.2。
    let moved = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        603,
        METHOD_DISPATCH_POINTER_MOVE,
        Some(json!({"x": fader_x, "y": fader_y + 40.0})),
    ));
    assert!(!moved.is_error(), "① 推子拖动必须落到真实窗口: {moved:?}");
    let volume_dragging_text = injected_strings(&window.get_track_volumes())[0].clone();
    assert_eq!(
        volume_dragging_text, "-33.2",
        "① 拖动期数值文本必须**立即**跟着动"
    );
    assert_eq!(
        port.display().undoable,
        0,
        "① 拖动期不许提交（每像素一次提交会把一步撤销碎成几百步）"
    );
    assert_eq!(
        mixer_readings(&port.project(), &view, 0).0.to_bits(),
        before[0].0.to_bits(),
        "① 拖动期工程里的 volume_db 必须**逐位**没动（视图态 ≠ 工程）"
    );
    assert!(window.get_mixer_drag_active(), "① 拖动期手势标志仍是 true");

    // ================================================================ ② `Escape`（真事件源）
    let pressed = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        604,
        METHOD_DISPATCH_KEY_PRESS,
        Some(json!({"keyCode": "Escape"})),
    ));
    assert!(!pressed.is_error(), "② `Escape` 注入必须成功: {pressed:?}");
    // 先判"取消 ≠ 提交"（工程与撤销栈一位不动），再判"视图态写回了起点"：
    // 两条失败模式（取消却提交 / 取消了但不写回）因此各有**指名**的红行。
    assert!(
        !window.get_mixer_drag_active(),
        "② `Escape` 必须让手势标志归位（否则就是粘住的手势态）：标志 = {}",
        window.get_mixer_drag_active()
    );
    assert_eq!(
        port.display().undoable,
        0,
        "② 取消**不是**提交：撤销栈一位不许涨"
    );
    assert_eq!(
        mixer_readings(&port.project(), &view, 0).0.to_bits(),
        before[0].0.to_bits(),
        "② 取消之后工程里的 volume_db 必须**逐位**等于起点"
    );
    let volume_after_escape_text = injected_strings(&window.get_track_volumes())[0].clone();
    assert_eq!(
        volume_after_escape_text, volume_before_text,
        "② `Escape` 必须把数值文本写回**起点**（与 `cancel_track_height_drag` 同口径）"
    );

    // ================================================================ ③ 孤立 move ＋ 迟到的松手
    let stray = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        605,
        METHOD_DISPATCH_POINTER_MOVE,
        Some(json!({"x": fader_x, "y": fader_y + 64.0})),
    ));
    assert!(!stray.is_error(), "③ 孤立 move 必须落到真实窗口: {stray:?}");
    assert_eq!(
        injected_strings(&window.get_track_volumes())[0].clone(),
        volume_before_text,
        "③ 手势已经结束 ⇒ 孤立的 `move` 不许再改音量"
    );
    assert_eq!(port.display().undoable, 0, "③ 孤立 `move` 不许提交");
    // 取消之后**真正的松手**也不许提交：这证明记录真的被取走了，而不只是标志翻了个面。
    let late_up = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        606,
        METHOD_DISPATCH_POINTER_UP,
        Some(json!({"button": "left"})),
    ));
    assert!(
        !late_up.is_error(),
        "③ 迟到松手必须落到真实窗口: {late_up:?}"
    );
    assert_eq!(
        port.display().undoable,
        0,
        "③ `Escape` 之后迟到的松手不许提交（否则「取消」是假的）"
    );
    assert_eq!(
        mixer_readings(&port.project(), &view, 0).0.to_bits(),
        before[0].0.to_bits(),
        "③ 迟到松手之后工程仍必须逐位等于起点"
    );
    report_line(&format!(
        "[mixer-escape-①fader] 起点 {volume_before_text} dB → 拖动期 {volume_dragging_text} dB\
         （工程仍 {} / undoable 0 / 标志 true）→ `Escape` 后文本 {volume_after_escape_text:?}\
         （工程逐位仍 {} / undoable {} / 标志 {}）→ 孤立 move ＋ 迟到松手之后 undoable {}",
        before[0].0,
        mixer_readings(&port.project(), &view, 0).0,
        port.display().undoable,
        window.get_mixer_drag_active(),
        port.display().undoable
    ));

    // ================================================================ ④ 声相：`Escape` 走**回调那一格**
    //
    // 第二条路径故意不经过端口：`invoke_key_action` 直接驱动 `.slint` 的 `key-action`
    // 回调（与平台事件源是同一个回调、同一张策略表）。两条路径都坏才会绿，反之都红。
    let pan = element_bounds(&mut plane, 611, "track-1-pan");
    let pan_x = pan["x"].as_f64().expect("x") + 10.0;
    let pan_y = pan["y"].as_f64().expect("y") + 7.0;
    let pan_before_text = injected_strings(&window.get_track_pans())[1].clone();
    assert_eq!(pan_before_text, "C", "④ 注入前的声相文本");
    let down = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        612,
        METHOD_DISPATCH_POINTER_DOWN,
        Some(
            json!({"elementId": "track-1-pan", "xOffset": 10.0, "yOffset": 7.0, "button": "left"}),
        ),
    ));
    assert!(!down.is_error(), "④ 声相按下必须落到真实窗口: {down:?}");
    assert!(window.get_mixer_drag_active(), "④ 声相手势在手");
    // 向右拖 9px：声相面 36px 走完 -1.0..=+1.0（跨度 2.0）⇒ 9 ÷ 36 × 2.0 = +0.5 ⇒ `R50`。
    let moved = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        613,
        METHOD_DISPATCH_POINTER_MOVE,
        Some(json!({"x": pan_x + 9.0, "y": pan_y})),
    ));
    assert!(!moved.is_error(), "④ 声相拖动必须落到真实窗口: {moved:?}");
    let pan_dragging_text = injected_strings(&window.get_track_pans())[1].clone();
    assert_eq!(pan_dragging_text, "R50", "④ 拖动期声相文本必须立即跟着动");
    assert_eq!(port.display().undoable, 0, "④ 拖动期不许提交");
    assert_eq!(
        mixer_readings(&port.project(), &view, 1).1.to_bits(),
        before[1].1.to_bits(),
        "④ 拖动期工程里的 pan 必须逐位没动"
    );
    assert!(
        window.invoke_key_action("\u{1b}".into(), false, false, false, false),
        "④ 声相手势在手时 `Escape` 必须被 DAW 消费（取消是落地实现）"
    );
    let pan_after_escape_text = injected_strings(&window.get_track_pans())[1].clone();
    assert_eq!(
        pan_after_escape_text, pan_before_text,
        "④ `Escape` 写回起点声相"
    );
    assert!(!window.get_mixer_drag_active(), "④ `Escape` 让手势标志归位");
    assert_eq!(port.display().undoable, 0, "④ 取消不是提交");
    assert_eq!(
        mixer_readings(&port.project(), &view, 1).1.to_bits(),
        before[1].1.to_bits(),
        "④ 取消之后工程里的 pan 必须逐位等于起点"
    );
    let stray = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        614,
        METHOD_DISPATCH_POINTER_MOVE,
        Some(json!({"x": pan_x + 30.0, "y": pan_y})),
    ));
    assert!(!stray.is_error(), "④ 孤立 move 必须落到真实窗口: {stray:?}");
    assert_eq!(
        injected_strings(&window.get_track_pans())[1].clone(),
        pan_before_text,
        "④ 手势已经结束 ⇒ 孤立的 `move` 不许再改声相"
    );
    // 每条情形都以一次**真正的松手**收场：`Escape` 已经取走了记录，这一松手因此什么都不做
    // —— 顺带证明"取消之后迟到的收尾"是干净的（指针抓取不会被留给下一条情形）。
    let late_up = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        615,
        METHOD_DISPATCH_POINTER_UP,
        Some(json!({"button": "left"})),
    ));
    assert!(
        !late_up.is_error(),
        "④ 迟到松手必须落到真实窗口: {late_up:?}"
    );
    assert_eq!(port.display().undoable, 0, "④ 迟到松手不许提交");
    assert!(
        !window.get_mixer_drag_active(),
        "④ 迟到松手之后标志仍是 false"
    );
    report_line(&format!(
        "[mixer-escape-②pan] 起点 {pan_before_text} → 拖动期 {pan_dragging_text}\
         （工程仍 {} / undoable 0）→ `Escape`（回调那一格）后文本 {pan_after_escape_text:?}\
         （工程逐位仍 {} / undoable {} / 标志 {}）→ 孤立 move 后文本 {:?}",
        before[1].1,
        mixer_readings(&port.project(), &view, 1).1,
        port.display().undoable,
        window.get_mixer_drag_active(),
        injected_strings(&window.get_track_pans())[1].clone()
    ));

    // 没有手势在手、**且时光机是关的**时，`Escape` 照旧**放行**（既有取向：`Action::Cancel`
    // 的语义在"没有可收尾的东西"这一点上一位不改；时光机关着时的收尾项就是零个）。
    let tree_open_before = port.undo_tree_open();
    assert!(
        !tree_open_before,
        "本步的前提是时光机**关着**（`Escape` 此时没有可收尾的对象）"
    );
    assert!(
        !window.invoke_key_action("\u{1b}".into(), false, false, false, false),
        "没有手势在手、时光机关着时 `Escape` 必须如实放行（而不是假装处理了）"
    );

    // ================================================================ ⑤ 对照：只松手 ⇒ 提交一次
    let fader2 = element_bounds(&mut plane, 621, "track-2-fader");
    let fader2_x = fader2["x"].as_f64().expect("x") + 11.0;
    let fader2_y = fader2["y"].as_f64().expect("y") + 8.0;
    let volume2_before_text = injected_strings(&window.get_track_volumes())[2].clone();
    assert_eq!(volume2_before_text, format!("{:.1}", before[2].0));
    let down = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        622,
        METHOD_DISPATCH_POINTER_DOWN,
        Some(
            json!({"elementId": "track-2-fader", "xOffset": 11.0, "yOffset": 8.0, "button": "left"}),
        ),
    ));
    assert!(!down.is_error(), "⑤ 按下必须落到真实窗口: {down:?}");
    assert!(
        window.get_mixer_drag_active(),
        "⑤ 按下之后手势标志必须是 true（否则这一拖根本没有开始）"
    );
    let moved = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        623,
        METHOD_DISPATCH_POINTER_MOVE,
        Some(json!({"x": fader2_x, "y": fader2_y + 40.0})),
    ));
    assert!(!moved.is_error(), "⑤ 拖动必须落到真实窗口: {moved:?}");
    assert_eq!(
        injected_strings(&window.get_track_volumes())[2].clone(),
        "-38.4",
        "⑤ 拖动期文本必须立即跟着动（-8.4 − 30 = -38.4）"
    );
    assert_eq!(port.display().undoable, 0, "⑤ 松手之前仍不许提交");
    let up = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        624,
        METHOD_DISPATCH_POINTER_UP,
        Some(json!({"button": "left"})),
    ));
    assert!(!up.is_error(), "⑤ 松手必须落到真实窗口: {up:?}");
    assert!(
        !window.get_mixer_drag_active(),
        "⑤ 松手之后手势标志必须归位（三条路径一条都不能留下粘住的状态）"
    );
    assert_eq!(
        port.display().undoable,
        1,
        "⑤ 对照：**只松手**（不按 `Escape`）⇒ 恰一步可撤销的编辑"
    );
    let committed = mixer_readings(&port.project(), &view, 2).0;
    assert_eq!(
        format!("{committed:.1}"),
        "-38.4",
        "⑤ 松手后工程里的 volume_db 必须真的是 -8.4 − 30 = -38.4 dB（读回来证明）"
    );
    assert!(
        window.invoke_key_action("z".into(), false, false, false, true),
        "⑤ `Cmd+Z` 必须被 DAW 消费"
    );
    assert_eq!(
        mixer_readings(&port.project(), &view, 2).0.to_bits(),
        before[2].0.to_bits(),
        "⑤ `Cmd+Z` 必须把 volume_db **逐位**还原到起点"
    );
    assert_eq!(port.display().undoable, 0, "⑤ 回退之后没有可撤销的编辑");
    report_line(&format!(
        "[mixer-escape-③release-commits] 起点 {volume2_before_text} dB → 拖动期\
         undoable 0 → 只松手 ⇒ 工程 {committed:.4} dB / undoable 1 → `Cmd+Z` ⇒\
         工程 {} dB（起点 {:?}）/ undoable 0",
        mixer_readings(&port.project(), &view, 2).0,
        before[2].0,
    ));

    // 收尾：三条轨道都必须逐位回到起点（取消与回退都不许残留）。
    for (index, expected) in before.iter().enumerate() {
        assert_eq!(
            mixer_readings(&port.project(), &view, index),
            *expected,
            "第 {index} 轨在本判据收尾时必须逐位回到起点"
        );
    }
}

/// 判据（像素**命题 ②**）：一次**提交了的**推子手势真的换了画面 —— 与 `Escape` 那条判据的
/// 命题 ①（「默认帧可复现」= 确定性）**分开陈述**，两者不可互相代替。
///
/// 驱动入口是 `MainWindow` 的三个混音台回调（`.slint` 的 `TouchArea` 调的就是它们，
/// 见 `ui/console/mixer_console.slint:318-328`）—— 与真实指针事件同一条链。
/// `LiveUi::capture` 是 `ui/screenshot` 内部用的**同一个** `capture_tier1`，因此这里
/// 不需要第二套像素路径，也**不碰 golden**（比的是同一次运行的两帧）。
#[test]
fn a_committed_mixer_fader_gesture_changes_the_frame() {
    use std::rc::Rc;

    use yeban_app::undo::{UndoPort, UndoSession};

    /// 会话打开时刻（与其它混音台判据同一个夹具常量）。
    const NOW: u64 = 1_760_000_000_000;

    let project = demo_project();
    let view = ViewState::from_project(&project).expect("演示投影");
    assert_eq!(
        mixer_readings(&project, &view, 0).0,
        -3.2,
        "0 号轨的音量起点是 -3.2 dB"
    );

    // 混音台的写入面只在装配给了撤销端口时接上（`live_surface` 的既有契约）。
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<判据>", "yeban-app", project.clone(), NOW).expect("打开"),
    ));
    let live = build_live_ui_with(
        &project,
        &LiveWiringOptions {
            permission: Permission::Interactive,
            console_tab: 1,
            save_path: None,
            engine_quanta: 0,
            undo: Some(Rc::clone(&port)),
        },
    )
    .expect("装配");
    let window = slint::ComponentHandle::clone_strong(live.ui());
    // 推子元素的运行时几何（抓帧之前先读树：像素证据要能指名"差在哪个元素上"）。
    let fader_bounds = live
        .tree_snapshot()
        .find_by_id("track-0-fader")
        .and_then(|node| node.bounds)
        .expect("`track-0-fader` 必须在运行时树里带几何");

    let frame_before = live.capture().expect("手势之前抓帧");
    let frame_before_again = live.capture().expect("手势之前再抓一帧");
    assert_eq!(
        frame_before, frame_before_again,
        "同一次运行里两次抓帧必须逐像素相同（确定性；命题 ① 的另一面）"
    );

    // 一次推子手势：按下 → 向下拖 40px（-3.2 → -33.2 dB）→ 松手（提交）。
    window.invoke_mixer_fader_grab(0, 100.0);
    assert!(window.get_mixer_drag_active(), "手势在手");
    window.invoke_mixer_fader_drag(0, 140.0);
    window.invoke_mixer_fader_release(0);
    assert!(!window.get_mixer_drag_active(), "松手之后手势标志归位");
    assert_eq!(
        injected_strings(&window.get_track_volumes())[0].clone(),
        "-33.2",
        "手势之后推子文本是 -33.2 dB"
    );

    let frame_after = live.capture().expect("手势之后抓帧");
    let frame_after_again = live.capture().expect("手势之后再抓一帧");
    let diff = frame_diff(&frame_before, &frame_after).unwrap_or_else(|| {
        panic!("工程真的改了、界面文本也变了，但两帧**逐字节相同** —— 画面没有跟着动")
    });
    assert_eq!(
        frame_after, frame_after_again,
        "手势之后两次抓帧也必须逐像素相同（否则上面的差异里混进了不确定性）"
    );
    assert!(
        diff.bbox.x < fader_bounds.right()
            && diff.bbox.right() > fader_bounds.x
            && diff.bbox.y < fader_bounds.bottom()
            && diff.bbox.bottom() > fader_bounds.y,
        "差异必须触及推子元素的运行时几何 {fader_bounds:?}（数值文本与推子帽都在它周围）: diff {:?}",
        diff.bbox
    );
    // 差异**落在哪**：混音台自身的矩形之外改了多少像素。这一格是**读数**而不是断言 ——
    // 提交会连带刷新撤销显示，那一处本来就在混音台之外；"不该动的地方动了"要靠这个数
    // 说话，而不是靠"差异非空"。
    let console_bounds = live
        .tree_snapshot()
        .find_by_id("mixer-console")
        .and_then(|node| node.bounds)
        .unwrap_or(fader_bounds);
    let mut outside_console = 0_u64;
    for y in 0..frame_before.height() {
        for x in 0..frame_before.width() {
            let inside = (x as i32) >= console_bounds.x
                && (x as i32) < console_bounds.right()
                && (y as i32) >= console_bounds.y
                && (y as i32) < console_bounds.bottom();
            if !inside && frame_before.pixel(x, y) != frame_after.pixel(x, y) {
                outside_console += 1;
            }
        }
    }
    report_line(&format!(
        "[mixer-pixel-②] 提交一次推子手势（-3.2 → -33.2 dB）之后: 差异 {} 像素 / 包围盒 {:?}\
         （推子元素几何 {:?} / 混音台矩形 {:?}）；混音台之外差异 {outside_console} 像素；\
         手势前两帧逐像素相同 = {}、手势后两帧逐像素相同 = {}",
        diff.count,
        diff.bbox,
        fader_bounds,
        console_bounds,
        frame_before == frame_before_again,
        frame_after == frame_after_again
    ));
}

/// 判据 7c（**像素命题 ②**，主控通道条）：提交一次**主控**推子手势之后，画面**真的变了** ——
/// 给差异像素数与差异包围盒（命题 ① "默认帧可复现" 是另一件事，见混音台那一对判据）。
///
/// 驱动入口是 `MainWindow` 的**主控**混音回调（`.slint` 的 `mixer-master-fader` 那个
/// `TouchArea` 调的就是它们，见 `ui/console/mixer_console.slint`）——
/// 与真实指针事件同一条链（后者由判据 7b 走端口）。
/// 比的是**同一次运行**的两帧，**不碰 golden**，也不用尺寸当证据
/// （本仓 PNG 是存储式 deflate，1920×1080 恒为 6,222,418 字节）。
#[test]
fn a_committed_master_fader_gesture_changes_the_frame() {
    use std::rc::Rc;

    use yeban_app::undo::{UndoPort, UndoSession};

    /// 会话打开时刻（与其它混音台判据同一个夹具常量）。
    const NOW: u64 = 1_760_000_000_000;

    let project = demo_project();
    assert_eq!(
        master_readings(&project),
        (0.0, 0.0),
        "主总线起点 0.0 dB / 居中"
    );

    let port = Rc::new(UndoPort::new(
        UndoSession::open("<判据>", "yeban-app", project.clone(), NOW).expect("打开"),
    ));
    let live = build_live_ui_with(
        &project,
        &LiveWiringOptions {
            permission: Permission::Interactive,
            console_tab: 1,
            save_path: None,
            engine_quanta: 0,
            undo: Some(Rc::clone(&port)),
        },
    )
    .expect("装配");
    let window = slint::ComponentHandle::clone_strong(live.ui());
    let master_fader_bounds = live
        .tree_snapshot()
        .find_by_id("mixer-master-fader")
        .and_then(|node| node.bounds)
        .expect("`mixer-master-fader` 必须在运行时树里带几何");

    let frame_before = live.capture().expect("手势之前抓帧");
    let frame_before_again = live.capture().expect("手势之前再抓一帧");
    assert_eq!(
        frame_before, frame_before_again,
        "同一次运行里两次抓帧必须逐像素相同（命题 ① 的另一面）"
    );

    // 一次主控推子手势：按下 → 向下拖 44px（0.0 → −33.0 dB）→ 松手（提交）。
    window.invoke_mixer_master_fader_grab(100.0);
    assert!(window.get_mixer_drag_active(), "主控手势在手");
    window.invoke_mixer_master_fader_drag(144.0);
    window.invoke_mixer_master_fader_release();
    assert!(!window.get_mixer_drag_active(), "松手之后手势标志归位");
    assert_eq!(
        window.get_master_volume_display().to_string(),
        "-33.0",
        "手势之后主控推子文本是 −33.0 dB"
    );
    assert_eq!(port.display().undoable, 1, "恰一步可撤销的编辑");
    assert_eq!(
        format!("{:.1}", master_readings(&port.project()).0),
        "-33.0",
        "工程里的主总线 volume_db 真的改了"
    );

    let frame_after = live.capture().expect("手势之后抓帧");
    let frame_after_again = live.capture().expect("手势之后再抓一帧");
    let diff = frame_diff(&frame_before, &frame_after).unwrap_or_else(|| {
        panic!("工程真的改了、界面文本也变了，但两帧**逐字节相同** —— 画面没有跟着动")
    });
    assert_eq!(
        frame_after, frame_after_again,
        "手势之后两次抓帧也必须逐像素相同（否则上面的差异里混进了不确定性）"
    );
    assert!(
        diff.bbox.x < master_fader_bounds.right()
            && diff.bbox.right() > master_fader_bounds.x
            && diff.bbox.y < master_fader_bounds.bottom()
            && diff.bbox.bottom() > master_fader_bounds.y,
        "差异必须触及主控推子元素的运行时几何 {master_fader_bounds:?}: diff {:?}",
        diff.bbox
    );
    let console_bounds = live
        .tree_snapshot()
        .find_by_id("mixer-console")
        .and_then(|node| node.bounds)
        .unwrap_or(master_fader_bounds);
    let mut outside_console = 0_u64;
    for y in 0..frame_before.height() {
        for x in 0..frame_before.width() {
            let inside = (x as i32) >= console_bounds.x
                && (x as i32) < console_bounds.right()
                && (y as i32) >= console_bounds.y
                && (y as i32) < console_bounds.bottom();
            if !inside && frame_before.pixel(x, y) != frame_after.pixel(x, y) {
                outside_console += 1;
            }
        }
    }
    report_line(&format!(
        "[mixer-master-pixel-②] 提交一次**主控**推子手势（0.0 → −33.0 dB）之后: 差异 {} 像素 / \
         包围盒 {:?}（主控推子元素几何 {:?} / 混音台矩形 {:?}）；混音台之外差异 {outside_console} \
         像素；手势前两帧逐像素相同 = {}、手势后两帧逐像素相同 = {}",
        diff.count,
        diff.bbox,
        master_fader_bounds,
        console_bounds,
        frame_before == frame_before_again,
        frame_after == frame_after_again
    ));
}

/// 判据 20（**编辑 ⇒ 发声**端到端，本票）：在**同一个活窗口**上做一次真实的混音台推子
/// 手势 ⇒ `UndoPort` 真的提交一步 ⇒ 控制线程心跳一跳 ⇒ **引擎快照里那一个字段**变了。
///
/// 它证明的那句话是："改一个混音参数之后，播放侧**真的**会看到新值。" 断点两侧都是**字面
/// 读数**：
///
/// | 时刻 | 读数 | 期望 |
/// | :--- | :--- | :--- |
/// | 起引擎之后 | `engine_track_volume_db(track0)` | `-3.2`（= 工程起点） |
/// | 手势之后、心跳之前 | 同上 | **仍是** `-3.2`（这就是本票修的那个断点） |
/// | 心跳之后 | 同上 | `-33.2`（= 提交进工程的那一版） |
///
/// 手势走的是 `MainWindow` 的**三个混音台回调**（`.slint` 的 `TouchArea` 调的就是它们，
/// 见 `ui/console/mixer_console.slint:320-326`）—— 与真实指针事件同一条链（后者另有判据 7）。
///
/// ## 会变红的注入（实测见交付报告）
///
/// | 注入 | 位置 | 现象 |
/// | :--- | :--- | :--- |
/// | 心跳里跳过 `publish_project`（"编辑不发布"） | `src/main.rs` / `engine_tick` | 第 3 行的读数仍是 `-3.2` ⇒ 红 |
/// | 心跳里跳过 `prune`（"发布了不回收"） | `EngineHost::heartbeat` | 第 2 跳 `released == 0` / `pending_len == 1` ⇒ 红 |
#[test]
fn a_mixer_gesture_reaches_the_engine_snapshot_on_the_live_window() {
    use std::rc::Rc;

    use yeban_app::engine_host::EditMark;
    use yeban_app::undo::{UndoPort, UndoSession};

    /// 会话打开时刻（与其它混音台判据同一个夹具常量）。
    const NOW: u64 = 1_760_000_000_000;

    let project = demo_project();
    let view = ViewState::from_project(&project).expect("演示投影");
    let ids = track_ids(&project);
    assert!(!ids.is_empty(), "演示工程至少有一条非主总线轨道");
    let track_id = ids[0];

    // 端口先建好并交给装配路径（与判据 7 同款：装配忘接混音台会被抓到）。
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<判据>", "yeban-app", project.clone(), NOW).expect("打开"),
    ));
    let mut live = build_live_ui_with(
        &project,
        &LiveWiringOptions {
            permission: Permission::Interactive,
            console_tab: 1,
            save_path: None,
            engine_quanta: 0,
            undo: Some(Rc::clone(&port)),
        },
    )
    .expect("装配");

    // 起一代引擎（与 `ui/reload_engine` 同一条路径，0 个量子 ⇒ 不空转）。
    live.start_engine().expect("起引擎");
    let opened_db = live
        .engine_track_volume_db(&track_id)
        .expect("快照里必须有这条轨道");
    assert_eq!(
        opened_db,
        mixer_readings(&port.project(), &view, 0).0,
        "起引擎之后：快照里的音量 = 工程起点"
    );
    assert_eq!(opened_db, -3.2, "0 号轨（鼓）的音量起点是 -3.2 dB");
    let opened_counts = live.engine_snapshot_counts().expect("有引擎必有计数");
    assert_eq!(opened_counts.published, 1, "reload 发了一份");
    assert_eq!(opened_counts.pending_len, 0, "基线：写者侧清单为空");

    // ---- 真实手势：三个混音台回调（下拖 40px = 66 dB × 40 ÷ 88 = 30 dB）----
    let window = live.ui();
    window.invoke_mixer_fader_grab(0, 8.0);
    window.invoke_mixer_fader_drag(0, 48.0);
    window.invoke_mixer_fader_release(0);
    let committed_db = mixer_readings(&port.project(), &view, 0).0;
    assert_eq!(
        format!("{committed_db:.1}"),
        "-33.2",
        "手势之后工程里的 volume_db 必须真的变了（模型读数）"
    );
    assert_eq!(port.display().undoable, 1, "一次拖动 = 恰一步可撤销");
    assert_eq!(
        live.engine_track_volume_db(&track_id).expect("快照"),
        opened_db,
        "★ 发布之前：引擎快照**仍是**旧值 —— 这就是本票修的那个断点"
    );

    // ---- 心跳第一跳：按需增量发布 + 回收（`run_gui` 的 60Hz 定时器逐字复制）----
    let mark = EditMark::from_display(&port.display());
    let first = live.engine_heartbeat(mark, Some(&port.project()));
    assert_eq!(first.published_revision, Some(2), "这一跳必须发布新快照");
    assert_eq!(
        first.readings.published, 2,
        "槽里累计发布 2 次（1 起 + 1 增量）"
    );
    assert_eq!(
        first.readings.pending_len, 1,
        "读者（音频侧）还没走过这个纪元 ⇒ 那一份必须保持存活"
    );
    assert_eq!(first.readings.released, 0, "读者没确认之前一条都不许释放");
    let edited_db = live
        .engine_track_volume_db(&track_id)
        .expect("快照里必须有这条轨道");
    assert_ne!(
        edited_db, opened_db,
        "★ 编辑 ⇒ 发声：快照里那一个字段必须不同"
    );
    assert_eq!(edited_db, committed_db, "快照读到的就是提交进工程的那一版");
    report_line(&format!(
        "[engine-snapshot] 推子手势 {opened_db:.1} → {committed_db:.1} dB; \
         快照 {opened_db:.1} → {edited_db:.1} dB; published={} pending={}",
        first.readings.published, first.readings.pending_len
    ));

    // 重复标记 ⇒ **不**再发布（"没改就不克隆工程"这条成本契约）。
    let second = live.engine_heartbeat(mark, Some(&port.project()));
    assert_eq!(second.published_revision, None, "标记没变 ⇒ 不许再发一份");
    assert_eq!(second.readings.published, 2, "发布计数一位不动");

    // ---- 设备回调那一侧：读者换快照 ⇒ 旧快照进退役队列 ⇒ 下一跳由心跳回收 ----
    assert_eq!(live.drive_audio(1), 1, "推一个量子");
    assert_eq!(
        live.engine_snapshot_switches(),
        Some(1),
        "音频读路径（begin_block）真的换到了新快照"
    );
    let third = live.engine_heartbeat(mark, None);
    assert_eq!(third.published_revision, None, "没有新标记 ⇒ 不发布");
    assert_eq!(
        third.readings.released, 1,
        "写者侧那一份在读者走过之后被回收"
    );
    assert_eq!(third.readings.retired, 1, "读者交出的旧快照被 Drop");
    assert_eq!(third.readings.pending_len, 0, "回收之后清单回到基线");
    let end_counts = live.engine_snapshot_counts().expect("有引擎必有计数");
    assert_eq!(end_counts.published, 2, "总计两份：reload 1 + 编辑 1");
    assert_eq!(end_counts.pruned, 1, "累计回收 1 份");
    assert_eq!(end_counts.pending_len, 0, "编辑结束：写者侧清单回到基线");
    assert_eq!(end_counts.retire_pending, 0, "编辑结束：退役队列为空");
}

/// 工程里第 `index` 条非主总线轨道的混音四格 `(volume_db, pan, mute, solo)`。
///
/// 身份从**投影**取（`ViewState::tracks[index].id`），因此这里的读数与界面上的
/// `track-{index}-*` 寻址的是**同一条轨道**（不是两套下标口径）。
fn mixer_readings(
    project: &YebanProjectV1,
    view: &ViewState,
    index: usize,
) -> (f32, f32, bool, bool) {
    let id = view.tracks[index].id.clone();
    let track = project
        .tracks
        .values()
        .find(|track| track.id.to_canonical_string() == id)
        .unwrap_or_else(|| panic!("工程里必须有身份 {id} 的轨道"));
    (track.volume_db, track.pan, track.mute, track.solo)
}

/// **主总线**的 `(volume_db, pan)` —— 按 `master_bus_track_id` 从**模型**读。
///
/// 主总线不在 `view.tracks` 里（投影刻意排除它）⇒ [`mixer_readings`] 的"按下标取"口径
/// 对它**不适用**，必须按身份取。这同时是本判据的**独立基准**：它不从界面读，
/// 因此"改了没有"不依赖被测的写入面。
fn master_readings(project: &YebanProjectV1) -> (f32, f32) {
    let id = project.master_bus_track_id;
    assert!(!id.is_nil(), "工程必须有主总线身份（演示夹具）");
    let track = project
        .tracks
        .get(&id)
        .unwrap_or_else(|| panic!("主总线 {id} 必须在 tracks 里"));
    assert_eq!(
        track.kind,
        yeban_model::project::TrackKind::Master,
        "`master_bus_track_id` 指向的必须是一条 `TrackKind::Master`（与模型 `validate` 同口径）"
    );
    (track.volume_db, track.pan)
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
    // 界面 6 条 == 手里那份投影 6 条 ⇒ 长度契约成立 ⇒ 必须注入。
    // 这条腿一旦重新变成"无条件注入"（无牙），下面的 `LengthMismatch` 判据会红。
    let MeterPump::Applied(snapshot) = live.pump_meters() else {
        panic!("长度相等的这一跳必须注入（本判据的其余断言都靠这一跳）");
    };
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

/// 判据 6b（**长度契约，测试端口那条腿**）：界面 `track-names` 行数与手里那份投影的
/// 轨道数不等长 ⇒ 与生产路径**同一份实现**：抽干照做、**跳过注入**、如实回报
/// `MeterPump::LengthMismatch { rows, projected }`。
///
/// ## 它钉的缺口（2026-10-08 收敛）
///
/// 本文件走的执行面（`build_live_ui*` ⇒ `LiveAdminSurface`）原先是"抽干 ＋ 应用"的
/// **第二份**实现，而它**没有**生产路径那道长度闸（`host::pump_meters`，
/// `crates/yeban-app/src/host.rs:1958`）⇒ 无头 Tier-1 判据面与产品路径跑的是**两条腿**，
/// 判据面可能因此看不见产品路径上的真实行为（缺口登记在
/// `docs/ledger/feature-alignment.md` 的"60Hz 主线程电平心跳"行）。现在两处共用
/// **同一个函数**（`LiveAdminSurface::pump_meters` 只加"重抓树"这一步）；
/// 本判据在"这条腿重新变成无牙（无条件注入）"时变红。
///
/// ## 造法（与生产判据 3 同款，但走**另一条腿**）
///
/// 窗口上装载 `filled` 工程（3 条非主总线轨道），执行面手里的投影也是 `filled`；
/// 随后把 `demo` 工程的投影（6 条）**直接注入窗口**（`host::apply_view`）——
/// 这正是"控制面换了工程、而持有循环的那一段还拿着旧投影"的形态
/// （生产那条腿的对应判据是 `tests/production_meter_leg.rs` 的判据 3）。
///
/// ## 为什么要四个字面读数
///
/// 1. **等长**那一跳：注入的已知幅度真的进控件树（前提：这条腿本来有牙）；
/// 2. **不等长**那一跳：`LengthMismatch { rows: 6, projected: 3 }`，且电平数组一位没动；
/// 3. **抽干照做**：队列容量取 **1** ⇒ "这一跳有没有抽干"可以从
///    `MeterPublisher::dropped()` 直接读出来（没抽干 ⇒ 队列还满着 ⇒ 下一条推送被丢）；
/// 4. **反证**：把长度对齐回去，面板里那帧必须重新进界面（⇒ 上面那条读数不是
///    "这条腿永远不注入"）。
#[test]
fn the_live_surface_meter_leg_reports_a_length_mismatch_and_skips_the_injection() {
    let filled = yeban_model::samples::filled_project();
    let demo = demo_project();
    let filled_ids = track_ids(&filled);
    assert_eq!(filled_ids.len(), 3, "filled 工程有 3 条非主总线轨道");
    let demo_view = ViewState::from_project(&demo).expect("演示工程投影");
    assert_eq!(
        demo_view.tracks.len(),
        6,
        "演示工程有 6 条非主总线轨道（与 filled 的 3 条不同 ⇒ 本条判据有分辨力）"
    );

    let mut live = build_live_ui_with(&filled, &options(Permission::ReadOnly, 1)).expect("装配");
    // 容量 1：**唯一**的目的是让"有没有抽干"变成可读的数（`dropped()`）。
    let (mut publisher, collector) = meter_channel(1);
    live.adopt_meter_collector(collector);

    // ---- 第 1 步（前提）：等长那一跳，注入必须真的进界面与控件树 ---------------
    publisher.publish(&[MeterFrame::new(filled_ids[0], 7, 1.0, 1.0)]);
    let MeterPump::Applied(snapshot) = live.pump_meters() else {
        panic!("长度相等的这一跳必须注入");
    };
    assert_eq!(snapshot.quantum, Some(7), "抽到的是注入的那个量子");
    assert_eq!(snapshot.tracks.len(), 3, "面板按手里那份投影生成 3 条");
    assert_eq!(publisher.dropped(), 0, "容量 1 推一条必须写得进去");
    let injected_label = label_of(&live.tree_snapshot(), "track-0-meter");
    assert!(
        injected_label.ends_with("峰值 0.0 RMS 0.0 dBFS"),
        "满幅帧必须进控件树, 实际 {injected_label:?}"
    );
    report_line(&format!(
        "[meter-leg-live] 等长那一跳: rows=3 projected=3 ⇒ track-0-meter={injected_label:?}\
         （注入 peak=1.0 ⇒ 0.0 dBFS）"
    ));

    // ---- 第 2 步：把**另一个长度**的投影注入窗口 ⇒ 手里那份变成陈旧的 ----------
    let width = slint::ComponentHandle::window(live.ui()).size().width as f32;
    host::apply_view(live.ui(), &demo_view, width, 0.0);
    assert_eq!(
        injected_strings(&live.ui().get_track_names()).len(),
        6,
        "窗口此刻是演示工程的 6 条轨道"
    );
    let after_apply_view = injected_strings(&live.ui().get_track_meter_peaks());
    assert_eq!(
        after_apply_view.len(),
        6,
        "`apply_view` 把电平数组重置成 6 条"
    );
    assert!(
        after_apply_view.iter().all(|label| label == "-120.0"),
        "`apply_view` 必须写静音（不是留下上一份工程的电平）, 实际 {after_apply_view:?}"
    );

    // ---- 第 3 步：不等长那一跳 ⇒ 只抽干、不注入、回报两个长度 ----------------
    // 命题②（**像素**）：先抓一帧（此刻界面已经是演示工程那个投影），走完这一跳再抓一帧。
    // ⚠ 这与"默认帧可复现"是**两个命题**：那条量的是外观的确定性，这条量的是
    // "这一跳有没有副作用"。不注入 ⇒ 两帧必须逐字节相同（不同像素 0 个）。
    let frame_before = live.capture().expect("Tier-1 截图（不等长那一跳之前）");
    publisher.publish(&[MeterFrame::new(filled_ids[0], 11, 1.0, 1.0)]);
    let pump = live.pump_meters();
    let MeterPump::LengthMismatch { rows, projected } = pump else {
        panic!("长度不等长时必须跳过注入并回报 LengthMismatch, 实际 {pump:?}");
    };
    assert_eq!(rows, 6, "回报的是界面此刻的长度（演示工程 6 条）");
    assert_eq!(projected, 3, "回报的是手里那份投影的长度（filled 的 3 条）");
    let after_mismatch = injected_strings(&live.ui().get_track_meter_peaks());
    assert_eq!(
        after_mismatch, after_apply_view,
        "长度不等长 ⇒ 电平数组必须一位没动（注入会按 `.slint` 的下标串到别的轨道）"
    );
    let frame_after = live.capture().expect("Tier-1 截图（不等长那一跳之后）");
    let diff = frame_diff(&frame_before, &frame_after);
    assert!(
        diff.is_none(),
        "不等长那一跳必须一个像素都不改（这一跳只抽干、不注入）, 实际不同像素 {} 个, 包围盒 {:?}",
        diff.as_ref().map_or(0, |diff| diff.count),
        diff.as_ref().map(|diff| diff.bbox)
    );
    report_line(&format!(
        "[meter-leg-live] 长度不等长: rows={rows} projected={projected} ⇒ 界面仍是 \
         {:?}（注入的量子 11 没有进界面）; 这一跳前后两帧不同像素 0 个",
        &after_mismatch[..3]
    ));

    // ---- 第 3b 步：抽干照做（队列容量 1 ⇒ 没抽干会让下一条推送被丢） ----------
    publisher.publish(&[MeterFrame::new(filled_ids[0], 15, 0.5, 0.25)]);
    assert_eq!(
        publisher.dropped(),
        0,
        "不等长那一跳必须仍然抽干队列 —— 没抽干时容量 1 的队列还满着, 这条推送会被丢"
    );

    // ---- 第 4 步（反证）：对齐回去 ⇒ 面板里那帧必须重新进界面 ----------------
    host::apply_view(
        live.ui(),
        &ViewState::from_project(&filled).expect("filled 投影"),
        width,
        0.0,
    );
    let MeterPump::Applied(resnapshot) = live.pump_meters() else {
        panic!("长度对齐之后必须重新注入");
    };
    assert_eq!(
        resnapshot.quantum,
        Some(15),
        "面板里最新的那帧是量子 15（⇒ 前面几跳的帧真的被消费了）"
    );
    let replayed = injected_strings(&live.ui().get_track_meter_peaks());
    assert_eq!(
        replayed[0], "-6.0",
        "对齐之后界面必须回到注入值（peak=0.5 ⇒ 20·log10(0.5) = -6.02 ⇒ \"-6.0\"）"
    );
    report_line(&format!(
        "[meter-leg-live] 对齐回去那一跳: rows=3 projected=3 ⇒ track-0-meter 回到 {}（量子 {:?}）",
        replayed[0], resnapshot.quantum
    ));
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
    let MeterPump::Applied(snapshot) = live.pump_meters() else {
        panic!("长度相等的这一跳必须注入（本判据读的就是注入后的读数）");
    };
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

/// 判据 21（**运行中换工程 1/2：控制面入口**）：`ui/open_project` 走**真的** JSON-RPC 文本
/// 换掉整份工程 —— 语义元素族、行几何、电平元素族与像素全部跟着换；失败路径
/// **当前工程一位不动**。
///
/// ## 它钉的缺口
///
/// 在这一条之前，产品与控制面**在进程跑起来之后**都无法换工程：唯一的打开入口是启动时的
/// CLI `--open`（`crates/yeban-app/src/main.rs` 的 `cli::load_project`），控制面 15 条
/// `ui/*` 方法里没有一条叫 `ui/open_project`。本判据证明新方法真的落到**同一个活窗口**上，
/// 而且复用**同一条**容器读取核心（`yeban_app::open` 的 `read_capped` ＋
/// `open_project_archive`；本方法用其上的 `open_project_file`）与**同一个**注入点
/// （`LiveAdminSurface::apply_project`）—— 因此它不造第二个权威。
///
/// ## 两个工程的选取（分辨力）
///
/// `filled`（3 条非主总线轨）与 `demo`（6 条）互不相同 ⇒ "轨道数真的变了"是一个有分辨力的
/// 读数；两者都带真实摆放，因此换工程后像素**必须**变。
#[test]
fn admin_open_project_swaps_the_whole_projection_through_the_control_plane() {
    use yeban_ui_mcp::live::ScreenshotProbe;
    use yeban_ui_mcp::methods::METHOD_OPEN_PROJECT;

    let a = yeban_model::samples::filled_project();
    let b = demo_project();
    assert_eq!(track_ids(&a).len(), 3, "A 有 3 条非主总线轨道");
    assert_eq!(track_ids(&b).len(), 6, "B 有 6 条非主总线轨道");

    let dir = scratch_dir("open-project-plane");
    let b_path = dir.join("b.yeban");
    // 目标是**真的落在磁盘上**的容器（`ui/open_project` 读的是文件，不是内存里的投影）。
    yeban_app::save::save_project_file(&b, &b_path).expect("写 B 容器");
    assert_eq!(
        open_project_file(&b_path).expect("B 可被权威入口读回"),
        b,
        "磁盘上的 B 必须与内存里的 B 逐字段相同（否则下面的读数不是 B 的）"
    );
    let a_path = dir.join("a.yeban");
    let mut wiring = options(Permission::Administrative, 1);
    wiring.save_path = Some(a_path.clone());
    let mut plane = build_live_ui_with(&a, &wiring)
        .expect("装配")
        .into_control_plane(Permission::Administrative);

    // ---- 起点（A）：3 条轨 / 3 个电平元素 / 没有第 4 条 / 落点还不存在 ----
    let (tree_a, json_a) = plane.plane().tree().expect("ui/tree(A)");
    assert_eq!(
        family_member_count(&tree_a, "track-", "-header"),
        3,
        "A 应当有 3 个轨道头"
    );
    assert_eq!(
        family_member_count(&tree_a, "track-", "-meter"),
        3,
        "A 应当有 3 个电平元素（Tab 1 = 调音台 ⇒ 电平元素族在运行时树里）"
    );
    assert!(tree_a.find("track-3-header").is_none());
    assert!(!a_path.exists(), "打开之前 A 的落点不该存在");
    let shot_a = ScreenshotProbe::parse(
        &plane
            .plane()
            .call(
                "ui/screenshot",
                Some(serde_json::json!({"maskDynamic": false})),
            )
            .expect("ui/screenshot(A)"),
    )
    .expect("A 的截图证据链必须闭合");
    report_line(&format!(
        "[open-project-plane] A: headers={} meters={} pngBytes={} fingerprint={}",
        family_member_count(&tree_a, "track-", "-header"),
        family_member_count(&tree_a, "track-", "-meter"),
        shot_a.png_bytes,
        shot_a.fingerprint
    ));

    // ---- 换工程：真的发一行 JSON-RPC 文本 ----
    let opened = plane
        .plane()
        .call(
            METHOD_OPEN_PROJECT,
            Some(serde_json::json!({
                "path": b_path.to_string_lossy(),
                "saveFirst": true,
            })),
        )
        .unwrap_or_else(|error| panic!("`{METHOD_OPEN_PROJECT}` 必须成功: {error}"));
    assert_eq!(opened["accepted"], true);
    assert_eq!(opened["operation"], "open_project");
    assert_eq!(opened["saveFirst"], true);
    let report = &opened["report"];
    assert_eq!(report["operation"], "open_project");
    assert_eq!(report["savedFirst"], true);
    // 三个**回读**读数：投影轨道数 / 界面行数 / 界面电平数组长度。
    assert_eq!(report["tracks"], 6, "投影的轨道数必须变成 B 的 6");
    assert_eq!(report["rows"], 6, "界面 `track-names` 的行数必须跟着变");
    assert_eq!(
        report["meterRows"], 6,
        "电平数组必须与轨道数组**等长**（这正是 `MeterPump::LengthMismatch` 要防的串轨）"
    );
    assert_eq!(report["tracks"], report["rows"]);
    assert_eq!(report["rows"], report["meterRows"]);

    // ---- 终点（B）：三条**独立**读法都必须看见新工程 ----
    let (tree_b, json_b) = plane.plane().tree().expect("ui/tree(B)");
    assert_eq!(
        family_member_count(&tree_b, "track-", "-header"),
        6,
        "B 应当有 6 个轨道头（轨道数真的变了）"
    );
    assert_eq!(
        family_member_count(&tree_b, "track-", "-meter"),
        6,
        "电平元素也必须变成 6 个（一个轨道一个电平元素 ⇒ 没有串轨）"
    );
    assert!(
        tree_b.find("track-5-header").is_some(),
        "B 的第 6 个轨道头必须真的进运行时树"
    );
    assert_ne!(json_a, json_b, "线上树文本必须变");
    // 行几何跟着变：B 的最后一个轨道头的包围盒必须比 A 的树里任何东西都靠下。
    let node_b = plane
        .plane()
        .probe(&ProbeOptions::new("track-5-header", "轨道").without_masking())
        .expect("ui/node 必须看见 B 的最后一个轨道头");
    let bounds_b = node_b.node.bounds.expect("轨道头必须有几何");
    assert!(bounds_b.width > 0 && bounds_b.height > 0, "{bounds_b:?}");
    report_line(&format!(
        "[open-project-plane] B: headers=6 meters=6 track-5-header bounds={bounds_b:?}"
    ));

    // ---- `saveFirst`：旧工程被原样写到**它自己的**落点 ----
    assert!(
        a_path.is_file(),
        "`saveFirst: true` 必须先把旧工程落盘（否则那条默认值是一句空话）"
    );
    assert_eq!(
        open_project_file(&a_path).expect("A 的落点可读回"),
        a,
        "写出去的必须是**旧**工程（不是刚打开的那一份）"
    );

    // ---- 像素：换工程后画面变了（**两个命题分开**） ----
    // 命题①"默认帧可复现"＝确定性，由本文件既有的像素判据与本次交付报告里的 sha256 覆盖；
    // 命题②"换工程后画面变了"＝本判据的读数。PNG 是**存储式 deflate** ⇒ 字节数不随内容变
    // （1920×1080 恒 6,222,418），所以判内容只能比**指纹/sha256**，不能比长度。
    let shot_b = ScreenshotProbe::parse(
        &plane
            .plane()
            .call(
                "ui/screenshot",
                Some(serde_json::json!({"maskDynamic": false})),
            )
            .expect("ui/screenshot(B)"),
    )
    .expect("B 的截图证据链必须闭合");
    assert_eq!(
        (shot_b.width, shot_b.height),
        (shot_a.width, shot_a.height),
        "换工程不改视口尺寸"
    );
    assert_ne!(
        shot_b.fingerprint, shot_a.fingerprint,
        "换工程必须改变画面（指纹相同 ⇒ 界面根本没换）"
    );
    report_line(&format!(
        "[open-project-plane] 换工程前后像素: A fingerprint={} B fingerprint={} (pngBytes {} / {})",
        shot_a.fingerprint, shot_b.fingerprint, shot_a.png_bytes, shot_b.png_bytes
    ));

    // ---- 失败路径 1：不存在的路径 ⇒ 既有错误码 ＋ 当前工程一位不动 ----
    let missing = dir.join("does-not-exist.yeban");
    let failure = plane
        .plane()
        .call(
            METHOD_OPEN_PROJECT,
            Some(serde_json::json!({"path": missing.to_string_lossy(), "saveFirst": false})),
        )
        .expect_err("不存在的路径必须失败");
    let yeban_ui_mcp::live::ProbeError::RpcFailed { code, .. } = failure else {
        panic!("必须是 JSON-RPC 错误: {failure:?}");
    };
    assert_eq!(
        code, -32005,
        "端口拒绝走**既有**的 `NOT_IMPLEMENTED`（D25: 不发明新错误码）"
    );
    let (tree_after_failure, json_after_failure) = plane.plane().tree().expect("ui/tree(失败之后)");
    assert_eq!(
        json_after_failure, json_b,
        "失败的打开必须让当前工程一位不动（半换是硬错误）"
    );
    assert_eq!(
        family_member_count(&tree_after_failure, "track-", "-header"),
        6
    );
    let shot_after_failure = ScreenshotProbe::parse(
        &plane
            .plane()
            .call(
                "ui/screenshot",
                Some(serde_json::json!({"maskDynamic": false})),
            )
            .expect("ui/screenshot(失败之后)"),
    )
    .expect("证据链必须闭合");
    assert_eq!(
        shot_after_failure.fingerprint, shot_b.fingerprint,
        "失败的打开必须一个像素都不改"
    );

    // ---- `dryRun`（先问后做，D48）：只读预览必须**真读盘**，且一个字节都不改 ----
    let preview = plane
        .plane()
        .call(
            METHOD_OPEN_PROJECT,
            Some(serde_json::json!({
                "path": b_path.to_string_lossy(),
                "saveFirst": true,
                "dryRun": true,
            })),
        )
        .expect("这个装配（A 的落点已配置）的 dryRun 必须成功");
    assert_eq!(preview["dryRun"], true);
    assert_eq!(preview["stateUnchanged"], true);
    assert_eq!(
        preview["preview"]["effect"]["openedTracks"], 6,
        "预览必须报出**真读盘**得到的轨道数（不是编的）"
    );
    assert_eq!(preview["preview"]["effect"]["currentTracks"], 6);
    let (tree_after_dry_run, json_after_dry_run) = plane.plane().tree().expect("ui/tree(dryRun)");
    assert_eq!(json_after_dry_run, json_b, "dryRun 必须一个字节都不改");
    assert_eq!(
        family_member_count(&tree_after_dry_run, "track-", "-header"),
        6
    );
    report_line(&format!(
        "[open-project-plane] dryRun 预览: path={} saveFirst={} currentTracks={} openedTracks={}",
        preview["preview"]["effect"]["path"],
        preview["preview"]["effect"]["saveFirst"],
        preview["preview"]["effect"]["currentTracks"],
        preview["preview"]["effect"]["openedTracks"]
    ));
}

/// 判据 21b（**运行中换工程 1b：旧工程的未保存改动**）：`saveFirst` 默认 `true` ⇒
/// 当前工程**没有落点**时 `ui/open_project` **拒绝**（既有错误码），当前工程一位不动；
/// 显式给 `saveFirst: false`（＝明确同意丢弃）时才放行。
///
/// ## 为什么这条判据有分辨力
///
/// 同一次调用在同一份装配上给出**两个相反**的结局（拒绝 / 成功），因此它证明的不是
/// "这条方法总是失败"，而是"拒绝的**原因**真的是没有落点"。
/// 默认值取自既有工具 `yeban_close_project` 的 `saveFirst`（`arg_bool(call, "saveFirst", true)`）
/// —— 不是本切片发明的口径。
#[test]
fn open_project_refuses_to_discard_unsaved_work_when_there_is_no_save_target() {
    use yeban_ui_mcp::live::ProbeError;
    use yeban_ui_mcp::methods::METHOD_OPEN_PROJECT;

    let a = yeban_model::samples::filled_project();
    let b = demo_project();
    let dir = scratch_dir("open-project-unsaved");
    let b_path = dir.join("b.yeban");
    yeban_app::save::save_project_file(&b, &b_path).expect("写 B 容器");
    // 关键：**没有** `save_path`（样本形态 / 从没保存过）。
    let wiring = options(Permission::Administrative, 1);
    assert!(
        wiring.save_path.is_none(),
        "本判据的前提就是当前工程没有落点"
    );
    let mut plane = build_live_ui_with(&a, &wiring)
        .expect("装配")
        .into_control_plane(Permission::Administrative);
    let (_, json_before) = plane.plane().tree().expect("ui/tree(起点)");
    assert_eq!(
        family_member_count(
            &plane.plane().tree().expect("ui/tree").0,
            "track-",
            "-header"
        ),
        3
    );

    // ---- 默认（`saveFirst` 缺省 = true）⇒ 拒绝，因为无处可存 ----
    let refused = plane
        .plane()
        .call(
            METHOD_OPEN_PROJECT,
            Some(serde_json::json!({"path": b_path.to_string_lossy()})),
        )
        .expect_err("没有落点时默认的 saveFirst=true 必须被拒绝");
    let ProbeError::RpcFailed { code, message, .. } = refused else {
        panic!("必须是 JSON-RPC 错误: {refused:?}");
    };
    assert_eq!(code, -32005, "既有错误码（D25）");
    assert!(
        message.contains("saveFirst") && message.contains("落点"),
        "错误消息必须点名原因（并且与 `dryRun` 报的是**同一句话**）: {message}"
    );
    let (_, json_after_refusal) = plane.plane().tree().expect("ui/tree(拒绝之后)");
    assert_eq!(
        json_after_refusal, json_before,
        "被拒绝的打开必须让当前工程一位不动"
    );
    report_line(&format!(
        "[open-project-unsaved] saveFirst 缺省 ⇒ 拒绝 (code {code}): {message}"
    ));

    // ---- 同一条请求的 `dryRun` 必须报**同一句话**（D48："预览说的 == 真做会得到的"） ----
    let preview_refusal = plane
        .plane()
        .call(
            METHOD_OPEN_PROJECT,
            Some(serde_json::json!({
                "path": b_path.to_string_lossy(),
                "saveFirst": true,
                "dryRun": true,
            })),
        )
        .expect_err("dryRun 也必须拒绝（『注定失败』不许被预览成成功）");
    let ProbeError::RpcFailed {
        code: preview_code,
        message: preview_message,
        ..
    } = preview_refusal
    else {
        panic!("必须是 JSON-RPC 错误: {preview_refusal:?}");
    };
    assert_eq!(preview_code, code, "预览与真调用必须是同一个错误码");
    assert_eq!(
        preview_message, message,
        "预览与真调用必须是**同一句话**（两处各写一遍就会漂移）"
    );

    // ---- `saveFirst: false`（明确同意丢弃）⇒ 放行，同一个装配、同一份文件 ----
    let opened = plane
        .plane()
        .call(
            METHOD_OPEN_PROJECT,
            Some(serde_json::json!({
                "path": b_path.to_string_lossy(),
                "saveFirst": false,
            })),
        )
        .expect("`saveFirst: false` 必须放行（拒绝的原因确实是没有落点）");
    assert_eq!(opened["saveFirst"], false);
    assert_eq!(opened["report"]["savedFirst"], false);
    assert_eq!(opened["report"]["tracks"], 6);
    assert_eq!(opened["report"]["rows"], 6);
    assert_eq!(opened["report"]["meterRows"], 6);
    let (tree_after, _) = plane.plane().tree().expect("ui/tree(放行之后)");
    assert_eq!(family_member_count(&tree_after, "track-", "-header"), 6);
    report_line(&format!(
        "[open-project-unsaved] saveFirst=false ⇒ 放行: tracks={} rows={} meterRows={}",
        opened["report"]["tracks"], opened["report"]["rows"], opened["report"]["meterRows"]
    ));
}

/// 判据 22（**运行中换工程 2/2：窗口读数**）：换工程后**行数与电平数组等长**（没有串轨）、
/// 行几何跟着换，并且画面**真的**变了（不同像素数 + 包围盒）。
///
/// 为什么这一条不能并进判据 21：判据 21 走控制面（`ui/tree` / `ui/node` / `ui/screenshot`
/// 的**线上形态**），而"电平数组与轨道数组等长"这件事住在**活窗口的属性**上
/// （`track-names` / `track-meter-levels`），控制面没有把它们暴露成 `ui/property` 的读法。
/// `into_control_plane` 会把执行面装箱取走 ⇒ 窗口读数与控制面读数**不可兼得**，因此两条判据
/// 各走一条路（与 `live_surface.rs` 的 `dispatch_key_press` 存在同一个理由）。
///
/// ## 为什么"等长"是可判据的（而不是一句注释）
///
/// `host::apply_meters` 的第一件事就是 `debug_assert_eq!(row_count(track_names),
/// snapshot.tracks.len())`，而 `cargo test` 默认开着 debug 断言 ⇒ 电平数组与行数一旦不等长，
/// 这一跳会**直接 panic**（不是静默串轨）。本判据另外把两个长度**读出来**逐个断言，
/// 因为"没 panic"与"长度真的相等"不是同一句话。
#[test]
fn open_project_keeps_the_meter_arrays_aligned_with_the_track_rows() {
    let a = yeban_model::samples::filled_project();
    let b = demo_project();

    let dir = scratch_dir("open-project-window");
    let b_path = dir.join("b.yeban");
    yeban_app::save::save_project_file(&b, &b_path).expect("写 B 容器");
    let a_path = dir.join("a.yeban");
    let mut wiring = options(Permission::Administrative, 1);
    wiring.save_path = Some(a_path.clone());
    let mut live = build_live_ui_with(&a, &wiring).expect("装配");

    // ---- 起点（A）：3 行 / 3 条电平 / 轨道几何数组 3 长 ----
    let names_a = injected_strings(&live.ui().get_track_names());
    let meters_a = injected_lengths(&live.ui().get_track_meter_levels());
    let peaks_a = injected_strings(&live.ui().get_track_meter_peaks());
    let (geometry_a, _) = row_geometry_snapshot(live.ui());
    assert_eq!(names_a.len(), 3, "A 的界面行数");
    assert_eq!(meters_a.len(), names_a.len(), "起点就必须等长");
    assert_eq!(peaks_a.len(), names_a.len(), "起点就必须等长");
    assert_eq!(geometry_a[0].len(), 3, "A 的 `track-ys` 行数");
    assert_eq!(geometry_a[1].len(), 3, "A 的 `track-heights` 行数");
    let frame_a = live.capture().expect("Tier-1 截图(A)");

    // ---- 真的打开 B（同一条载体路径；权限闸门照过） ----
    live.open_project(&b_path.to_string_lossy(), true)
        .expect("`ui/open_project` 必须成功");

    // ---- ⭐ 串轨的第一道读数：这一跳**不许**是 `LengthMismatch` ----
    //
    // `host::pump_meters` 在注入前比"界面 `track-names` 行数 vs 手里那份投影的轨道数"。
    // 换工程之后两者必须**等长**；`LengthMismatch` 的含义正是"界面此刻的 6 行电平会与
    // 手里那份 3 轨的投影错位"（`MeterPump::LengthMismatch` 的文档点名它就是
    // "控制面换了工程"造出来的）。因此这一跳是**最直接**的串轨判据。
    let pump_after_open = live.pump_meters();
    assert!(
        !matches!(pump_after_open, MeterPump::LengthMismatch { .. }),
        "换工程之后手里那份投影必须与界面等长（LengthMismatch = 电平按下标串轨）: {pump_after_open:?}"
    );
    report_line(&format!(
        "[open-project-window] 换工程之后的一跳电平: {pump_after_open:?}"
    ));

    // ---- 终点（B）：行数 / 两条电平数组 / 两条**轨道**几何数组全部 6 长 ----
    // ⚠ 只断言 `track-ys` / `track-heights` 这两条**逐轨**数组：同一份快照里的
    // `clip-*` 与 `automation-lane-band-*` 是**逐片段 / 逐泳道**的，它们与轨道数不同长
    // 是正常的（把"行数"当成"所有几何数组的长度"是本仓库点名过的一种口径错误）。
    let names_b = injected_strings(&live.ui().get_track_names());
    let meters_b = injected_lengths(&live.ui().get_track_meter_levels());
    let peaks_b = injected_strings(&live.ui().get_track_meter_peaks());
    let (geometry_b, _) = row_geometry_snapshot(live.ui());
    assert_eq!(names_b.len(), 6, "B 的界面行数");
    assert_eq!(
        meters_b.len(),
        names_b.len(),
        "⭐ 电平数组必须与轨道数组**等长**（不等长 = `.slint` 按下标取会串轨）"
    );
    assert_eq!(
        peaks_b.len(),
        names_b.len(),
        "峰值标签数组也必须等长（同一个契约的另一个数组）"
    );
    for (index, label) in ["`track-ys`", "`track-heights`"].iter().enumerate() {
        assert_eq!(
            geometry_b[index].len(),
            6,
            "轨道几何数组 {label} 必须跟着换工程变成 6 长"
        );
    }
    assert_ne!(geometry_a, geometry_b, "行几何必须跟着换工程变");
    report_line(&format!(
        "[open-project-window] rows {} -> {}; meterLevels {} -> {}; meterPeaks {} -> {}; \
         trackYs {} -> {}; trackHeights {} -> {}",
        names_a.len(),
        names_b.len(),
        meters_a.len(),
        meters_b.len(),
        peaks_a.len(),
        peaks_b.len(),
        geometry_a[0].len(),
        geometry_b[0].len(),
        geometry_a[1].len(),
        geometry_b[1].len()
    ));

    // ---- 命题②：换工程后画面变了 —— 给**不同像素数 + 包围盒**（不是"看起来不一样"） ----
    let frame_b = live.capture().expect("Tier-1 截图(B)");
    let diff = frame_diff(&frame_a, &frame_b).expect("换工程必须改变像素");
    assert_eq!(
        (frame_b.width(), frame_b.height()),
        (frame_a.width(), frame_a.height()),
        "换工程不改视口尺寸"
    );
    report_line(&format!(
        "[open-project-window] 换工程前后像素: 不同像素 {} 个, 包围盒 {:?} (视口 {}x{})",
        diff.count,
        diff.bbox,
        frame_a.width(),
        frame_a.height()
    ));

    // ---- 旧工程被先保存，且**落点跟着新工程走** ----
    assert_eq!(
        open_project_file(&a_path).expect("A 的落点可读回"),
        a,
        "`saveFirst: true` 必须把旧工程写到它自己的落点"
    );
    // 落点必须已经指向 B：再抓一次 A 的落点内容并与 B 比，若落点没换，下一次保存会把
    // B 的内容写进 A 的文件里。
    let mut plane = live.into_control_plane(Permission::Administrative);
    let saved = plane
        .plane()
        .call("ui/force_save", None)
        .expect("换工程之后的保存必须成功");
    assert_eq!(saved["report"]["saveEpoch"], 1);
    // 落点 = `b_path` ⇒ B 的文件被重写；A 的文件**保持**上一跳写进去的内容。
    assert_eq!(
        open_project_file(&b_path).expect("B 的落点可读回"),
        b,
        "保存必须写到**新**工程的路径"
    );
    assert_eq!(
        open_project_file(&a_path).expect("A 的落点仍可读回"),
        a,
        "A 的文件不得被新工程覆盖（落点没换的话这里就红了）"
    );
}

/// 判据 20（`ROAD-M4-008` 选项 (a)：**单一写者会话**的 GUI 侧）：挂在控制面上时，
/// `ui/force_save` 写出的**不是**界面缓存，而是**权威会话当前那一版**。
///
/// ## 为什么这条判据有分辨力（而不是"两条路写出同样的字节"）
///
/// 关键是让"界面缓存"与"权威工程"**故意分叉**：会话侧改一次工程，但**不**调
/// `sync_authority` ⇒ 活窗口仍画着旧那一版；随后按保存。
///
/// | 走的路 | 结果 |
/// | :--- | :--- |
/// | 界面缓存（改动前的本地路径 `save_project_file`） | 要么写旧那一版（新泳道不在文件里），要么被会话的排他锁拒（`ui/force_save` 直接失败） |
/// | 权威（`ProjectAuthorityHandle::save_to`） | 文件里**有**那条新泳道，且回执 `writtenByAuthority = true` |
///
/// ## 论证顺序
///
/// | 步 | 动作 | 读回 |
/// | :--- | :--- | :--- |
/// | 1 | 在真工程文件上以**单一写者**形态挂载，并以权威装配真实界面（带 `save_path`） | 权威可写；窗口里没有 pan 泳道 |
/// | 2 | 真环回 socket 发 `yeban_edit_automation`（建 pan 泳道）；**不**同步投影 | 权威工程里有新泳道；活窗口仍**没有** —— 界面缓存确实是旧的 |
/// | 3 | `ui/force_save` | 成功，`report.bytes > 0`、`report.writtenByAuthority = true` |
/// | 4 | 用**后续读者**读同一个文件 | 新泳道**在**文件里 ⇒ 写的是权威那一版，不是陈旧缓存 |
#[cfg(feature = "in-process-mcp")]
#[test]
fn a_gui_force_save_while_mounted_writes_through_the_authority() {
    use std::collections::BTreeMap;
    use yeban_app::mcp_mount::{InProcessMcp, SessionSource};

    // ---- 夹具：磁盘上一份真容器（与 `in_process_mcp_lock.rs` 同一份数据）----
    let dir = scratch_dir("single-writer-save");
    let project_path = dir.join("mounted.yeban");
    let project = yeban_model::samples::filled_project();
    let history = serde_json::to_vec(&yeban_model::CommitGraph::new()).expect("空图谱 JSON");
    let bytes =
        yeban_model::container::write_project_container(&project, &history, &BTreeMap::new())
            .expect("写真容器");
    std::fs::write(&project_path, &bytes).expect("写夹具");

    // ---- 1. 单一写者形态挂载 + 以权威装配界面（带 save_path）----
    let mount = InProcessMcp::start_for_project(
        true,
        yeban_model::samples::filled_project(),
        SessionSource::WritableFile(project_path.clone()),
    )
    .expect("挂载决策")
    .expect("运行期开关打开时必须真的挂载");
    let authority = mount.project_authority();
    assert!(authority.is_writable(), "写形态会话必须可写");
    let bearer = format!("Bearer {}", mount.token().expose());

    let authority_view =
        ViewState::from_project(&authority.project().expect("权威有活跃工程")).expect("投影");
    let lead = authority_view.tracks.first().expect("有轨道");
    let lead_id = lead.id.clone();
    let lane_id = format!("track-{}-automation-pan-lane", lead.index);
    assert!(
        !authority_view
            .automation_lane_element_ids()
            .contains(&lane_id),
        "起点：权威里不该有 `{lane_id}`"
    );

    let mut opts = options(Permission::Administrative, 0);
    opts.save_path = Some(project_path.clone());
    let mut plane = live::build_live_ui_from_authority_with(&authority, &opts)
        .expect("以权威装配真实界面")
        .into_control_plane(Permission::Administrative);
    let (tree_before, _) = plane.plane().tree().expect("ui/tree");
    assert!(
        tree_before.find(&lane_id).is_none(),
        "起点：活窗口里不该有 `{lane_id}`"
    );

    // ---- 2. 会话侧改工程，但**不**同步投影 ⇒ 界面缓存与权威故意分叉 ----
    let reply = mcp_call(
        mount.address(),
        &bearer,
        &format!(
            r#"{{"jsonrpc":"2.0","id":61,"method":"tools/call","params":{{"name":"yeban_edit_automation","arguments":{{"trackId":"{lead_id}","lane":"TrackPan","point":{{"tick":0,"value":0.25,"curve":"Linear"}}}}}}}}"#
        ),
    );
    assert_eq!(
        reply["result"]["status"], "success",
        "写类工具必须成功: {reply}"
    );
    assert!(
        ViewState::from_project(&authority.project().expect("权威有活跃工程"))
            .expect("投影")
            .automation_lane_element_ids()
            .contains(&lane_id),
        "权威工程里必须已经有 `{lane_id}`"
    );
    let (tree_stale, _) = plane.plane().tree().expect("ui/tree");
    assert!(
        tree_stale.find(&lane_id).is_none(),
        "故意不同步 ⇒ 活窗口必须**仍然**画着旧那一版（否则本判据的对照不成立）"
    );

    // ---- 3. `ui/force_save` ⇒ 走权威 ----
    let saved = plane
        .plane()
        .try_line(r#"{"jsonrpc":"2.0","id":62,"method":"ui/force_save"}"#);
    assert!(
        !saved.is_error(),
        "挂载时 `ui/force_save` 必须成功（否则它落到了被排他锁拒的本地路径上）: {saved:?}"
    );
    let result = saved.result.expect("有 result");
    assert_eq!(result["accepted"], true, "{result}");
    assert_eq!(
        result["report"]["writtenByAuthority"], true,
        "回执必须点名这条保存走了权威（不是界面缓存）: {result}"
    );
    let written = result["report"]["bytes"].as_u64().expect("字节数");
    assert!(written > 0);
    assert_eq!(
        std::fs::metadata(&project_path).expect("元数据").len(),
        written,
        "回执里的字节数必须等于磁盘上的字节数"
    );

    // ---- 4. 后续读者读回：写的是**权威当前那一版** ----
    let on_disk = open_project_file(&project_path).expect("后续读者读回");
    assert!(
        ViewState::from_project(&on_disk)
            .expect("投影")
            .automation_lane_element_ids()
            .contains(&lane_id),
        "文件里必须有会话侧刚建的那条泳道 `{lane_id}` ⇒ 保存用的是权威的字节，不是陈旧缓存"
    );

    mount.stop().expect("停机");
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

    // ---- 副作用：**注入的那一帧作废了**，界面转而显示新引擎自己的读数 ----
    //
    // ⚠ 这条判据的期望值在 `line/engine-sound` 合并后**必须改**（CI run 37244018311 红在这里）:
    // 旧期望是"电平回到下限 −120 dB", 那个期望**建立在"渲染是占位静音"之上** ——
    // 真实合成接上后, 新引擎对"鼓"轨立刻渲染出真实电平（实测 峰值 −6.0 / RMS −23.4 dBFS）,
    // 于是"回下限"不再成立。**但判据真正要证明的东西没变**: 注入的 0.0/0.0 必须被丢掉、
    // 界面必须接上**新引擎的电平队列**。所以改成断言那件真正的事:
    //   ① 注入值（0.0/0.0）确实不在了; ② 读数是新引擎的真实、有限、非下限的读数。
    // 这是"跨线前提是隐式契约"的第二个实例（第一个是 engine-sound 自己发现的 S3）:
    // 凡是"因为另一处还没实现, 所以这里可以这样测"的判据, 前提消失时都要主动复核。
    let (after, _) = plane.plane().tree().expect("ui/tree");
    let track_label = label_in(&after, "track-0-meter");
    assert_ne!(
        track_label, "轨道 鼓 电平表 峰值 0.0 RMS 0.0 dBFS",
        "引擎换代之后**注入的旧读数必须作废** —— 界面不许继续显示换代前那一帧"
    );
    let master_label = label_in(&after, "mixer-master-meter");
    assert_ne!(
        master_label, "主控电平表 峰值 0.0 RMS 0.0 dBFS",
        "主控电平也必须来自新引擎的队列"
    );
    // 读数必须是**真实合成**的结果: 有限、且不在静音下限上。
    for (what, label) in [("轨道", &track_label), ("主控", &master_label)] {
        let peak =
            dbfs_value(label).unwrap_or_else(|| panic!("{what} 电平标签里应当有峰值: {label}"));
        assert!(
            peak.is_finite() && peak > -120.0,
            "{what} 换代后应当有新引擎的**真实读数**(有限且高于 −120 dB 下限), 实际: {label}"
        );
    }

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

// ===========================================================================
// `ui-mcp-dryrun-ime` 工作线：`dryRun`（ADR-0001 **D48**）与 IME 合成态
// ===========================================================================
//
// 这一半的判据必须在**真实窗口**上跑（Tier-1 光栅化 + 真控件树 + 真 `MainWindow`
// 属性），因为"dryRun 不改状态"这句话只有对着**真的会改状态**的执行面才有意义：
// 零 Slint 的假面那 10 条判据证明的是管线与词表，这里证明的是**接线**。
//
// 本机纪律禁止编译 Slint ⇒ 这 3 条**只能由 CI 判**（`docs/ledger/ui-mcp-dryrun-ime-notes.md`
// 的"本机 vs CI"一节）。本机跑过的对应判据是 `crates/yeban-ui-mcp/src/service.rs` 的那批。

/// 判据 13（D48）：`dryRun=true` 在**真实界面**上不改一个状态位，且它**预告**的事情
/// 与随后的真调用**逐字段一致**。
///
/// 观测面有四个，缺一不可：
/// - `ui/tree` 的**线上 JSON**（逐字节）；
/// - 磁盘（`ui/force_save` 的落点必须不存在）；
/// - `MainWindow.arrangement-view` 的回读（经 `ui/switch_main_view` 的回执）；
/// - 引擎代数（`ui/reload_engine` 的回执 `generation` 必须是**第一次**真调用推进的 1，而不是 2）。
#[test]
fn dry_run_leaves_the_live_window_and_the_disk_untouched() {
    let project = demo_project();
    let dir = scratch_dir("dry-run");
    let path = dir.join("dry-run-must-not-exist.yeban");
    let mut opts = options(Permission::Administrative, 1);
    opts.save_path = Some(path.clone());
    let mut plane = build_live_ui_with(&project, &opts)
        .expect("装配")
        .into_control_plane(Permission::Administrative);

    let (before, before_json) = plane.plane().tree().expect("ui/tree");

    // ① `ui/switch_main_view` 的 dryRun：预览说"会写进 arrangement-view = false"，
    //    而现在的读数是 true（默认 Arrangement）。
    let switched = plane.plane().try_line(
        r#"{"jsonrpc":"2.0","id":71,"method":"ui/switch_main_view","params":{"view":"session","dryRun":true}}"#,
    );
    assert!(!switched.is_error(), "dryRun 必须成功: {switched:?}");
    assert_eq!(switched.status, 200);
    let result = switched.result.expect("有 result");
    assert_eq!(result["dryRun"], true, "响应必须自证这是模拟");
    assert_eq!(result["stateUnchanged"], true);
    assert_eq!(result["wouldChangeState"], true);
    assert_eq!(result["method"], "ui/switch_main_view");
    assert_eq!(result["requiredScope"], "app:admin");
    assert_eq!(result["arguments"]["view"], "session");
    assert_eq!(result["preview"]["operation"], "switch_main_view");
    assert_eq!(
        result["preview"]["effect"]["currentArrangementView"], true,
        "预览必须报**当前**读数"
    );
    assert_eq!(
        result["preview"]["effect"]["arrangementView"], false,
        "预览必须报'将要变成什么'"
    );
    assert!(
        result.get("report").is_none(),
        "dryRun 不得产生（更不得取走）管理动作回执: {result}"
    );

    // ② `ui/force_save` 的 dryRun：预告 saveEpoch=1（**不是**"我又存了一次"）。
    let saved = plane
        .plane()
        .try_line(r#"{"jsonrpc":"2.0","id":72,"method":"ui/force_save","params":{"dryRun":true}}"#);
    assert!(!saved.is_error(), "dryRun 必须成功: {saved:?}");
    let saved = saved.result.expect("有 result");
    assert_eq!(saved["preview"]["operation"], "force_save");
    assert_eq!(saved["preview"]["effect"]["saveEpoch"], 1);
    assert_eq!(saved["preview"]["effect"]["containerEntries"], 2);

    // ③ `ui/reload_engine` 的 dryRun：预告 generation=1。
    let reloaded = plane.plane().try_line(
        r#"{"jsonrpc":"2.0","id":73,"method":"ui/reload_engine","params":{"dryRun":true}}"#,
    );
    assert!(!reloaded.is_error(), "dryRun 必须成功: {reloaded:?}");
    let reloaded = reloaded.result.expect("有 result");
    assert_eq!(reloaded["preview"]["operation"], "reload_engine");
    assert_eq!(reloaded["preview"]["effect"]["generation"], 1);

    // ④ `ui/dispatch_key_press` 的 dryRun：指针事件的影响只有窗口知道（如实报 null），
    //    但**按键**的处置是可判定的（`[UI-A11Y-002]`）。
    let pressed = plane.plane().try_line(
        r#"{"jsonrpc":"2.0","id":74,"method":"ui/dispatch_key_press","params":{"keyCode":"Space","dryRun":true}}"#,
    );
    assert!(!pressed.is_error(), "dryRun 必须成功: {pressed:?}");
    let pressed = pressed.result.expect("有 result");
    assert_eq!(pressed["preview"]["operation"], "dispatch_key");
    assert_eq!(
        pressed["preview"]["effect"]["resolution"], "action",
        "画布焦点 + 非合成态: `Space` 命中走带播放/暂停（真 `InputContext::resolve` 的回答）"
    );
    assert_eq!(pressed["preview"]["effect"]["isComposing"], false);

    let moved = plane.plane().try_line(
        r#"{"jsonrpc":"2.0","id":75,"method":"ui/dispatch_pointer_move","params":{"x":10.0,"y":20.0,"dryRun":true}}"#,
    );
    assert!(!moved.is_error(), "dryRun 必须成功: {moved:?}");
    let moved = moved.result.expect("有 result");
    assert_eq!(moved["preview"]["operation"], "dispatch_pointer");
    assert!(
        moved["preview"]["effect"].is_null(),
        "执行面给不出影响时必须如实报 null, 不许编造: {moved}"
    );

    // 状态一位都没变。
    assert!(!path.exists(), "dryRun 的 `ui/force_save` 写了盘");
    let (after, after_json) = plane.plane().tree().expect("ui/tree");
    assert_eq!(after_json, before_json, "dryRun 之后运行时树必须逐字节相同");
    assert_eq!(after.count, before.count);
    assert!(
        after.find("workspace-arrangement-canvas").is_some(),
        "dryRun 不得把视图切走"
    );

    // ---- 真做：预览说的就是实际发生的（计数从 0 起，而不是 2） ----
    let real_reload = plane
        .plane()
        .try_line(r#"{"jsonrpc":"2.0","id":76,"method":"ui/reload_engine"}"#);
    assert!(!real_reload.is_error(), "真重载必须成功: {real_reload:?}");
    let real_reload = real_reload.result.expect("有 result");
    assert_eq!(
        real_reload["report"]["generation"], 1,
        "dryRun 不得推进引擎代数（真调用才是第一次）"
    );

    let real_save = plane
        .plane()
        .try_line(r#"{"jsonrpc":"2.0","id":77,"method":"ui/force_save"}"#);
    assert!(!real_save.is_error(), "真保存必须成功: {real_save:?}");
    let real_save = real_save.result.expect("有 result");
    assert_eq!(real_save["report"]["saveEpoch"], 1);
    assert!(path.exists(), "真保存必须真的落盘");

    let real_switch = plane.plane().try_line(
        r#"{"jsonrpc":"2.0","id":78,"method":"ui/switch_main_view","params":{"view":"session"}}"#,
    );
    assert!(!real_switch.is_error(), "真切换必须成功: {real_switch:?}");
    let real_switch = real_switch.result.expect("有 result");
    assert_eq!(
        real_switch["report"]["arrangementView"], false,
        "真调用写进去的值必须与预览说的**逐字段相同**"
    );
    let (switched_tree, switched_json) = plane.plane().tree().expect("ui/tree");
    assert_ne!(switched_json, before_json, "真调用必须真的换掉控件树");
    assert!(switched_tree.find("workspace-session-canvas").is_some());
    report_line(&format!(
        "[ui-mcp-dryrun-ime] dryRun: 树 {} 字节未变; 预览 saveEpoch=1/generation=1/arrangementView=false 与真调用逐字段一致",
        before_json.len()
    ));
}

/// 判据 14（`[UI-A11Y-002]`）：IME 合成态在**真实界面**上可观测，且读的是
/// `LiveUi::input_context()` 交出的**那一个**状态机（不是影子变量）。
///
/// 两个方向都断言：非合成 → 合成 → 非合成（含"焦点离开文本域自动结束合成"的既有语义）。
#[test]
fn ime_composition_is_observable_on_the_live_window() {
    let project = demo_project();
    // ⚠ 必须是 **Interactive**（= 测试模式）：`ui/dispatch_key_press` 的 `dryRun` 也要过
    // `ui:inject` 这道闸门 —— CI run 37254896937 就是这么告诉我的：用 `ReadOnly`
    // （⇒ `RunMode::Production`）时它拿到 `403 forbidden-in-production`。
    // 那**不是**缺陷，是"dryRun 不绕过授权"这条对齐的直接后果（本机判据
    // `dry_run_keeps_every_existing_error_code` 的第 ④ 条钉着同一件事）。
    let ui = build_live_ui(&project, Permission::Interactive).expect("装配");
    // 驱动点：生产上是 Slint 平台的 IME 事件，判据里直接驱动**同一个**对象。
    let input = ui.input_context();
    let mut plane = ui.into_control_plane(Permission::Interactive);

    let read = |plane: &mut LiveControlPlane| {
        let call = plane.plane().try_line(
            r#"{"jsonrpc":"2.0","id":81,"method":"ui/property","params":{"elementId":"transport-bpm-field","name":"isComposing"}}"#,
        );
        assert!(!call.is_error(), "读 IME 位必须成功: {call:?}");
        call.result.expect("有 result")
    };

    // 方向 1：画布聚焦、没有合成。
    let idle = read(&mut plane);
    assert_eq!(idle["name"], "isComposing");
    assert_eq!(idle["value"], false);
    assert_eq!(idle["focus"], "main-canvas");
    assert_eq!(idle["specId"], "UI-A11Y-002");
    assert_eq!(idle["id"], "transport-bpm-field");

    // 文本域聚焦 + 开始合成（真的驱动状态机）。
    input
        .borrow_mut()
        .set_focus(yeban_app::input::Focus::TextInput);
    input.borrow_mut().begin_composition();
    let composing = read(&mut plane);
    assert_eq!(composing["value"], true, "合成态必须可观测");
    assert_eq!(composing["focus"], "text-input");
    assert_ne!(composing["value"], idle["value"], "两个方向必须可区分");

    // 同一份状态也被 dryRun 的按键预览读到（"Space 会被输入法吞掉"）。
    let press = plane.plane().try_line(
        r#"{"jsonrpc":"2.0","id":82,"method":"ui/dispatch_key_press","params":{"keyCode":"Space","dryRun":true}}"#,
    );
    assert!(!press.is_error(), "dryRun 必须成功: {press:?}");
    let press = press.result.expect("有 result");
    assert_eq!(
        press["preview"]["effect"]["resolution"], "consumed-by-ime",
        "合成态下 Space 必须被输入法吞掉 [UI-A11Y-002] MUST"
    );
    assert_eq!(
        press["preview"]["effect"]["isComposing"],
        composing["value"]
    );

    // 方向 2：焦点离开文本域 ⇒ `InputContext::set_focus` 自己结束合成态。
    input
        .borrow_mut()
        .set_focus(yeban_app::input::Focus::MainCanvas);
    let left = read(&mut plane);
    assert_eq!(left["value"], false);
    assert_ne!(left["value"], composing["value"], "方向 2");
    assert_eq!(left["value"], idle["value"]);
    report_line(
        "[ui-mcp-dryrun-ime] IME 位: 真实 `InputContext` 的两次读数 false -> true -> false 全部对得上",
    );
}

/// 判据 14b（`[UI-A11Y-002]` §7.2，**本线新增的那一半**）：合成态的**事件源**。
///
/// 判据 14 证明的是"状态机可观测"，它直接写 `input_context()` 那个对象 ——
/// 也就是说 `.slint` 一侧到底有没有事件源，它**证明不了**（这正是上一版 `transport.slint`
/// 只写着"先用 role=text-input 占位"时留下的缺口）。
///
/// 本判据从**事件源那一侧**驱动：`ui/transport.slint` 的 `bpm-input`
/// （真 `TextInput`）在 `changed preedit-text` / `changed has-focus` 里调用的两个回调，
/// 由 `host::wire_input` 接到**同一个** `InputContext`。链路因此是：
///
/// ```text
/// Slint TextInput.preedit-text --changed--> MainWindow.ime-composition-changed
///   --host::wire_input--> InputContext --ui/property--> isComposing
/// ```
///
/// ⚠ **诚实边界**：上游有信号（`preedit-text`，出处见 `ui/transport.slint` 的注释），
/// 但 Slint 1.18.1 **没有公开的 preedit 注入面**（`WindowEvent` 没有合成变体、
/// `InternalKeyEvent`/`KeyEventType` 未被再导出、`process_key_input` 是 `pub(crate)`），
/// 因此判据只能驱动到 `.slint` 回调这一格。原因与取舍写在
/// `docs/ledger/app-projection-notes.md` 的 needs 里，本判据的 `report_line` 也会打印它。
#[test]
fn ime_composition_flows_from_the_slint_event_source_into_the_control_plane() {
    let project = demo_project();
    let ui = build_live_ui(&project, Permission::Interactive).expect("装配");
    // 先取窗口强引用（`clone_strong` 是共享而不是复制），再交出控制面 ——
    // 事件源驱动点必须与观测点指向**同一个活窗口**。
    let window = slint::ComponentHandle::clone_strong(ui.ui());
    let mut plane = ui.into_control_plane(Permission::Interactive);

    let read = |plane: &mut LiveControlPlane| {
        let call = plane.plane().try_line(
            r#"{"jsonrpc":"2.0","id":83,"method":"ui/property","params":{"elementId":"transport-bpm-field","name":"isComposing"}}"#,
        );
        assert!(!call.is_error(), "读 IME 位必须成功: {call:?}");
        call.result.expect("有 result")
    };
    let press_space = |plane: &mut LiveControlPlane| {
        let call = plane.plane().try_line(
            r#"{"jsonrpc":"2.0","id":84,"method":"ui/dispatch_key_press","params":{"keyCode":"Space","dryRun":true}}"#,
        );
        assert!(!call.is_error(), "dryRun 必须成功: {call:?}");
        call.result.expect("有 result")["preview"]["effect"]["resolution"].clone()
    };

    // ① 起点：画布焦点、非合成。
    let idle = read(&mut plane);
    assert_eq!(idle["value"], false);
    assert_eq!(idle["focus"], "main-canvas");
    assert_eq!(press_space(&mut plane), "action");

    // ② 焦点进入敲入控件（`bpm-input` 的 `changed has-focus`）⇒ 焦点分类跟着变。
    window.invoke_ime_focus_changed(true);
    let focused = read(&mut plane);
    assert_eq!(
        focused["focus"], "text-input",
        "Slint 的焦点事件必须真的进状态机（不是影子变量）"
    );

    // ③ 合成开始（`changed preedit-text`：候选词非空）⇒ 位变 true。
    window.invoke_ime_composition_changed(true);
    let composing = read(&mut plane);
    assert_eq!(composing["value"], true, "合成态必须由**事件源**点亮");
    assert_ne!(composing["value"], idle["value"], "两态必须可区分");

    // ④ **行为断言**：合成中的 Space 被输入法吞掉（§7.2 MUST），不是"元素存在"。
    assert_eq!(
        press_space(&mut plane),
        "consumed-by-ime",
        "合成态下裸快捷键必须被吞掉"
    );

    // ⑤ 上屏 / 取消（preedit 清空）⇒ 位回落。
    window.invoke_ime_composition_changed(false);
    let ended = read(&mut plane);
    assert_eq!(ended["value"], false, "合成结束必须回落");
    assert_eq!(ended["value"], idle["value"]);
    // 焦点**仍在**敲入控件 ⇒ 裸快捷键交给文本控件（`input.rs` 的既有语义：
    // 文本域里连非合成态的裸快捷键都不冒泡成 DAW 动作）—— 这不是"吞键"，是"打字"。
    assert_eq!(
        press_space(&mut plane),
        "pass-through",
        "焦点在敲入控件时 Space 必须交给文本控件（不是 DAW 动作）"
    );

    // ⑥ 失焦 ⇒ 焦点回画布、裸快捷键恢复；**合成中失焦**也必须清零（既有语义的兜底）。
    window.invoke_ime_focus_changed(false);
    let canvas = read(&mut plane);
    assert_eq!(canvas["focus"], "main-canvas");
    assert_eq!(canvas["value"], false);
    assert_eq!(
        press_space(&mut plane),
        "action",
        "焦点回到画布 ⇒ Space 必须恢复成走带动作（否则守卫会永久吞键）"
    );
    window.invoke_ime_composition_changed(true);
    assert_eq!(read(&mut plane)["value"], true);
    window.invoke_ime_focus_changed(false);
    let blurred = read(&mut plane);
    assert_eq!(blurred["focus"], "main-canvas");
    assert_eq!(blurred["value"], false, "合成中失焦必须结束合成态");
    report_line(
        "[ui-mcp-dryrun-ime] Slint 事件源 → InputContext → isComposing: \
         false→true→false 全部对得上; 合成中 Space=consumed-by-ime, 文本域聚焦时 Space=pass-through, \
         焦点回画布后 Space=action; SKIP: 平台级 preedit 注入在 Slint 1.18.1 无公开入口（见 ledger needs）",
    );
}

/// 判据 15（D48）：`dryRun` 对"注定失败的真调用"**如实报错** —— 与真调用同码、同话、同 data。
///
/// 未配置保存路径（`LiveWiringOptions::save_path = None`）时 `ui/force_save` 会报
/// `-32005`（"这条能力在这个装配上没接线"）；`dryRun` 必须报**同一件事**，
/// 而不是给一份看起来成功的预览（领域侧 dryRun 的口径就是"只做参数与领域合法性校验"）。
#[test]
fn dry_run_reports_the_same_failure_as_the_real_save() {
    let project = demo_project();
    let mut plane = build_live_ui_with(&project, &options(Permission::Administrative, 0))
        .expect("装配")
        .into_control_plane(Permission::Administrative);

    let dry = plane
        .plane()
        .try_line(r#"{"jsonrpc":"2.0","id":91,"method":"ui/force_save","params":{"dryRun":true}}"#);
    let real = plane
        .plane()
        .try_line(r#"{"jsonrpc":"2.0","id":92,"method":"ui/force_save"}"#);
    assert!(dry.is_error(), "没有落点时必须失败: {dry:?}");
    assert!(real.is_error(), "没有落点时真调用也必须失败: {real:?}");
    assert_eq!(dry.code, real.code, "dryRun 与真调用必须同码");
    assert_eq!(
        dry.message, real.message,
        "连话都得一样: {dry:?} vs {real:?}"
    );
    assert_eq!(dry.kind, real.kind);
    assert_eq!(dry.status, real.status);
    report_line(&format!(
        "[ui-mcp-dryrun-ime] dryRun 如实报错: code={:?} kind={:?}（与真调用逐字段相同）",
        dry.code, dry.kind
    ));
}

// ===========================================================================
// `N2` 裁决 (1)：GUI 路径的**逻辑键**快捷键
// ===========================================================================
//
// `docs/ledger/open-questions.md` 问题 1 的裁决 (1)：**GUI 绑逻辑键，无头端口保留物理码判据**。
// 依据是 Slint 的公开按键事件只有 `text`（`KeyEvent { text, modifiers, repeat }`），
// 拿不到 `src/input.rs` 扫描码表要的物理键身份。
//
// 这一节判的就是裁决要求的那条判据：一个用**逻辑键**表达的快捷键，在**真实界面**上
// 真的作用到宿主状态。链路是：
//
// ```text
// ui/dispatch_key_press(KeyCode) --LivePort--> Slint WindowEvent::KeyPressed { text }
//   --ui/app.slint 的 key-handler(FocusScope).key-pressed--> callback key-action(text, 修饰位)
//   --host::wire_keys--> input::resolve_logical --> apply_action --> MainWindow 的属性
// ```
//
// 没有一处经过物理码；物理码那条路（`physical_key_of` + `InputContext::resolve`）保持原样，
// 仍由本文件其它判据与 `test_port_adapter.rs` / `undo.rs` 的判据覆盖。

/// 判据 16（`N2` 裁决 (1)）：逻辑键快捷键**真的**触发动作 —— 经端口的真事件源，也经回调那一格。
///
/// 三个观测面：
/// 1. `ui/dispatch_key_press` 注入 `"3"`（端口的 `KeyCode::Character` → `type_char` →
///    `WindowEvent::KeyPressed { text: "3" }`）⇒ `active-tool` 必须变成矩阵第 3 行；
/// 2. 同一个端口注入 `Tab` ⇒ 画布焦点下必须切视图，且**被消费**（`accept` 意味着
///    Slint 自己的 `Tab` 焦点轮转不再接管 —— 这正是 §2.2 MUST 的形态）；
/// 3. `invoke_key_action` 直接驱动 `.slint` 回调那一格：已绑定的键被消费、未绑定的键
///    与"这个装配没有撤销会话"的 `Cmd+Z` 如实 `reject`（不假装处理了）。
#[test]
fn a_logical_key_shortcut_from_the_event_source_reaches_the_host_action() {
    let project = demo_project();
    let ui = build_live_ui(&project, Permission::Interactive).expect("装配");
    let window = slint::ComponentHandle::clone_strong(ui.ui());
    let mut plane = ui.into_control_plane(Permission::Interactive);

    // ① 起点：矩阵第 1 行（选择工具）、Arrangement 视图。
    assert_eq!(window.get_active_tool(), 1, "默认工具是选择");
    assert!(window.get_arrangement_view(), "默认视图是 Arrangement");

    // ② 端口注入逻辑键 `"3"`（**没有**任何物理码参与）。
    let pressed = plane.plane().try_line(
        r#"{"jsonrpc":"2.0","id":101,"method":"ui/dispatch_key_press","params":{"keyCode":"3"}}"#,
    );
    assert!(!pressed.is_error(), "逻辑键注入必须成功: {pressed:?}");
    let after_digit = window.get_active_tool();
    assert_eq!(
        after_digit, 3,
        "逻辑键 `3` 必须真的切到剪刀工具（矩阵第 3 行）"
    );

    // ③ 同一个事件源注入 `Tab`：画布焦点 ⇒ 切视图，且这一键被 DAW 消费（不再做焦点轮转）。
    let tab = plane.plane().try_line(
        r#"{"jsonrpc":"2.0","id":102,"method":"ui/dispatch_key_press","params":{"keyCode":"Tab"}}"#,
    );
    assert!(!tab.is_error(), "Tab 注入必须成功: {tab:?}");
    let after_tab = window.get_arrangement_view();
    assert!(!after_tab, "画布聚焦时 `Tab` 必须切到 Session 视图");

    // ④ `.slint` 回调那一格直接驱动（同一个回调、同一张策略表）。
    assert!(
        window.invoke_key_action("5".into(), false, false, false, false),
        "已绑定的逻辑键 `5` 必须被消费（accept）"
    );
    let after_five = window.get_active_tool();
    assert_eq!(after_five, 5, "逻辑键 `5` 必须切到橡皮擦");
    assert!(
        window.invoke_key_action("b".into(), false, false, false, false),
        "`B` 必须被消费"
    );
    let after_b = window.get_active_tool();
    assert_eq!(after_b, 2, "`B` 把非铅笔工具切到铅笔（箭头 ⇄ 铅笔）");

    // ⑤ 未绑定的逻辑键**不得**被消费（`reject` ⇒ 事件继续冒泡 / 交给焦点系统）。
    assert!(
        !window.invoke_key_action("q".into(), false, false, false, false),
        "没有绑定的逻辑键不得被吞掉"
    );
    // ⑥ 撤销族在**本装配**上没有撤销会话（`wire_keys(.., None)`）⇒ 如实不消费。
    assert!(
        !window.invoke_key_action("z".into(), false, true, false, false),
        "没有撤销会话时 `Cmd+Z` 必须如实放行（而不是假装撤销了）"
    );

    // ⑦ 表里**已绑定、但宿主没有落地实现**的键：真事件源上必须如实 `reject`，并且没有副作用。
    //    这是 `--print-shortcuts` 把这两行标成 `(未实现)` 的行为侧证据（表的对账在判据 B11b）。
    let tool_before_zoom = window.get_active_tool();
    let view_before_zoom = window.get_arrangement_view();
    assert!(
        !window.invoke_key_action("z".into(), false, false, false, false),
        "`Z → 选区撑满视口` 尚无落地实现 ⇒ 必须如实 reject（不得假装缩放了）"
    );
    assert!(
        !window.invoke_key_action("Z".into(), true, false, false, false),
        "`Shift+Z → 全曲总览` 尚无落地实现 ⇒ 必须如实 reject（不得假装缩放了）"
    );
    assert_eq!(
        window.get_active_tool(),
        tool_before_zoom,
        "被拒绝的缩放键不得有副作用"
    );
    assert_eq!(
        window.get_arrangement_view(),
        view_before_zoom,
        "被拒绝的缩放键不得切视图"
    );

    report_line(&format!(
        "[n2-logical-keys] 逻辑键路径实测: 端口注入 `3` ⇒ active-tool 1→{after_digit}; \
         端口注入 `Tab` ⇒ arrangement-view true→{after_tab}; 回调注入 `5` ⇒ {after_five}、`B` ⇒ {after_b}; \
         未绑定的 `q`、无撤销会话的 `Cmd+Z`、尚无落地实现的 `Z` / `Shift+Z` 都如实 reject（物理码判据未改动）"
    ));
}

// ---------------------------------------------------------------------------
// 判据 17：`ROAD-M4-008` 选项 (a) —— 唯一可变权威是控制面会话，界面是它的投影
// ---------------------------------------------------------------------------

/// 真环回 socket 上的一次 JSON-RPC 往返（判据 17 的**客户端**那一半）。
///
/// **它不是第二份协议实现**：协议层（405 / 411 / 413 / 431 / 401 / 200）的判据住在
/// `crates/yeban-app/tests/in_process_mcp.rs` 与 `yeban-mcp` 的单元判据里，本文件
/// **不重复**它们。这里只需要"把一条 `tools/call` 送到那个监听口、把响应体读回来"。
///
/// 为什么走 socket 而不是宿主口：这条判据要证的是"**经 MCP 会话**的改动会到界面"，
/// 因此改动必须真的从一个客户端、带令牌、过鉴权与分发器进来 —— 那是外部 AI 的路径。
#[cfg(feature = "in-process-mcp")]
fn mcp_call(address: std::net::SocketAddr, bearer: &str, body: &str) -> serde_json::Value {
    use std::io::{Read as _, Write as _};
    use std::net::TcpStream;
    use std::time::Duration;

    let timeout = Duration::from_secs(5);
    let mut stream =
        TcpStream::connect_timeout(&address, timeout).expect("连接环回控制面（真 socket）");
    stream.set_read_timeout(Some(timeout)).expect("设置读超时");
    stream.set_write_timeout(Some(timeout)).expect("设置写超时");
    let head = format!(
        "POST {} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n\
         Authorization: {bearer}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        yeban_mcp::transport::http::MCP_PATH,
        body.len()
    );
    stream.write_all(head.as_bytes()).expect("写请求头");
    stream.write_all(body.as_bytes()).expect("写请求体");
    stream.flush().expect("刷出请求");
    let mut raw = String::new();
    stream
        .read_to_string(&mut raw)
        .expect("读响应（超时即失败）");
    let body = raw
        .split_once("\r\n\r\n")
        .unwrap_or_else(|| panic!("响应里没有头/体分隔符: {raw}"))
        .1;
    serde_json::from_str(body).unwrap_or_else(|error| panic!("响应体不是 JSON ({error}): {body}"))
}

/// 判据 17（`docs/ledger/open-questions.md` 问题 6 选项 (a)）：**经 MCP 会话改动工程 ⇒
/// 界面投影跟着变**，因为两者**只有一个**可变权威。
///
/// ## 这条判据要证的句子
///
/// `build_live_ui_from_authority` 装配出来的界面**没有**自己的工程来源：它只从
/// `ProjectAuthorityHandle`（= 控制面正在服务的那一个 `Domain`）取工程。因此一次 AI
/// 的工具调用改了会话的工程之后，`LiveUi::sync_authority` 依**施加修订号**重投影，
/// 真实 `MainWindow` 的语义元素随之改变 —— 而只读调用一位都不改。
///
/// ## 论证顺序（每一步都用上一步的实读值）
///
/// | 步 | 动作 | 读回 |
/// | :--- | :--- | :--- |
/// | 1 | 以权威装配真实界面（该入口**不接受**工程参数） | 运行时控件树里只有样本那一条泳道 `track-0-automation-volume-lane` |
/// | 2 | 一个**真客户端**带令牌在环回 socket 上发 `yeban_edit_automation`（在从没有泳道的 `TrackPan` 上写一个点） | `status = success`、`applied = true` |
/// | 3 | 从**宿主口**读同一个会话 | 施加修订号 **+1**；工程里真的多了 `track-0-automation-pan-lane` |
/// | 4 | `sync_authority` | `Reprojected { revision }`（不是 `Unchanged`） |
/// | 5 | 再读运行时控件树（同一个活窗口） | **新元素**在树里；它的 `accessible-label` 与"权威工程的投影"逐字相等 |
/// | 5b | **同案两帧差分**（`ADR-0005` S1，不碰 golden）：域侧改动前 / 后各抓一帧 | 两帧不同；差异包围盒落在被改轨道的**投影泳道带栈**内；区域外逐字节相同；指纹可复现 |
/// | 6 | 一次**只读**工具调用（`yeban_query_project`） | 修订号不动、`sync_authority = Unchanged` ⇒ 界面不是被查询刷来刷去 |
/// | 7 | `ui/tree` / `ui/node` / `ui/screenshot` 端到端 | 控制面服务的就是更新后的那一棵树（同一个窗口），且未遮罩截图与第 5b 步那一帧指纹相同 |
///
/// ## 这条判据怎么变红（负向实测见工作线报告）
///
/// - 让 `Domain::apply` 不推进 `apply_revision`（或 `Plan::mutates_project` 对
///   `EditAutomation` 恒为 `false`）⇒ 第 4 步变成 `Unchanged`，第 5 步的新元素不在树里；
/// - 让 `sync_authority` 拿**装配时**那一份工程（而不是每次从权威取）⇒ 同样在第 5 步变红；
/// - 把 `host::apply_view` 的注入摘掉 ⇒ 两帧**逐字节相同**，第 5b 步的差分断言先变红
///   （比第 5 步更早、更直接地指出"界面没跟着权威动"）；
/// - 让改动**溅到别的轨道**（例如同一次运行里再改一条别的轨道的泳道）⇒ 差异包围盒越出
///   被改轨道的投影带栈，第 5b 步变红。
#[cfg(feature = "in-process-mcp")]
#[test]
fn an_mcp_mutation_reaches_the_live_ui_projection_through_the_single_authority() {
    use yeban_app::mcp_mount::{InProcessMcp, SessionSource};

    let project = yeban_model::samples::filled_project();
    let mount = InProcessMcp::start_for_project(
        true,
        project,
        SessionSource::InMemory(std::path::PathBuf::from("sample:m4-008")),
    )
    .expect("挂载决策")
    .expect("运行期开关打开时必须真的挂载");
    let authority = mount.project_authority();
    let bearer = format!("Bearer {}", mount.token().expose());

    // ---- 1. 以权威为唯一工程来源装配真实界面 ----
    let mut ui = build_live_ui_from_authority(&authority, Permission::ReadOnly).expect("装配");
    let view_before =
        ViewState::from_project(&authority.project().expect("权威有活跃工程")).expect("投影");
    let lead_index = view_before.tracks[0].index;
    let lead_id = view_before.tracks[0].id.clone();
    let lane_id = format!("track-{lead_index}-automation-pan-lane");
    assert!(
        view_before
            .automation_lane_element_ids()
            .contains(&format!("track-{lead_index}-automation-volume-lane")),
        "起点：主轨必须已经有那条音量泳道（否则本判据的对照不成立）"
    );
    assert!(
        !view_before.automation_lane_element_ids().contains(&lane_id),
        "起点：`{lane_id}` 还不存在 —— 第 2 步要新建的就是它"
    );
    let tree_before = ui.tree_snapshot();
    assert!(
        !tree_before.contains(&lane_id),
        "起点：运行时控件树里不该有 `{lane_id}`"
    );
    let nodes_before = tree_before.len();
    let revision_before = authority.apply_revision();
    // 第 5b 步的**改动前**那一帧：域侧调用之前抓，走的仍是 `ui/screenshot` 内部那条
    // Tier-1 抓帧路径（`LiveUi::capture` ⇒ `capture_tier1`）。
    let frame_before = ui.capture().expect("域侧改动前抓帧");

    // ---- 2. 真客户端、真令牌、真环回 socket：一次**会改工程**的工具调用 ----
    let reply = mcp_call(
        mount.address(),
        &bearer,
        &format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"yeban_edit_automation","arguments":{{"trackId":"{lead_id}","lane":"TrackPan","point":{{"tick":0,"value":0.25,"curve":"Linear"}}}}}}}}"#
        ),
    );
    assert_eq!(
        reply["result"]["status"], "success",
        "写类工具必须成功: {reply}"
    );
    assert_eq!(reply["result"]["data"]["applied"], true, "{reply}");

    // ---- 3. 同一个会话：修订号 +1，工程里真的多了那条泳道 ----
    assert_eq!(
        authority.apply_revision(),
        revision_before + 1,
        "一次改工程的施加必须恰好推进一个修订号"
    );
    let project_after = authority.project().expect("权威有活跃工程");
    let view_after = ViewState::from_project(&project_after).expect("投影");
    assert!(
        view_after.automation_lane_element_ids().contains(&lane_id),
        "权威工程里必须出现 `{lane_id}`: {:?}",
        view_after.automation_lane_element_ids()
    );
    let expected_label = view_after
        .automation_lanes
        .iter()
        .find(|lane| lane.element_id == lane_id)
        .expect("新泳道的投影")
        .label
        .clone();

    // ---- 4. 投影刷新：**只有**权威改了才动手 ----
    assert_eq!(
        ui.sync_authority().expect("刷新投影"),
        AuthoritySync::Reprojected {
            revision: revision_before + 1
        },
        "权威改过工程 ⇒ 这一次必须真的重投影"
    );

    // ---- 5b. 同案**像素**证据（`ADR-0005` S1）：改动前 / 后各一帧，同一次运行 ----
    //
    // 两帧走**同一条**抓帧路径：`LiveUi::capture` 就是控制面 `ui/screenshot` 内部用的
    // 那一次 `capture_tier1`（判据 1 已把"控制面发出去的原始帧 == 这一帧"逐字节钉住）。
    // 这里**不引入**第二条像素路径，也**不碰 golden**（比的是同一次运行的两帧）。
    let tree_after = ui.tree_snapshot();
    let frame_after = ui.capture().expect("域侧改动后抓帧");
    let diff = frame_diff(&frame_before, &frame_after).unwrap_or_else(|| {
        panic!(
            "域侧真的改了工程、`sync_authority` 也报了 `Reprojected`，但两帧**逐字节相同** \
             —— 界面没有跟着权威动（UI 线程上的重投影注入被绕过？）"
        )
    });
    // 期望区域 = **被改轨道**的泳道带栈（改前 ∪ 改后的投影 `band_y` / `band_height` 并集）。
    //
    // 为什么不是"新泳道那一条带"（**实测读数**，`ADR-0005` S1 原文假定足迹只落在新带内）：
    // 给一条轨道加一条泳道会重新**等分**整条带栈（`automation.rs` 的
    // `band_height = (stride − 2×inset) / lane_count`），于是**既有的**音量泳道带高
    // 52 → 26、它的曲线与角标一起移动。改动的投影足迹因此是整条带栈，不是新带那半条。
    let volume_lane_id = format!("track-{lead_index}-automation-volume-lane");
    let volume_bounds = tree_before
        .find_by_id(&volume_lane_id)
        .and_then(|node| node.bounds)
        .expect("起点：音量泳道必须在运行时树里带几何（`D28` 的唯一注入点）");
    let volume_band_y = view_before
        .automation_lanes
        .iter()
        .find(|lane| lane.element_id == volume_lane_id)
        .expect("起点：音量泳道的投影")
        .band_y;
    let expected_box = projected_lane_stack_box(
        [&view_before, &view_after],
        lead_index,
        volume_band_y,
        volume_bounds,
        LANE_DIFF_TOLERANCE_PX,
    );
    assert!(
        diff.bbox.x >= expected_box.x
            && diff.bbox.y >= expected_box.y
            && diff.bbox.right() <= expected_box.right()
            && diff.bbox.bottom() <= expected_box.bottom(),
        "差异包围盒必须落在**被改轨道**的投影泳道带栈内: diff {:?} ({} 像素) vs 期望 {:?}",
        diff.bbox,
        diff.count,
        expected_box
    );
    assert_eq!(
        diff_pixels_outside(&frame_before, &frame_after, expected_box),
        0,
        "期望区域**之外**必须逐字节相同（整屏重排 / 别处漏改都会在这里变红）: 期望 {:?}",
        expected_box
    );
    let pan_bounds = tree_after
        .find_by_id(&lane_id)
        .and_then(|node| node.bounds)
        .expect("重投影后新泳道必须在运行时树里带几何");
    assert!(
        diff.bbox.x < pan_bounds.right()
            && diff.bbox.right() > pan_bounds.x
            && diff.bbox.y < pan_bounds.bottom()
            && diff.bbox.bottom() > pan_bounds.y,
        "差异必须触及**新泳道**的投影几何 `{pan_bounds:?}`（进了树却没画出来）: diff {:?}",
        diff.bbox
    );
    // ④ 指纹可复现：同一次运行里再抓一帧、再编码一次，都必须逐字节相同。
    let frame_after_again = ui.capture().expect("同一次运行里再抓一帧");
    assert_eq!(
        frame_fingerprint(&frame_after),
        frame_fingerprint(&frame_after_again),
        "同一次运行里两次抓帧的指纹必须相同（像素路径是确定的）"
    );
    let (png_a, _) = encode_with_evidence(&frame_after, DEFAULT_MAX_PNG_BYTES).expect("差分帧编码");
    let (png_b, _) =
        encode_with_evidence(&frame_after_again, DEFAULT_MAX_PNG_BYTES).expect("对照帧编码");
    assert_eq!(png_a, png_b, "同一帧两次编码必须逐字节相同");
    report_line(&format!(
        "[m4-008] 差分像素: 帧指纹 {} → {}; 差异 {} 像素, 包围盒 {:?}; 期望区域 {:?} \
         （被改轨道的投影泳道带栈, 容差 {LANE_DIFF_TOLERANCE_PX}px）; 区域外差异 0; 指纹可复现 {}",
        frame_fingerprint(&frame_before),
        frame_fingerprint(&frame_after),
        diff.count,
        diff.bbox,
        expected_box,
        frame_fingerprint(&frame_after_again)
    ));

    // ---- 5. 界面真的变了：新语义元素在树里，标签 == 权威工程的投影 ----
    assert!(
        tree_after.contains(&lane_id),
        "重投影之后运行时控件树里必须有 `{lane_id}`（界面没跟着权威走）"
    );
    assert!(
        tree_after.len() > nodes_before,
        "控件树节点数必须增加: {nodes_before} → {}",
        tree_after.len()
    );
    let label_after = label_of(&tree_after, &lane_id);
    assert_eq!(
        label_after, expected_label,
        "界面上的标签必须等于**权威工程**的投影（不是装配时那一份）"
    );

    // ---- 6. 只读调用不刷新界面：修订号不动 ⇒ `Unchanged` ----
    let reply = mcp_call(
        mount.address(),
        &bearer,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"yeban_query_project","arguments":{"limit":2}}}"#,
    );
    assert_eq!(reply["result"]["status"], "success", "{reply}");
    assert_eq!(
        authority.apply_revision(),
        revision_before + 1,
        "只读工具不得推进施加修订号"
    );
    assert_eq!(
        ui.sync_authority().expect("刷新投影"),
        AuthoritySync::Unchanged,
        "没有新版本 ⇒ 不得重投影"
    );
    assert_eq!(
        ui.tree_snapshot().len(),
        tree_after.len(),
        "`Unchanged` 必须意味着界面一位没动"
    );

    // ---- 7. 端到端：控制面服务的就是这**一个**更新后的活窗口 ----
    let nodes_after = tree_after.len();
    let mut plane = ui.into_control_plane(Permission::ReadOnly);
    let viewport = plane.viewport();
    let probe = plane
        .plane()
        .probe(
            &ProbeOptions::new(lane_id.clone(), "自动化")
                .expecting_size(viewport.width, viewport.height),
        )
        .unwrap_or_else(|error| panic!("端到端 probe 失败: {error}"));
    assert_eq!(probe.node.id, lane_id);
    assert_eq!(
        probe.node.label, expected_label,
        "`ui/node` 读到的标签必须就是权威工程的投影"
    );
    assert!(
        probe.tree_json.contains(&label_after),
        "`ui/tree` 的线上文本里必须真的有这条泳道的标签"
    );
    // 把第 5b 步的差分帧与**控制面真正发出去的** `ui/screenshot` 帧钉在一起：
    // 未遮罩地再抓一次，指纹必须就是差分用的那一帧（同一条抓帧路径，没有第二个像素来源）。
    let raw = plane
        .plane()
        .probe(
            &ProbeOptions::new(lane_id.clone(), "自动化")
                .expecting_size(viewport.width, viewport.height)
                .without_masking(),
        )
        .unwrap_or_else(|error| panic!("未遮罩帧 probe 失败: {error}"));
    assert_eq!(
        raw.screenshot.fingerprint,
        frame_fingerprint(&frame_after),
        "`ui/screenshot`（未遮罩）产出的原始帧必须就是第 5b 步差分用的那一帧"
    );

    report_line(&format!(
        "[m4-008] 经 MCP 会话改工程 ⇒ 界面投影跟随: 环回 socket 上 `yeban_edit_automation` \
         新建 `{lane_id}` ⇒ 施加修订号 {revision_before}→{}; 控件树 {nodes_before}→{nodes_after} 个节点; \
         标签 = `{label_after}`; 随后的只读调用 = Unchanged（界面不是被查询刷新的）",
        authority.apply_revision(),
    ));
    mount.stop().expect("停机必须成功（有 5 秒上限）");
}

// ---------------------------------------------------------------------------
// 判据 17b：差分**容差**的上界（`ADR-0005` S1 的"容差本身要被判据钉住"）
// ---------------------------------------------------------------------------

/// 把 [`LANE_DIFF_TOLERANCE_PX`] 放大（例如放到全屏）就必须在这里变红；把上界
/// [`LANE_DIFF_TOLERANCE_MAX_PX`] 也放大去迁就它，同样在这里变红。
///
/// 两条断言都走**局部绑定**而不是直接比较两个常量字面量：`clippy::assertions_on_constants`
/// 会把后者当成"恒真的断言"而拒绝（`-D warnings`）。
#[cfg(feature = "in-process-mcp")]
#[test]
fn the_lane_diff_tolerance_stays_bounded() {
    let tolerance = LANE_DIFF_TOLERANCE_PX;
    let max = LANE_DIFF_TOLERANCE_MAX_PX;
    assert!(
        tolerance <= max,
        "差分容差 {tolerance}px 超过上界 {max}px —— 它已经能容下泳道之外的差异（例如整屏重排）"
    );
    assert!(
        max <= 4,
        "容差上界本身不许被放大（现在是 {max}px; 视口是 1920x1080）"
    );
}

// ---------------------------------------------------------------------------
// 判据 18 / 19（`ROAD-M4-008` 选项 (a) **第二片**）：GUI 的写入口落到唯一可变权威上
// ---------------------------------------------------------------------------

/// 工程里**全部**音符身份（`clip_pool` 键序；与 `bridge::ViewState` 的枚举同一来源）。
#[cfg(feature = "in-process-mcp")]
fn project_note_ids(project: &YebanProjectV1) -> Vec<String> {
    let mut ids = Vec::new();
    for entry in project.clip_pool.values() {
        let Some(notes) = entry.content.notes() else {
            continue;
        };
        ids.extend(notes.keys().map(|id| id.to_canonical_string()));
    }
    ids
}

/// 判据 18：**GUI 的撤销与环回会话的撤销改的是同一个 `Domain`**，因此两条写路径在
/// **同一个投影**上汇合。
///
/// ## 这条判据要证的两句话
///
/// 1. GUI 的写入口（`UndoPort`）**不再持有**自己的 `UndoSession`：它由
///    `UndoPort::from_authority` 构造，只握 `ProjectAuthorityHandle`；因此
///    第 3 步的 GUI 撤销必须推进**同一个**权威的施加修订号（`r+1 → r+2`）。
/// 2. 会话侧的写入（`yeban_edit_automation` / `yeban_redo`）与 GUI 侧的写入
///    **互相可见**：同一棵运行时控件树按同一条 `sync_authority` 重投影。
///
/// ## 论证顺序（每一步都用上一步的实读值）
///
/// | 步 | 谁写 | 读回 |
/// | :--- | :--- | :--- |
/// | 1 | 以权威装配真实界面 + 由**同一个**句柄构造端口 | 起点：`track-0-automation-pan-lane` 不在树里；端口显示态 == 权威显示态 |
/// | 2 | **会话侧**：真客户端 + 真令牌发 `yeban_edit_automation`（`TrackPan` 写一个点） | 修订号 `r → r+1`；`sync_authority = Reprojected`；泳道进树 |
/// | 3 | **GUI 侧**：`host::wire_undo` 接的真实 Slint 回调 `undo-step` | 修订号 `r+1 → r+2`（同一个权威！）；端口的 `undone == 1` |
/// | 4 | 投影跟随 | `sync_authority = Reprojected`；泳道离开运行时树 |
/// | 5 | **会话侧**：`yeban_redo` | 修订号 `r+2 → r+3`；同一条泳道回到同一投影 |
///
/// ## 怎么变红（负向实测见工作线报告）
///
/// 把 `UndoPort::from_authority(authority.clone())` 换回 `UndoPort::new(<另一份 UndoSession>)`
/// ⇒ 第 3 步的修订号停在 `r+1`（GUI 写的是**另一个**会话），第 4 步 `Unchanged`、泳道仍在树里。
#[cfg(feature = "in-process-mcp")]
#[test]
fn a_gui_action_and_a_session_action_share_one_projection_through_the_authority() {
    use std::rc::Rc;
    use yeban_app::mcp_mount::{InProcessMcp, SessionSource};
    use yeban_app::undo::UndoPort;

    let project = yeban_model::samples::filled_project();
    let mount = InProcessMcp::start_for_project(
        true,
        project,
        SessionSource::InMemory(std::path::PathBuf::from("sample:m4-008-gui-undo")),
    )
    .expect("挂载决策")
    .expect("运行期开关打开时必须真的挂载");
    let authority = mount.project_authority();
    let bearer = format!("Bearer {}", mount.token().expose());

    // ---- 1. 以权威为唯一工程来源装配真实界面；端口也只握这一个句柄 ----
    let mut ui = build_live_ui_from_authority(&authority, Permission::ReadOnly).expect("装配");
    let port = Rc::new(UndoPort::from_authority(authority.clone()));
    yeban_app::host::wire_undo(ui.ui(), &port);
    yeban_app::host::apply_undo(ui.ui(), &port);

    let view_before =
        ViewState::from_project(&authority.project().expect("权威有活跃工程")).expect("投影");
    let lead_index = view_before.tracks[0].index;
    let lead_id = view_before.tracks[0].id.clone();
    let lane_id = format!("track-{lead_index}-automation-pan-lane");
    let revision_before = authority.apply_revision();
    assert!(
        !ui.tree_snapshot().contains(&lane_id),
        "起点：`{lane_id}` 不该在运行时树里"
    );
    assert_eq!(
        (port.display().undone, port.display().commit_count),
        (
            authority.undo_display().undone,
            authority.undo_display().commit_count
        ),
        "端口的显示态必须逐字段来自**权威**（它没有自己的会话可读）"
    );

    // ---- 2. 会话侧（AI）：真环回 socket 写一个自动化点 ----
    let reply = mcp_call(
        mount.address(),
        &bearer,
        &format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"yeban_edit_automation","arguments":{{"trackId":"{lead_id}","lane":"TrackPan","point":{{"tick":0,"value":0.25,"curve":"Linear"}}}}}}}}"#
        ),
    );
    assert_eq!(reply["result"]["status"], "success", "{reply}");
    assert_eq!(
        authority.apply_revision(),
        revision_before + 1,
        "会话侧写入必须推进权威修订号"
    );
    assert_eq!(
        ui.sync_authority().expect("刷新投影"),
        AuthoritySync::Reprojected {
            revision: revision_before + 1
        }
    );
    assert!(
        ui.tree_snapshot().contains(&lane_id),
        "会话侧写入必须出现在同一投影里"
    );

    // ---- 3. GUI 侧：走 `host::wire_undo` 接的**真实** Slint 回调（撤销一步） ----
    ui.ui().invoke_undo_step();
    assert_eq!(
        authority.apply_revision(),
        revision_before + 2,
        "GUI 的写入必须落在**同一个**权威上（若 GUI 还持自己的会话，这里会停在 r+1）"
    );
    assert_eq!(
        port.display().undone,
        1,
        "GUI 撤销必须真的落到权威会话的游标上"
    );
    let record = port.last_record().expect("GUI 动作日志");
    assert_eq!(record.action, "undo");
    assert!(
        record.changed_project(),
        "GUI 动作日志必须证明工程真的变了: {record:?}"
    );

    // ---- 4. 投影跟随：泳道离开同一棵运行时控件树 ----
    assert_eq!(
        ui.sync_authority().expect("刷新投影"),
        AuthoritySync::Reprojected {
            revision: revision_before + 2
        }
    );
    assert!(
        !ui.tree_snapshot().contains(&lane_id),
        "GUI 撤销之后泳道必须离开运行时树（界面没有跟着权威走）"
    );

    // ---- 5. 会话侧再来一次：重做 ⇒ 同一条泳道回到同一投影 ----
    let reply = mcp_call(
        mount.address(),
        &bearer,
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"yeban_redo","arguments":{"steps":1}}}"#,
    );
    assert_eq!(reply["result"]["status"], "success", "{reply}");
    assert_eq!(authority.apply_revision(), revision_before + 3);
    assert_eq!(
        ui.sync_authority().expect("刷新投影"),
        AuthoritySync::Reprojected {
            revision: revision_before + 3
        }
    );
    assert!(
        ui.tree_snapshot().contains(&lane_id),
        "会话侧重做之后同一条泳道必须回到同一投影"
    );

    report_line(&format!(
        "[m4-008] GUI 写入口落在唯一可变权威上: GUI 回调 `undo-step` ⇒ 施加修订号 {}→{} \
         （与会话侧写入同一个号）; 泳道 `{lane_id}` 随 `sync_authority` 出现 / 消失 / 再出现",
        revision_before,
        authority.apply_revision()
    ));
    mount.stop().expect("停机必须成功（有 5 秒上限）");
}

/// 判据 19：**GUI 的卷帘铅笔编辑进的是同一个权威** —— 会话侧一次 `yeban_undo` 能把它撤掉，
/// 同一棵运行时控件树随之更新。
///
/// ## 论证顺序
///
/// | 步 | 动作 | 读回 |
/// | :--- | :--- | :--- |
/// | 1 | 以权威装配真实界面；`wire_roll_edit` 接上**同一个**句柄构造的端口 | 起点音符身份集 |
/// | 2 | 用**与宿主同一条解析**（`host::pencil_op_for`）找一个真落在片段内的点击位置 | `Some(AddNote)` |
/// | 3 | **GUI 事件源**：`app.slint` 的 `clicked` 回调 → `wire_roll_edit` | 权威工程多**恰好一个**音符；修订号 `r → r+1` |
/// | 4 | `sync_authority` + 运行时树 | 新音符 `note-{ulid}-rect` 在树里 |
/// | 5 | **会话侧**：真环回 socket 发 `yeban_undo` | 音符身份集回到起点；新元素离开树 |
///
/// ## 怎么变红
///
/// 把 `wire_roll_edit` 里的 `port.commit_ops(..)` 摘掉（或让端口改回本地会话）
/// ⇒ 第 3 步权威工程里音符数不变 ⇒ 本判据立刻红。
#[cfg(feature = "in-process-mcp")]
#[test]
fn a_gui_pencil_edit_lands_in_the_same_authority_as_the_session() {
    use std::collections::BTreeSet;
    use std::rc::Rc;
    use yeban_app::host::{PENCIL_TOOL_DIGIT, ROLL_SNAP_GRID_TICKS, pencil_op_for};
    use yeban_app::mcp_mount::{InProcessMcp, SessionSource};
    use yeban_app::undo::UndoPort;

    let project = yeban_model::samples::filled_project();
    let mount = InProcessMcp::start_for_project(
        true,
        project,
        SessionSource::InMemory(std::path::PathBuf::from("sample:m4-008-gui-pencil")),
    )
    .expect("挂载决策")
    .expect("运行期开关打开时必须真的挂载");
    let authority = mount.project_authority();
    let bearer = format!("Bearer {}", mount.token().expose());

    let mut ui = build_live_ui_from_authority(&authority, Permission::ReadOnly).expect("装配");
    let port = Rc::new(UndoPort::from_authority(authority.clone()));
    yeban_app::host::wire_roll_edit(ui.ui(), &port);
    // 矩阵第 3 行 = 铅笔（`Tool::from_digit(2)`）；`wire_roll_edit` 只在这个工具下提交。
    ui.ui().set_active_tool(i32::from(PENCIL_TOOL_DIGIT));

    let project_before = authority.project().expect("权威有活跃工程");
    let scroll = ui.ui().get_roll_scroll_x();
    let y = 100.0_f32;
    let hit_x = (0..240)
        .map(|step| 10.0 * f32::from(u8::try_from(step).unwrap_or(0)))
        .find(|x| {
            pencil_op_for(
                &project_before,
                scroll,
                *x,
                y,
                ROLL_SNAP_GRID_TICKS,
                i32::from(PENCIL_TOOL_DIGIT),
            )
            .is_some()
        })
        .expect("夹具里必须存在一个真的落在片段内的点击位置（否则本判据的对照不成立）");
    let notes_before: BTreeSet<String> = project_note_ids(&project_before).into_iter().collect();
    let tree_before = ui.tree_snapshot();
    let revision_before = authority.apply_revision();

    // ---- GUI 事件源：`app.slint` 的 `clicked` → `wire_roll_edit` → 端口 → 权威 ----
    ui.ui().invoke_clicked(hit_x, y);

    let project_after = authority.project().expect("权威有活跃工程");
    let notes_after: BTreeSet<String> = project_note_ids(&project_after).into_iter().collect();
    let added: Vec<&String> = notes_after.difference(&notes_before).collect();
    assert_eq!(
        added.len(),
        1,
        "GUI 铅笔画一次必须恰好在**权威工程**里加一个音符（起点 {} 个，现在 {} 个）",
        notes_before.len(),
        notes_after.len()
    );
    assert_eq!(
        authority.apply_revision(),
        revision_before + 1,
        "GUI 的提交必须推进**同一个**权威的施加修订号"
    );
    let element_id = format!("note-{}-rect", added[0]);
    assert!(
        !tree_before.contains(&element_id),
        "起点不该已经有 `{element_id}`"
    );

    // ---- 投影跟随：新音符出现在同一棵运行时控件树里 ----
    assert_eq!(
        ui.sync_authority().expect("刷新投影"),
        AuthoritySync::Reprojected {
            revision: revision_before + 1
        }
    );
    assert!(
        ui.tree_snapshot().contains(&element_id),
        "新音符 `{element_id}` 必须出现在活窗口的运行时控件树里"
    );

    // ---- 会话侧（AI）看得见这次 GUI 编辑：一次 `yeban_undo` 把它撤掉 ----
    let reply = mcp_call(
        mount.address(),
        &bearer,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"yeban_undo","arguments":{"steps":1}}}"#,
    );
    assert_eq!(reply["result"]["status"], "success", "{reply}");
    assert_eq!(
        project_note_ids(&authority.project().expect("权威有活跃工程"))
            .into_iter()
            .collect::<BTreeSet<String>>(),
        notes_before,
        "会话侧撤销必须撤掉 GUI 刚加的那个音符（证明它进的是同一个会话）"
    );
    assert_eq!(
        ui.sync_authority().expect("刷新投影"),
        AuthoritySync::Reprojected {
            revision: revision_before + 2
        }
    );
    assert!(
        !ui.tree_snapshot().contains(&element_id),
        "撤销后新音符必须离开同一棵运行时控件树"
    );

    report_line(&format!(
        "[m4-008] GUI 卷帘铅笔经权威提交: 点击 ({hit_x}, {y}) ⇒ 权威工程音符 {}→{} ⇒ \
         树里出现 `{element_id}`; 会话侧 `yeban_undo` ⇒ 音符回到 {} 个、元素离树",
        notes_before.len(),
        notes_after.len(),
        notes_before.len()
    ));
    mount.stop().expect("停机必须成功（有 5 秒上限）");
}

/// `[length]` 属性 → `Vec<f32>`（本文件只读注入面；生产侧的读回在 `host::read_lengths`）。
fn injected_lengths(model: &slint::ModelRc<f32>) -> Vec<f32> {
    use slint::Model as _;
    model.iter().collect()
}

/// `[string]` 属性 → `Vec<String>`（本文件只读注入面；生产侧的读回在 `host::read_strings`）。
fn injected_strings(model: &slint::ModelRc<slint::SharedString>) -> Vec<String> {
    use slint::Model as _;
    model.iter().map(|value| value.to_string()).collect()
}

/// 窗口上那 7 个行几何数组的快照（顺序 = `host::apply_row_geometry` 的写入顺序）。
fn row_geometry_snapshot(window: &yeban_app::ui::MainWindow) -> (Vec<Vec<f32>>, Vec<String>) {
    (
        vec![
            injected_lengths(&window.get_track_ys()),
            injected_lengths(&window.get_track_heights()),
            injected_lengths(&window.get_clip_ys()),
            injected_lengths(&window.get_clip_heights()),
            injected_lengths(&window.get_automation_lane_band_ys()),
            injected_lengths(&window.get_automation_lane_band_heights()),
        ],
        injected_strings(&window.get_automation_path_commands()),
    )
}

/// **投影**里同样那 7 个字段的快照（本判据的独立基准；不是"另一个注入写者"）。
fn projection_geometry_snapshot(view: &ViewState) -> (Vec<Vec<f32>>, Vec<String>) {
    (
        vec![
            view.track_ys(),
            view.track_heights(),
            view.clip_ys(),
            view.clip_heights(),
            view.automation_lane_band_ys(),
            view.automation_lane_band_heights(),
        ],
        view.automation_path_commands(),
    )
}

/// 轨道头 `track-0-header` 在**运行时树**里的几何原点与行高（窗口逻辑坐标）。
///
/// 判据用它取注入坐标，而不是写死像素 —— 侧栏宽度 / 顶栏高度一改，坐标跟着走。
fn header_box(plane: &mut LiveControlPlane, id: i64) -> (f64, f64, f64) {
    let bounds = header_bounds(plane, id);
    (
        bounds["x"].as_f64().expect("x"),
        bounds["y"].as_f64().expect("y"),
        bounds["height"].as_f64().expect("height"),
    )
}

/// 同 [`header_box`]，但给宽度（日志里用）。
fn header_box_width(plane: &mut LiveControlPlane, id: i64) -> f64 {
    header_bounds(plane, id)["width"].as_f64().expect("width")
}

/// `ui/node` 读 `track-0-header` 的 `bounds` —— 没有它（元素不在树里 / 没有几何）就**响亮失败**。
fn header_bounds(plane: &mut LiveControlPlane, id: i64) -> serde_json::Value {
    let node = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        id,
        "ui/node",
        Some(serde_json::json!({"elementId": "track-0-header"})),
    ));
    assert!(!node.is_error(), "轨道头必须在运行时树里: {node:?}");
    let bounds = node.result.expect("有 result")["node"]["bounds"].clone();
    assert!(!bounds.is_null(), "轨道头必须有几何包围盒");
    bounds
}

/// `tests/undo_wiring_ui.rs` 用的同一个时间戳夹具（会话打开时刻）。
const TRACK_HEIGHT_DRAG_NOW: u64 = 1_760_000_000_000;

/// 拖拽手势判据的装配：真实界面 + **同一份工程**的撤销端口 + 生产形态的两条接线。
///
/// 返回 `(window, port, plane)`：
/// - `window` 是活窗口的强引用（`clone_strong` —— 与执行面持有的是**同一个**组件实例），
///   判据用它读注入面（`track-ys` / `track-heights` / `track-height-*`）与直接驱动回调；
/// - `plane` 是同一个窗口上的控制面（注入真实指针 / 键盘事件）。
///
/// 两条接线都是**产品路径上的既有调用**（`src/main.rs` 里逐字相同）：
/// `host::wire_track_height_drag`（手势本体）与 `host::wire_keys`（`Escape` 的取消路径
/// 需要用端口把几何重新投影回起点）。`build_live_ui` 内部已经用 `None` 接过一次
/// `wire_keys`；这里按**生产形态**（`Some(port)`）再接一次 —— 两次接的是同一个
/// `InputContext`，后一次覆盖前一次（Slint 的回调 setter 是覆盖语义）。
fn assemble_track_height_drag() -> (
    yeban_app::ui::MainWindow,
    std::rc::Rc<yeban_app::undo::UndoPort>,
    LiveControlPlane,
) {
    use std::rc::Rc;

    use yeban_app::undo::{UndoPort, UndoSession};

    let project = yeban_model::samples::filled_project();
    let ui = build_live_ui(&project, Permission::Interactive).expect("装配");
    let window = slint::ComponentHandle::clone_strong(ui.ui());
    // 端口持有的是**同一份**工程（`filled_project()` 的身份是固定夹具 ⇒ 身份逐位相同）。
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<判据>", "yeban-app", project, TRACK_HEIGHT_DRAG_NOW).expect("打开"),
    ));
    yeban_app::host::wire_track_height_drag(&window, &port);
    yeban_app::host::wire_keys(&window, ui.input_context(), Some(Rc::clone(&port)));
    let plane = ui.into_control_plane(Permission::Interactive);
    (window, port, plane)
}

/// 判据（`ADR-0004` **S1**）：轨道高度的**视图态**真的走到了注入的行几何上 ——
/// 即"每轨高度 + 全局乘子"这条能力**在真实窗口上有消费者**，而不是一个没人调用的参数。
///
/// 链路（每一步都是既有实现，本判据不新增测试专用路径）：
///
/// ```text
/// MainWindow 的 track-height-* 三个属性        ← host::set_track_height_override / _percent
///   └─ host::track_height_layout（唯一读点）
///        └─ ViewState::from_project_with_layout（唯一的 clamp：base × percent / 100，再夹紧）
///             └─ host::apply_view → track-ys / track-heights
///                  └─ 活窗口上的 ModelRc（本判据直接读）
/// ```
///
/// 驱动入口是**既有的**重投影点 `LiveUi::apply_project`（不是为判据新开的口子）：
/// 它读回窗口上的布局再投影 —— 若不读回来，"设置行高之后再重投影"会把高度静默清零，
/// 这条判据就会红。
///
/// 行高的**取值范围**是 `[16, 320]`（`MIN/MAX_TRACK_HEIGHT_PX`，`ADR-0004` Q2 点名常量、
/// 未给数字 ⇒ 本仓登记的工程选择；理由见常量文档）。这里取的 120/150% 落在区间内，
/// 因此断言的是"公式"，不是夹紧。
#[test]
fn track_height_view_state_reaches_the_injected_row_geometry() {
    let project = yeban_model::samples::demo_project();
    let mut ui = build_live_ui(&project, Permission::ReadOnly).expect("装配");
    // 第 0 条非主总线轨道的**身份**（`track-{i}-*` 的 `i` 就是投影下标 —— 语义 ID 用它）。
    let view = yeban_app::bridge::ViewState::from_project(&project).expect("投影");
    let first_id = view.tracks[0].id.clone();

    // 默认：注入的行矩形高 = 56 − 2（S0 的 `height: 54px`），一位没变。
    let before = injected_lengths(&ui.ui().get_track_heights());
    assert!(before.len() >= 2, "演示工程必须有多条轨道");
    assert!(
        before.iter().all(|height| *height == 54.0),
        "默认布局的注入几何必须是 54.0（S0 的字面量）: {before:?}"
    );

    // 设置"第 0 轨 120px + 全局 150%"，然后走**既有**重投影入口。
    assert!(
        yeban_app::host::set_track_height_override(ui.ui(), &first_id, 120),
        "第一次设置必须报告布局变了"
    );
    assert!(
        !yeban_app::host::set_track_height_override(ui.ui(), &first_id, 120),
        "同一个值 ⇒ 没变（幂等：调用方不必重投影）"
    );
    assert!(
        yeban_app::host::set_track_height_percent(ui.ui(), 150),
        "乘子必须写进视图态"
    );
    ui.apply_project(&project).expect("重投影");

    let ys = injected_lengths(&ui.ui().get_track_ys());
    let heights = injected_lengths(&ui.ui().get_track_heights());
    // 120 × 150% = 180（行矩形高 178）；未覆盖的行是默认基准 56 × 150% = 84（行矩形高 82）。
    assert_eq!(heights[0], 178.0, "第 0 轨：120 × 150% − 2");
    assert_eq!(heights[1], 82.0, "第 1 轨：默认 56 × 150% − 2");
    assert_eq!(ys[0], 42.0, "第 0 行的顶沿仍是 S0 的 42px");
    assert_eq!(ys[1], 42.0 + 180.0, "前缀和必须用**逐行**行高");

    // 复位（`px == 0` = 删除覆盖）之后几何回到默认 —— 视图态的"回到默认"也是可达的。
    assert!(yeban_app::host::set_track_height_override(
        ui.ui(),
        &first_id,
        0
    ));
    assert!(yeban_app::host::set_track_height_percent(ui.ui(), 100));
    ui.apply_project(&project).expect("重投影");
    let after = injected_lengths(&ui.ui().get_track_heights());
    assert_eq!(after, before, "复位之后必须逐位回到默认几何");

    report_line(&format!(
        "[adr-0004-s1] 行高视图态: 第 0 轨 120px × 150% ⇒ 注入 {:.0}px / y {:.0}px; \
         未覆盖轨 {:.0}px; 复位后逐位回到默认 {:?}",
        heights[0], ys[1], heights[1], after
    ));
}

/// 判据（`ADR-0004` **S1 的纵向入口**）：轨道头**拖拽手势**真的改行高，而且收尾干净。
///
/// ## 为什么这条判据用 `ui/dispatch_pointer_*` 而不是新加一个 `ui/set_track_height`
///
/// §12.4 的两个注入方法**已经**在方法表里（`ui/dispatch_pointer_*`），而它们驱动的是与用户
/// **一模一样**的事件源：真实指针事件 → Slint 命中测试 → `.slint` 的轨道头 `TouchArea` →
/// `host::wire_track_height_drag`。于是这条判据证明的是"**手势真的能用**"，
/// 比"存在一个把数字写进去的设置器"更强（后者证明不了 `.slint` 那一半接上了）。
///
/// ## 三条会变红的注入（每条都实测过）
///
/// | 注入 | 位置 | 现象 |
/// | :--- | :--- | :--- |
/// | 摘掉 `.slint` 里的 `track-height-grab/drag/release` 转发 | `arrangement_view.slint` | 指针注入之后行几何一位不变（本判据第一段红） |
/// | 摘掉 `host::wire_track_height_drag` 里的 `set_track_height_override` | `src/host.rs` | 同上 |
/// | 摘掉 `end_track_height_drag` 里的 `set_track_height_drag_active(false)` | `src/host.rs` | 收尾断言红（粘住的拖拽态） |
#[test]
fn the_track_header_drag_gesture_changes_the_row_geometry_and_ends_cleanly() {
    use serde_json::json;
    use yeban_ui_mcp::methods::{METHOD_DISPATCH_POINTER_DOWN, METHOD_DISPATCH_POINTER_MOVE};

    let (window, _port, mut plane) = assemble_track_height_drag();

    // ---- 起点：默认布局（无覆盖、乘子 100）⇒ S0 的几何一位没变 ----
    let before_heights = injected_lengths(&window.get_track_heights());
    let before_ys = injected_lengths(&window.get_track_ys());
    assert!(before_heights.len() >= 2, "夹具必须有多条轨道");
    assert!(
        before_heights.iter().all(|px| *px == 54.0),
        "默认布局下注入的行矩形高必须是 54.0（S0 的字面量）: {before_heights:?}"
    );
    assert_eq!(before_ys[0], 42.0, "第 0 行的顶沿是 S0 的 42px");
    assert!(
        injected_strings(&window.get_track_height_override_ids()).is_empty(),
        "起点不该有任何每轨覆盖"
    );
    assert_eq!(window.get_track_height_percent(), 100);
    assert!(
        !window.get_track_height_drag_active(),
        "起点不该有进行中的手势"
    );

    // ---- 轨道头的**真实几何**（由控制面自己的树读出来，不写死坐标）----
    let (header_x, header_y, header_h) = header_box(&mut plane, 201);
    let header_w = header_box_width(&mut plane, 201);
    assert_eq!(header_h, 54.0, "默认行高的包头高就是注入的 54px");
    // 取包头**靠右**的一点（躲开左上角的静音 / 独奏按钮与色标），纵向取行中。
    let grab_x = header_x + 150.0;
    let grab_y = header_y + header_h / 2.0;

    // ---- 按下 + 向上拖 40px ----
    let down = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        202,
        METHOD_DISPATCH_POINTER_DOWN,
        Some(json!({
            "elementId": "track-0-header",
            "xOffset": 150.0,
            "yOffset": header_h / 2.0,
            "button": "left"
        })),
    ));
    assert!(!down.is_error(), "按下必须落到真实窗口: {down:?}");
    assert!(
        window.get_track_height_drag_active(),
        "按下之后手势必须处于进行中（`.slint` 的 down 转发接上了）"
    );

    let moved = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        203,
        METHOD_DISPATCH_POINTER_MOVE,
        Some(json!({"x": grab_x, "y": grab_y - 40.0})),
    ));
    assert!(!moved.is_error(), "拖动必须落到真实窗口: {moved:?}");

    // 拖动过程中：视图态里已经写进了**基准** 56 + 40 = 96（整数像素）。
    let track_id = injected_strings(&window.get_track_ids())[0].clone();
    let ids = injected_strings(&window.get_track_height_override_ids());
    assert_eq!(ids, vec![track_id.clone()], "覆盖表按**身份**键控");
    let pxs = injected_lengths(&window.get_track_height_override_pxs());
    assert_eq!(pxs, vec![96.0], "基准高 = 起点 56 + 位移 40（乘子 100）");
    let dragging_heights = injected_lengths(&window.get_track_heights());
    assert_eq!(
        dragging_heights[0], 94.0,
        "第 0 行的矩形高 = 96 − 2（间隙常量不变）"
    );
    assert_eq!(
        dragging_heights[1], 54.0,
        "别的轨道不受影响（手势只作用于按下那一行）"
    );
    assert_eq!(
        injected_lengths(&window.get_track_ys())[1],
        42.0 + 96.0,
        "前缀和必须用**新的**行高：第 1 行的顶沿随之下移"
    );

    // ---- 松手：手势必须收尾（这是"不留粘住的拖拽态"的第一条路径）----
    let up = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        204,
        "ui/dispatch_pointer_up",
        Some(json!({"button": "left"})),
    ));
    assert!(!up.is_error(), "松手必须落到真实窗口: {up:?}");
    assert!(
        !window.get_track_height_drag_active(),
        "松手之后手势必须结束（否则下一次孤立的 move 会继续改高度）"
    );
    assert_eq!(
        window.get_track_height_drag_track(),
        "",
        "收尾必须把「拖的是哪一行」也清掉"
    );
    // 松手之后几何**保留**（松手 = 这一拖算数）。
    let after = injected_lengths(&window.get_track_heights());
    assert_eq!(after[0], 94.0, "松手不该回退高度");

    report_line(&format!(
        "[adr-0004-s1-drag] 轨道头拖拽: 包头 {:.0}x{:.0} @ ({:.0},{:.0}) 按住向上拖 40px ⇒ \
         基准 {}px（授权身份 {}…）、注入矩形高 {:.0}px、第 1 行顶沿 {:.0}px；松手后 active={}",
        header_w,
        header_h,
        header_x,
        header_y,
        pxs[0],
        &track_id[..track_id.len().min(8)],
        after[0],
        injected_lengths(&window.get_track_ys())[1],
        window.get_track_height_drag_active()
    ));
}

/// 判据（`ADR-0004` S1 的纵向入口）：`Escape` **取消**这一拖（写回按下时的高度）并收尾。
///
/// 三条收尾路径里，`Escape` 是唯一"取消"语义的那条（松手与指针离开窗口都是"算数"）。
/// 它与另外两条共用 `host::end_track_height_drag`（手势状态的唯一清零点），
/// 但多一步"把基准高写回起点 + 重投影" —— 少了那一步，用户看到的是"按了 Esc 没反应"。
///
/// ## 怎么变红
///
/// 把 `apply_action` 里的 `Action::Cancel => cancel_track_height_drag(ui, undo)` 摘掉：
/// `Escape` 回到"未实现 ⇒ 放行"的老行为，行高**留在 116px** ⇒ 本判据红。
#[test]
fn escape_cancels_the_track_height_drag_and_restores_the_starting_height() {
    use serde_json::json;
    use yeban_ui_mcp::methods::{METHOD_DISPATCH_KEY_PRESS, METHOD_DISPATCH_POINTER_DOWN};

    let (window, port, mut plane) = assemble_track_height_drag();
    let grab = |plane: &mut LiveControlPlane, id: i64, x_offset: f64, y_offset: f64| {
        plane.plane().try_line(&yeban_ui_mcp::live::request_line(
            id,
            METHOD_DISPATCH_POINTER_DOWN,
            Some(json!({
                "elementId": "track-0-header",
                "xOffset": x_offset,
                "yOffset": y_offset,
                "button": "left"
            })),
        ))
    };

    // ---- 坐标来自**运行时树**（不写死像素：布局一改判据跟着走）----
    let (header_x, header_y, header_h) = header_box(&mut plane, 210);
    let grab_x = header_x + 150.0;
    let grab_y = header_y + header_h / 2.0;
    assert_eq!(header_h, 54.0, "默认行高的包头高就是注入的 54px");

    // ---- 第一次手势：拖到 96px 并**松手**（这一拖算数）----
    let down = grab(&mut plane, 211, 150.0, header_h / 2.0);
    assert!(!down.is_error(), "{down:?}");
    let moved = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        212,
        "ui/dispatch_pointer_move",
        Some(json!({"x": grab_x, "y": grab_y - 40.0})),
    ));
    assert!(!moved.is_error(), "{moved:?}");
    let up = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        213,
        "ui/dispatch_pointer_up",
        Some(json!({"button": "left"})),
    ));
    assert!(!up.is_error(), "{up:?}");
    let committed = injected_lengths(&window.get_track_heights());
    assert_eq!(committed[0], 94.0, "第一次拖到 96px（矩形高 94）");
    assert!(!window.get_track_height_drag_active());

    // ---- 第二次手势：从 96px 再拖 20px（到 116），然后按 `Escape` ----
    let down2 = grab(&mut plane, 214, 150.0, header_h / 2.0);
    assert!(!down2.is_error(), "{down2:?}");
    let moved2 = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        215,
        "ui/dispatch_pointer_move",
        Some(json!({"x": grab_x, "y": grab_y - 20.0})),
    ));
    assert!(!moved2.is_error(), "{moved2:?}");
    assert_eq!(
        injected_lengths(&window.get_track_heights())[0],
        114.0,
        "第二次拖到 116px（矩形高 114）"
    );
    assert!(
        window.get_track_height_drag_active(),
        "`Escape` 之前手势必须还在进行中（否则没有可取消的对象）"
    );

    let pressed = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        216,
        METHOD_DISPATCH_KEY_PRESS,
        Some(json!({"keyCode": "Escape"})),
    ));
    assert!(!pressed.is_error(), "`Escape` 注入必须成功: {pressed:?}");

    // ---- 取消之后：几何回到**第一次松手时**的高度，且手势收尾 ----
    assert!(
        !window.get_track_height_drag_active(),
        "`Escape` 必须收尾（手势状态清零）"
    );
    assert_eq!(
        injected_lengths(&window.get_track_heights())[0],
        94.0,
        "`Escape` 取消这一拖 ⇒ 回到按下时的 96px（矩形高 94）"
    );
    assert_eq!(
        injected_lengths(&window.get_track_ys())[1],
        42.0 + 96.0,
        "取消之后前缀和也必须回到起点（`track-ys` 是**画布**坐标，行 0 恒为 S0 的 42px）"
    );
    assert_eq!(
        injected_lengths(&window.get_track_height_override_pxs()),
        vec![96.0],
        "取消写回的是**起点**基准高，不是默认 56"
    );

    // ---- 没有手势在手、**且时光机关着**时，`Escape` 照旧不被消费
    //（行为与接线之前逐位相同；时光机关着时的收尾项是零个）----
    assert!(
        !port.undo_tree_open(),
        "本步的前提是时光机**关着** —— 这是「没有可收尾的对象」这句断言成立的条件"
    );
    assert!(
        !window.invoke_key_action("\u{1b}".into(), false, false, false, false),
        "没有手势在手、时光机关着时 `Escape` 必须如实放行（而不是假装处理了）"
    );

    report_line(&format!(
        "[adr-0004-s1-drag] `Escape` 取消: 第一次松手后 {:.0}px（基准 96）→ 第二次拖到 {:.0}px \
         ⇒ `Escape` 之后回到 {:.0}px、active={}；无手势时 `Escape` 被消费 = {}",
        committed[0],
        114.0,
        injected_lengths(&window.get_track_heights())[0],
        window.get_track_height_drag_active(),
        window.invoke_key_action("\u{1b}".into(), false, false, false, false)
    ));
}

/// 判据（`ADR-0004` S1 的纵向入口）：**孤立的拖拽事件一位都改不动几何** ——
/// 这是"粘住的拖拽态"那一类缺陷的机械防线。
///
/// 四种形态（每一种都对应一个真实的失败模式）：
///
/// | # | 形态 | 真实的失败模式 |
/// | :--- | :--- | :--- |
/// | ① | 没有按下的 `move` | 松在窗口外 / 事件丢失之后，下一次移动继续改高度 |
/// | ② | 没有手势的 `release` | 收尾代码把"没有手势"当成"手势结束"并写坏状态 |
/// | ③ | 下标越界的 `grab` | 索引越界时猜第一条轨道（高度被改到别的轨上） |
/// | ④ | 手势进行中、**另一个下标**发来的 `move` | 多指 / 事件串扰把高度改到别的轨上 |
#[test]
fn a_stray_drag_event_without_a_grab_never_touches_the_row_geometry() {
    let (window, _port, _plane) = assemble_track_height_drag();
    let before = injected_lengths(&window.get_track_heights());

    // ① 没有按下的 move。
    window.invoke_track_height_drag(0, 10.0);
    assert_eq!(
        injected_lengths(&window.get_track_heights()),
        before,
        "没有按下的 move 不得改几何"
    );
    assert!(
        !window.get_track_height_drag_active(),
        "孤立的 move 不得把手势点亮"
    );

    // ② 没有手势的 release。
    window.invoke_track_height_release(0);
    assert_eq!(injected_lengths(&window.get_track_heights()), before);
    assert!(injected_strings(&window.get_track_height_override_ids()).is_empty());

    // ③ 下标越界的 grab：手势**不开始**。
    window.invoke_track_height_grab(9_999, 0.0);
    assert!(
        !window.get_track_height_drag_active(),
        "下标越界时手势不得开始"
    );
    window.invoke_track_height_drag(9_999, -400.0);
    assert_eq!(
        injected_lengths(&window.get_track_heights()),
        before,
        "下标越界的 move 不得改任何一条轨道"
    );

    // ④ 手势进行中、另一个下标发来的 move。
    window.invoke_track_height_grab(0, 200.0);
    assert!(window.get_track_height_drag_active());
    assert_eq!(
        window.get_track_height_drag_start_px(),
        56,
        "起点基准 = 默认 56"
    );
    window.invoke_track_height_drag(1, 0.0);
    assert_eq!(
        injected_lengths(&window.get_track_heights()),
        before,
        "手势进行中，别的行发来的 move 不得改几何"
    );
    assert!(
        injected_strings(&window.get_track_height_override_ids()).is_empty(),
        "别的行发来的 move 不得写进覆盖表"
    );
    // 收尾（并证明收尾之后仍然改不动）。
    window.invoke_track_height_release(0);
    assert!(!window.get_track_height_drag_active());
    window.invoke_track_height_drag(0, -400.0);
    assert_eq!(
        injected_lengths(&window.get_track_heights()),
        before,
        "收尾之后的 move 不得改几何"
    );

    report_line(
        "[adr-0004-s1-drag] 孤立的拖拽事件（move/release/越界 grab/串扰 move 共 5 次）\
         之后注入几何逐位等于起点 ⇒ 没有粘住的拖拽态",
    );
}

/// 判据（`ADR-0004` S1 的**控制面入口**）：`ui/set_track_height` 真的改行高，且**两个观测面**
/// 都能回读出来。
///
/// ## 为什么这条判据有两个观测面
///
/// | # | 观测面 | 读法 | 证明的是 |
/// | :--- | :--- | :--- | :--- |
/// | 1 | 注入的投影数组 | `MainWindow.get_track_heights()` | 视图态 → 投影 → 界面的那条链真的走到了 |
/// | 2 | 控制面**自己的**运行时树 | `ui/node` 的 `bounds.height` | 执行面在改完之后**重抓过树**（否则 `ui/tree` 会一直卖旧几何给 AI） |
///
/// ## 顺带钉住的口径（每条一行断言）
///
/// - `heightPx: 0` = **取消覆盖**（回到默认基准），与 `host::set_track_height_override` 逐字相同；
/// - 幂等：同一个值再来一次 ⇒ 回执里 `changed: false`，几何一位不动；
/// - `dryRun: true` ⇒ 预览给出"将要写进去的基准高"，而**状态一位不变**；
/// - **非法参数用既有错误码**（D25）：负数是 `-32602 INVALID_PARAMS`、不是轨道头是 `-32006`；
/// - 权限层：`ui/methods` 里它的 `requiredScope` 是 `ui:inject`（`Interactive` 层）。
#[test]
fn the_set_track_height_method_reaches_the_injected_row_geometry() {
    use serde_json::json;
    use yeban_ui_mcp::methods::METHOD_SET_TRACK_HEIGHT;

    let (window, _port, mut plane) = assemble_track_height_drag();
    let track_id = injected_strings(&window.get_track_ids())[0].clone();

    // ---- 起点：默认几何 ----
    assert_eq!(injected_lengths(&window.get_track_heights())[0], 54.0);
    assert_eq!(injected_lengths(&window.get_track_ys())[1], 42.0 + 56.0);

    // ---- ① 设 96px ⇒ 注入的几何 + 控制面自己的树几何都必须跟着走 ----
    let set = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        301,
        METHOD_SET_TRACK_HEIGHT,
        Some(json!({"elementId": "track-0-header", "heightPx": 96})),
    ));
    assert!(
        !set.is_error(),
        "`{METHOD_SET_TRACK_HEIGHT}` 必须成功: {set:?}"
    );
    let result = set.result.expect("有 result");
    assert_eq!(result["accepted"], json!(true));
    assert_eq!(result["operation"], json!("set_track_height"));
    assert_eq!(result["heightPx"], json!(96));
    let report = &result["report"];
    assert_eq!(report["operation"], json!("set_track_height"));
    assert_eq!(report["changed"], json!(true));
    assert_eq!(report["trackId"], json!(track_id));
    assert_eq!(report["drawnPx"], json!(94.0));

    assert_eq!(
        injected_lengths(&window.get_track_heights())[0],
        94.0,
        "观测面 1：注入的矩形高 = 基准 96 − 2（间隙）"
    );
    assert_eq!(
        injected_lengths(&window.get_track_ys())[1],
        42.0 + 96.0,
        "观测面 1：前缀和用的是新的行槽高"
    );
    assert_eq!(
        injected_lengths(&window.get_track_height_override_pxs()),
        vec![96.0]
    );
    let (_, _, header_h) = header_box(&mut plane, 302);
    assert_eq!(
        header_h, 94.0,
        "观测面 2：控制面的运行时树必须看到新的包头高（没重抓树就会卖旧几何）"
    );

    // ---- ② 幂等：同一个值再来一次 ⇒ `changed: false`，几何一位不动 ----
    let again = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        303,
        METHOD_SET_TRACK_HEIGHT,
        Some(json!({"elementId": "track-0-header", "heightPx": 96})),
    ));
    assert!(!again.is_error(), "{again:?}");
    assert_eq!(
        again.result.expect("有 result")["report"]["changed"],
        json!(false),
        "同一个值必须如实报 `changed: false`"
    );
    assert_eq!(injected_lengths(&window.get_track_heights())[0], 94.0);

    // ---- ③ dryRun：预览报出将要写的值，状态一位不变 ----
    let dry = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        304,
        METHOD_SET_TRACK_HEIGHT,
        Some(json!({"elementId": "track-0-header", "heightPx": 200, "dryRun": true})),
    ));
    assert!(!dry.is_error(), "{dry:?}");
    let dry_result = dry.result.expect("有 result");
    assert_eq!(dry_result["preview"]["effect"]["requestedPx"], json!(200));
    assert_eq!(dry_result["preview"]["effect"]["currentBasePx"], json!(96));
    assert_eq!(
        injected_lengths(&window.get_track_heights())[0],
        94.0,
        "`dryRun` 之后几何一位不变"
    );
    assert_eq!(
        injected_lengths(&window.get_track_height_override_pxs()),
        vec![96.0],
        "`dryRun` 不得改视图态"
    );

    // ---- ④ 取消覆盖：`heightPx: 0` ⇒ 回默认基准 ----
    let cleared = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        305,
        METHOD_SET_TRACK_HEIGHT,
        Some(json!({"elementId": "track-0-header", "heightPx": 0})),
    ));
    assert!(!cleared.is_error(), "{cleared:?}");
    assert!(
        injected_strings(&window.get_track_height_override_ids()).is_empty(),
        "`0` 必须**取消**这条覆盖（与 setter 的契约逐字相同）"
    );
    assert_eq!(injected_lengths(&window.get_track_heights())[0], 54.0);
    assert_eq!(injected_lengths(&window.get_track_ys())[1], 42.0 + 56.0);

    // ---- ⑤ 非法参数用**既有**错误码（D25：不发明新码）----
    let negative = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        306,
        METHOD_SET_TRACK_HEIGHT,
        Some(json!({"elementId": "track-0-header", "heightPx": -1})),
    ));
    assert_eq!(
        negative.code,
        Some(-32602),
        "负数是**参数**错误（既有 `INVALID_PARAMS`）: {negative:?}"
    );
    let not_a_header = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        307,
        METHOD_SET_TRACK_HEIGHT,
        Some(json!({"elementId": "transport-play-button", "heightPx": 96})),
    ));
    assert_eq!(
        not_a_header.code,
        Some(-32006),
        "不是轨道头 ⇒ 既有 `ELEMENT_NOT_FOUND`: {not_a_header:?}"
    );
    assert!(
        injected_strings(&window.get_track_height_override_ids()).is_empty(),
        "两次被拒的调用都不得留下状态"
    );

    // ---- ⑥ 权限层：`ui/methods` 如实报 `ui:inject` ----
    let catalogue = plane.plane().methods().expect("ui/methods");
    let listed = catalogue["methods"].as_array().expect("数组");
    let entry = listed
        .iter()
        .find(|entry| entry["name"] == json!(METHOD_SET_TRACK_HEIGHT))
        .expect("方法表里必须有它");
    assert_eq!(entry["requiredScope"], json!("ui:inject"));
    assert_eq!(entry["dryRunSupported"], json!(true));

    report_line(&format!(
        "[adr-0004-s1-method] `{METHOD_SET_TRACK_HEIGHT}`: 96px ⇒ 注入矩形高 94.0 / 第 1 行顶沿 138.0 / \
         控制面树的包头高 94.0（两个观测面）；幂等 changed=false；dryRun 预览 requestedPx=200 且状态不变；\
         `0` 取消覆盖回 54.0；负值 ⇒ -32602，非轨道头 ⇒ -32006"
    ));
}

/// 判据（`ADR-0004` S0/S1 的**接线纪律**）：两条注入路径写进界面的行几何**逐位等于投影**。
///
/// ## 判据用的是什么"尺"（这是本判据有牙的关键）
///
/// 它拿 **`ViewState`（投影本身）** 当独立基准，而不是拿"另一条注入路径"当基准。
/// 理由是一次真测得来的：我最初写成"窄注入 == 完整注入"，注入验证（从
/// `host::apply_row_geometry` 里删掉 `set_clip_ys`）**没有让它变红** —— 因为
/// `apply_view` 调用的就是 `apply_row_geometry`，两条路一起漏 ⇒ 两边相等。
/// 那就是一条**空转的判据**。
///
/// 现在两条路各自与投影对账：
///
/// | 注入路径 | 谁用 | 断言 |
/// | :--- | :--- | :--- |
/// | `host::apply_row_geometry`（窄，7 个数组） | 拖拽手势的每一次 move | 窗口上的 7 个数组 == `ViewState` 的对应字段 |
/// | `host::apply_view`（完整） | 换工程 / 撤销 / 打开工程 | 同上 |
///
/// 于是"漏掉任何一个行几何数组"（无论在哪条路里）都会红。
///
/// ## 怎么变红（实测）
///
/// 从 `apply_row_geometry` 里删掉 `set_clip_ys` / `set_clip_heights` ⇒ 窗口上的
/// `clip-ys` 停在旧值、投影给了新值 ⇒ 本判据红。
#[test]
fn the_row_geometry_writers_carry_the_projections_geometry_bit_for_bit() {
    let (window, _port, _plane) = assemble_track_height_drag();
    let project = yeban_model::samples::filled_project();
    let default_view = ViewState::from_project_with_layout(
        &project,
        &yeban_app::host::track_height_layout(&window),
    )
    .expect("投影");
    // 起点（默认几何）：先让窗口处于默认几何，作为"必须真的变过"的对照。
    yeban_app::host::apply_row_geometry(&window, &default_view);
    let default_snapshot = row_geometry_snapshot(&window);
    assert_eq!(
        default_snapshot,
        projection_geometry_snapshot(&default_view),
        "默认几何也必须逐位等于投影"
    );

    // 造一份**非默认**布局（第 0 轨 137px、乘子 150%）——137 × 150% = 205（夹紧区间内）。
    assert!(yeban_app::host::set_track_height_override(
        &window,
        &default_view.tracks[0].id,
        137
    ));
    assert!(yeban_app::host::set_track_height_percent(&window, 150));
    let view = ViewState::from_project_with_layout(
        &project,
        &yeban_app::host::track_height_layout(&window),
    )
    .expect("投影");
    let oracle = projection_geometry_snapshot(&view);
    assert_ne!(
        oracle, default_snapshot,
        "夹具必须真的画出非默认几何（否则本判据是空转）"
    );

    // ① 窄注入（拖拽手势那条路）。
    yeban_app::host::apply_row_geometry(&window, &view);
    assert_eq!(
        row_geometry_snapshot(&window),
        oracle,
        "窄注入（`apply_row_geometry`）的 7 个数组必须逐位等于投影 —— \
         漏掉任何一个行几何数组都会在这里红"
    );

    // ② 完整注入（换工程那条路）：先把窗口推回默认，再完整注入一次。
    yeban_app::host::apply_row_geometry(&window, &default_view);
    assert_eq!(row_geometry_snapshot(&window), default_snapshot);
    yeban_app::host::apply_view(&window, &view, 1920.0, 0.0);
    assert_eq!(
        row_geometry_snapshot(&window),
        oracle,
        "完整注入（`apply_view`）的 7 个数组必须逐位等于投影"
    );

    let (lengths, commands) = oracle;
    report_line(&format!(
        "[adr-0004-s1-drag] 行几何 ↔ 投影逐位对账: 窄注入与完整注入各自等于投影 \
         （track-ys/heights {}/{}, clip-ys/heights {}/{}, automation-band-ys/heights {}/{}, \
         path-commands {}）；第 0 轨 137px × 150% ⇒ 有效 205px",
        lengths[0].len(),
        lengths[1].len(),
        lengths[2].len(),
        lengths[3].len(),
        lengths[4].len(),
        lengths[5].len(),
        commands.len()
    ));
}

/// 判据（`ADR-0004` S0/S1）：**默认外观一位不变** —— 没有人碰过高度时，注入的行几何逐字段
/// 等于 S0 的字面量，而且拖拽手势的四个状态位全是默认值。
///
/// 这条是"像素影响"的可判据那一半（另一half 是渲染帧 sha256 的 A/B，见交付报告）：
/// 新的 `TouchArea` **不画任何东西**，新的属性与回调**不进像素**，因此默认帧的输入
/// （`track-ys` / `track-heights` / `clip-ys` / `clip-heights` / 自动化带与折线）
/// 必须与加手势之前**逐位相同**。
#[test]
fn the_default_row_geometry_is_bit_for_bit_the_s0_literals() {
    let (window, _port, _plane) = assemble_track_height_drag();

    // ① 视图态：没有覆盖、乘子 100、没有进行中的手势。
    assert!(
        injected_strings(&window.get_track_height_override_ids()).is_empty(),
        "默认装配不该有每轨覆盖"
    );
    assert!(
        injected_lengths(&window.get_track_height_override_pxs()).is_empty(),
        "默认装配不该有每轨覆盖"
    );
    assert_eq!(window.get_track_height_percent(), 100);
    assert!(!window.get_track_height_drag_active());
    assert_eq!(window.get_track_height_drag_track(), "");
    assert_eq!(window.get_track_height_drag_start_px(), 0);
    assert_eq!(window.get_track_height_drag_start_y(), 0.0);

    // ② 行几何：逐字段等于 S0 的字面量（行 0 从 42 起、每行 56 的槽高、矩形高 54）。
    let ys = injected_lengths(&window.get_track_ys());
    let heights = injected_lengths(&window.get_track_heights());
    assert!(ys.len() >= 3, "夹具必须有多条轨道");
    for (index, y) in ys.iter().enumerate() {
        assert_eq!(
            *y,
            42.0 + 56.0 * index as f32,
            "第 {index} 行的顶沿必须是 S0 的前缀和（42 + 56 × i）"
        );
    }
    for (index, height) in heights.iter().enumerate() {
        assert_eq!(*height, 54.0, "第 {index} 行的矩形高必须是 S0 的字面量 54");
    }
    // 剪辑与自动化带同源：默认几何下它们的 y 也落在 S0 的前缀和上。
    let clip_ys = injected_lengths(&window.get_clip_ys());
    assert!(
        clip_ys
            .iter()
            .all(|y| (*y - 46.0).rem_euclid(56.0).abs() < f32::EPSILON),
        "默认几何下每个剪辑的顶沿必须落在某一行内（S0：行顶沿 + 4px 内缩）: {clip_ys:?}"
    );

    report_line(&format!(
        "[adr-0004-s1-drag] 默认外观: {} 行 ⇒ ys {:?} / heights {:?}（S0 字面量）；\
         clip-ys {} 项全部落在 S0 行内；手势状态位全为默认",
        ys.len(),
        &ys[..ys.len().min(4)],
        &heights[..heights.len().min(4)],
        clip_ys.len()
    ));
}

/// 判据 18（**产品形态**）：`--enable-ui-mcp-http` 走的那一条装配路径
/// （`LiveUi::into_production_control_plane` + `LiveControlPlane::into_service`）。
///
/// 为什么这条判据必须存在（而不是"产品代码写好了就行"）：
///
/// 1. `src/live_surface.rs` 同时被产品库与**本测试目标**编译（`#[path]`）。这两个方法
///    只被产品路径调用 ⇒ 少了本判据，它们在测试目标的编译里是 `dead_code`
///    （本仓库 `-D warnings` ⇒ 硬错误），而且"产品形态真的按生产模式装配"这句话
///    就没有任何会变红的证据；
/// 2. 产品形态与判据形态的**唯一**差别就是运行模式：判据用 `RunMode::Test` 才能注入事件，
///    产品必须用 `RunMode::Production`（`ui:inject` 族硬禁）。这条判据把那个差别钉住：
///    在**同一条**真实执行面上，注入被硬拒而管理族照常授权。
#[test]
fn production_plane_denies_injection_and_really_switches_the_live_view() {
    use live::{LiveWiringOptions, build_live_ui_with};
    use serde_json::{Value, json};
    use yeban_ui_mcp::methods::{METHOD_DISPATCH_KEY_PRESS, METHOD_SWITCH_MAIN_VIEW};

    let project = yeban_model::samples::filled_project();
    let wiring = LiveWiringOptions {
        permission: Permission::Administrative,
        console_tab: 0,
        // `ui/force_save` 的落点：本判据不碰保存（它落盘），所以不给路径 ——
        // 与产品路径"样本没有磁盘对应物"的形态一致。
        save_path: None,
        engine_quanta: 0,
        // 本判据只判生产模式的注入闸门，不接撤销端口。
        undo: None,
    };
    let mut plane = build_live_ui_with(&project, &wiring)
        .expect("真实界面 + Tier-1 执行面")
        .into_production_control_plane(Permission::Administrative);

    // ① 绑定之前的进程内探针（`src/ui_mcp_serve.rs` 的第一步就是它）：树真的可读。
    let (tree, before) = plane.plane().tree().expect("ui/tree");
    assert!(
        tree.count >= 40,
        "运行时控件树只有 {} 个节点，布局或 debug info 疑似没生效",
        tree.count
    );

    // ② 生产模式 ⇒ 注入族硬禁。注意这条链路上的令牌是**对**的（`try_line` 自己带），
    //    所以 403 而不是 401 —— 这正是"硬禁**先于**令牌校验"的 HTTP 侧证据。
    let denied = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        101,
        METHOD_DISPATCH_KEY_PRESS,
        Some(json!({"keyCode": "Tab"})),
    ));
    assert_eq!(denied.status, 403, "{denied:?}");
    assert_eq!(denied.kind.as_deref(), Some("forbidden-in-production"));

    // ③ 管理族（`app:admin`）真的被授权，而且**真的改了窗口**：
    //    回执里 `arrangementView` 是**回读**值，运行时控件树也随之换了字节。
    //
    //    目标视图由**观测到的树**推出（而不是写死一个方向）：默认装配落在哪一支是
    //    `MainWindow.arrangement-view` 的初值决定的，写死方向会让这条判据在初值变化时
    //    变成"换到一个本来就是的视图 ⇒ 树没变 ⇒ 假红"。用树里的画布 ID 决定方向，
    //    正好也是外部调用方（人 / AI）唯一能依据的证据。
    let (target, other, expected_arrangement) = if before.contains("workspace-arrangement-canvas") {
        ("session", "workspace-arrangement-canvas", false)
    } else {
        ("arrangement", "workspace-session-canvas", true)
    };
    assert!(
        before.contains(other),
        "初次读到的树里应当有 `{other}`（否则这条判据的前提不成立）"
    );
    let switched = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        102,
        METHOD_SWITCH_MAIN_VIEW,
        Some(json!({"view": target})),
    ));
    assert!(!switched.is_error(), "换主视图必须被授权: {switched:?}");
    let result = switched.result.expect("成功响应必须有 result");
    assert_eq!(result["accepted"], Value::Bool(true));
    assert_eq!(result["view"], target);
    assert_eq!(
        result["report"]["arrangementView"],
        Value::Bool(expected_arrangement),
        "`arrangementView` 必须是写完之后**回读**到的值: {result}"
    );
    let (_, after) = plane.plane().tree().expect("ui/tree (after)");
    assert_ne!(
        before, after,
        "换主视图必须改变运行时控件树的投影（否则这条动作只是个回执）"
    );
    assert!(
        after.contains(&format!("workspace-{target}-canvas")),
        "换到 {target} 之后运行时树里必须有对应的画布: {after}"
    );
    report_line(&format!(
        "[ui-mcp-http] 生产模式平面: 树 {} 节点; 注入族 403 {}; \
         换主视图 -> {target} ⇒ arrangementView={expected_arrangement}, 树字节 {} -> {}",
        tree.count,
        denied.kind.as_deref().unwrap_or("<none>"),
        before.len(),
        after.len(),
    ));

    // ④ `into_service()` 交出去的**就是**这一个服务：同一个令牌、同一个模式、同一个执行面。
    //    这一条不是恒等式 —— 环回传输按值持有它，交错了（或另生成一份凭据）就全错。
    let token = plane.plane().token().expose().to_owned();
    let surface = plane.plane().surface_name();
    let service = plane.into_service();
    assert!(
        service.mode().is_production(),
        "产品形态必须用 RunMode::Production (ui:inject 的硬禁就靠它)"
    );
    assert_eq!(service.expected_token().expose(), token);
    assert_eq!(service.surface_name(), surface);
}

/// 判据 18b（**跨 crate 字面值对账**）：`yeban-app::cli` 里那三个"为了默认构建也能打用法
/// 文本"而保留的副本，必须与 `yeban-ui-mcp` 的权威定义**逐字节相同**。
///
/// 为什么必须有这条：字面值有两份是实现的需要（默认构建里 `yeban-ui-mcp` 只是
/// dev-dependency，`usage_text()` 却必须打得出来），而"两份不许漂移"只能靠判据 ——
/// 与 `tests/in_process_mcp.rs` 对 `--enable-mcp-http` / `YEBAN_MCP_HTTP` 做的是同一件事。
/// 端点路径那一条还有第二个理由：`ui/*` 与领域 MCP 接**不同**的路径，
/// 接错端口时人应当能一眼看出来（`/ui-mcp` vs `/mcp`）。
#[test]
fn the_ui_mcp_http_switch_literals_match_the_ui_mcp_crate() {
    use yeban_app::cli::{UI_MCP_HTTP_FEATURE, UI_MCP_HTTP_SWITCH, UI_MCP_PATH};

    assert_eq!(UI_MCP_HTTP_SWITCH, yeban_ui_mcp::ENABLE_HTTP_FLAG);
    assert_eq!(UI_MCP_HTTP_FEATURE, yeban_ui_mcp::HTTP_FEATURE_NAME);
    assert_eq!(UI_MCP_PATH, yeban_ui_mcp::transport::UI_MCP_PATH);
    assert_eq!(UI_MCP_HTTP_FEATURE, "ui-mcp-http");
    assert_ne!(
        UI_MCP_PATH, "/mcp",
        "UI 控制面的端点必须与领域 MCP 的 `/mcp` 不同"
    );
    report_line(&format!(
        "[ui-mcp-http] 字面值对账: {UI_MCP_HTTP_SWITCH} / feature {UI_MCP_HTTP_FEATURE} / \
         端点 {UI_MCP_PATH} 与 yeban-ui-mcp 的权威定义逐字节相同"
    ));
}

// ===========================================================================
// 删除选区（`Delete`/`Backspace`）：真实事件源 → 一次可撤销提交
// ===========================================================================

/// 运行树里的**真**音符身份：`note-{26 字符 ULID}-rect` 的 `{ulid}` 段。
///
/// 不能只按 `note-` / `-rect` 前后缀取：静态叠加层 `note-suggestion-overlay-rect`
/// 也满足那对前后缀（实测：一棵 85 节点的树里 5 个候选、其中 1 个是叠加层）。
fn note_ulids_in_tree(tree: &ControlTree) -> Vec<String> {
    tree.with_prefix("note-")
        .filter_map(|node| {
            node.id
                .strip_prefix("note-")
                .and_then(|rest| rest.strip_suffix("-rect"))
                .map(str::to_owned)
        })
        .filter(|ulid| ulid.len() == 26 && ulid.chars().all(|c| c.is_ascii_alphanumeric()))
        .collect()
}

/// 音符族规模 = [`note_ulids_in_tree`] 的条目数（**条目**, 不是行）。
fn note_rects(tree: &ControlTree) -> usize {
    note_ulids_in_tree(tree).len()
}

/// 工程里的音符**条目**数（片段池键序 → 音符键序；与投影同源）。
fn model_note_count(project: &YebanProjectV1) -> usize {
    project
        .clip_pool
        .values()
        .filter_map(|entry| entry.content.notes())
        .map(std::collections::BTreeMap::len)
        .sum()
}

/// 判据 D1（删除选区, **默认构建**）：`Backspace` 经真实事件源删除当前选区,
/// 且**一次撤销整批回来**。
///
/// ## 三行读数（全部来自活窗口）
///
/// | 步 | 动作 | 读数 |
/// | :--- | :--- | :--- |
/// | ① | 注入前 | 运行时树里的音符元素数 = N |
/// | ② | 注入 `Backspace` | N − k（k = 选区大小） |
/// | ③ | `Cmd+Z` | 回到 N |
///
/// ## 怎么变红
///
/// 把 `apply_action` 的 `DeleteSelection` 分支摘掉（或让按键被吞却不提交）⇒ ② 步读数不变。
/// 把整批拆成 k 次 `commit_ops`（非原子）⇒ ③ 步只回来 1 个。
#[test]
fn backspace_deletes_the_selection_through_the_event_source_and_one_undo_restores_all() {
    use slint::Model as _;
    use std::rc::Rc;
    use yeban_app::undo::{UndoPort, UndoSession};
    use yeban_ui_test_port::port::KeyCode;

    const NOW: u64 = 1_760_000_000_000;
    let project = yeban_model::samples::filled_project();
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<判据:删除选区>", "yeban-app", project.clone(), NOW)
            .expect("打开撤销会话"),
    ));
    let wiring = LiveWiringOptions {
        permission: Permission::Interactive,
        console_tab: 0,
        save_path: None,
        engine_quanta: 0,
        undo: Some(Rc::clone(&port)),
    };
    let mut ui = build_live_ui_with(&project, &wiring).expect("真实界面 + Tier-1 执行面");

    // 选区 = 界面属性 `selected-ulids`（删除的唯一事实源）。选运行时树里**全部**音符元素
    // （这样"树里数到 k 个"与"选中 k 个"是同一份集合, 不存在树/投影口径差）。
    let selected = note_ulids_in_tree(&ui.tree_snapshot());
    let k = selected.len();
    assert!(
        k >= 1,
        "夹具必须至少有一个可见音符（否则本判据的对照不成立）"
    );
    ui.ui()
        .set_selected_ulids(slint::ModelRc::new(slint::VecModel::from(
            selected
                .iter()
                .map(|id| slint::SharedString::from(id.as_str()))
                .collect::<Vec<_>>(),
        )));
    ui.ui()
        .set_selected_note_count(i32::try_from(k).unwrap_or(i32::MAX));

    let before_tree = note_rects(&ui.tree_snapshot());
    let before_model = model_note_count(&port.project());
    assert_eq!(
        before_tree, k,
        "① 起点: 树里的音符元素数必须等于被选中的身份数"
    );

    // ① → ②：真实事件源注入 `Backspace`（端口 → 窗口事件 → key-handler → wire_keys → apply_action）。
    ui.dispatch_key_press(KeyCode::Backspace)
        .expect("注入 Backspace");
    assert_eq!(
        ui.ui().get_note_ulids().row_count(),
        before_tree - k,
        "宿主自己的重投影必须立刻把界面音符数组减掉 k 个"
    );
    // 键盘路径直接写窗口属性 ⇒ 控制面的树缓存要显式重抓（按端口工程重新投影）。
    ui.apply_project(&port.project()).expect("按端口工程重抓树");
    let after_tree = note_rects(&ui.tree_snapshot());
    let after_model = model_note_count(&port.project());
    assert_eq!(
        after_tree,
        before_tree - k,
        "② 树里的音符元素数必须 = N − k"
    );
    assert_eq!(after_model, before_model - k, "模型侧也必须少 k 个音符");

    // ③：`Cmd+Z`（`ui/dispatch_key_press` 表达不了修饰键 chord ⇒ 走 `.slint` 回调那一格,
    // 与既有判据 16 的第 ④ 步同款）。
    assert!(
        ui.ui()
            .invoke_key_action("z".into(), false, true, false, false),
        "`Cmd+Z` 必须被消费"
    );
    ui.apply_project(&port.project()).expect("撤销后重抓树");
    let restored_tree = note_rects(&ui.tree_snapshot());
    let restored_model = model_note_count(&port.project());
    assert_eq!(
        restored_tree, before_tree,
        "③ 一次撤销必须让 k 个音符**全部**回到树里"
    );
    assert_eq!(restored_model, before_model, "模型侧也必须回到 N");

    report_line(&format!(
        "[delete-selection] 默认装配: 注入前 音符元素 N={before_tree}; 注入 `Backspace` 后 \
         N−k={after_tree}（k={k}）; `Cmd+Z` 后 N={restored_tree}"
    ));
}

// ===========================================================================
// 复制选区（`Cmd`/`Ctrl+D`）：真实事件源 → 一次可撤销提交（右移一个吸附网格）
// ===========================================================================

/// 判据 D3（复制选区, **默认构建**）：`Cmd+D` 经真实事件源把选区里的每个音符
/// **复制到下一格**（右移一个吸附网格）, 副本拿**全新身份**; 且**一次撤销整批撤掉**;
/// 空选区**不消费**（`reject`）且一位不变。
///
/// ## 量什么 / 怎么量 / 单位
///
/// | 量 | 怎么量 | 单位 |
/// | :--- | :--- | :--- |
/// | 音符条目数 | [`model_note_count`]（**模型**侧 oracle, 走 `clip_pool`） | 条目 |
/// | 音符元素数 | [`note_rects`]（运行时控制树的 `note-{ulid}-rect`） | 条目 |
/// | 副本位置 | `(start_tick, pitch, id)` 三元组, 从 `port.project()` 读 | tick / 半音 / 身份 |
/// | 键是否被消费 | `invoke_key_action` 的**返回值**（`true` = `accept`） | 布尔 |
/// | 提交数 | `port.display().commit_count`（`CommitGraph` 的模型读数） | 次 |
/// | 画面变化 | [`frame_diff`] 的差异**像素数** + 包围盒（逐像素比 RGB） | 像素 |
///
/// ## 算术（全部整数；期望值在这里手算, **不引用被测函数**）
///
/// - 夹具（`filled_project`）：**4** 个音符, 起点 `0 / 960 / 1920 / 2880`,
///   音高 `60 / 64 / 67 / 72`, 时值各 `480`。
///   出处：`crates/yeban-model/src/samples.rs` 的 `filled_project`。
/// - 吸附网格 = **240** tick（1/16）—— 与 `host::ROLL_SNAP_GRID_TICKS` 同值, 本文件写成
///   字面量当独立 oracle。
/// - 全选（`k = 4`）⇒ 复制后 `N + k = 4 + 4 = 8` 条。
/// - 副本起点 = 原文 + 240 ⇒ `240 / 1200 / 2160 / 3120`; 音高**不变**（`60 / 64 / 67 / 72`）。
///   两个 tick 集合**不相交** ⇒ 副本与原文可区分。
/// - 一次 `Cmd+Z` ⇒ 回到 `4` 条 ⇒ 整批是**一个** `Op::Batch`（`ARCH-OPS-002`）。
/// - 一次 `Cmd+D` ⇒ `commit_count` 恰好 `+1`（不是 `+k`）。
///
/// ## 怎么变红（注入实测, 见本提交的说明）
///
/// | 注入 | 位置 | 现象 |
/// | :--- | :--- | :--- |
/// | 把 `Duplicate` 放回 `action_has_implementation` 的 `!matches!` | `src/host.rs` | ② 步音符数不变（键被 `reject`）, `cli_contract.rs` 的 B11e 也红 |
/// | 让副本复用原文的 `id` | `src/host.rs` 的 `duplicate_ops_for` | ② 步为 0：`AddNote::precondition` 以 `DuplicateEntityId` 拒绝整批 |
/// | 把整批拆成 k 次 `commit_ops` | `src/host.rs` 的 `Duplicate` 分支 | ③ 步只回来 1 条, 且 ② 步 `commit_count` 变成 `+k` |
#[test]
fn cmd_d_duplicates_the_selection_one_grid_to_the_right_and_one_undo_restores_all() {
    use slint::Model as _;
    use std::collections::BTreeSet;
    use std::rc::Rc;
    use yeban_app::undo::{UndoPort, UndoSession};

    /// 吸附网格（1/16）。独立 oracle：与 `host::ROLL_SNAP_GRID_TICKS` 同值但分开写。
    const GRID: u64 = 240;
    const NOW: u64 = 1_760_000_000_000;

    /// 模型里全部音符的 `(start_tick, pitch, id)` 三元组, 按字典序排序。
    /// 单位：tick / 半音 / 26 字符规范文本身份。**模型侧 oracle**, 不走投影。
    fn model_note_keys(project: &YebanProjectV1) -> Vec<(u64, u8, String)> {
        let mut all: Vec<(u64, u8, String)> = project
            .clip_pool
            .values()
            .filter_map(|entry| entry.content.notes())
            .flat_map(|notes| notes.values())
            .map(|note| (note.start_tick, note.pitch, note.id.to_canonical_string()))
            .collect();
        all.sort();
        all
    }

    let project = yeban_model::samples::filled_project();
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<判据:复制选区>", "yeban-app", project.clone(), NOW)
            .expect("打开撤销会话"),
    ));
    let wiring = LiveWiringOptions {
        permission: Permission::Interactive,
        console_tab: 0,
        save_path: None,
        engine_quanta: 0,
        undo: Some(Rc::clone(&port)),
    };
    let mut ui = build_live_ui_with(&project, &wiring).expect("真实界面 + Tier-1 执行面");

    // ---- ① 注入前：音符数 N + 选区 ----
    // 选区 = 界面属性 `selected-ulids`（唯一事实源）。选运行时树里**全部**音符元素,
    // 这样"树里数到 k 个"与"选中 k 个"是同一份集合。
    let selected = note_ulids_in_tree(&ui.tree_snapshot());
    let k = selected.len();
    assert_eq!(
        k, 4,
        "夹具（`filled_project`）写入了 4 个音符, 运行时树里必须数到 4 个"
    );
    select_ulids(ui.ui(), &selected);
    // 选中标志由 `apply_view` 从 `selected-ulids` 重算 ⇒ 改选区之后必须重投影一次。
    ui.apply_project(&port.project()).expect("重抓树（全选）");

    let before_model = model_note_count(&port.project());
    let before_tree = note_rects(&ui.tree_snapshot());
    assert_eq!(
        (before_model, before_tree),
        (4, 4),
        "① 起点: 模型条目数与树元素数都必须是 4"
    );
    let before_keys = model_note_keys(&port.project());
    assert_eq!(
        before_keys
            .iter()
            .map(|(tick, pitch, _)| (*tick, *pitch))
            .collect::<Vec<_>>(),
        vec![(0, 60), (960, 64), (1920, 67), (2880, 72)],
        "① 起点: 夹具的 (tick, pitch) 与 `samples.rs` 逐项一致"
    );
    assert_eq!(
        port.display().undoable,
        0,
        "① 起点不该有可撤销的编辑（会话打开时的首个提交已经落在 `commit_count` 里）"
    );
    let before_commits = port.display().commit_count;

    // 默认真外观（**未注入按键**）：同一状态两次抓帧逐字节相同 = 命题①（确定性）。
    // sha256 是**PNG 字节**的读数（本仓 PNG 是存储式 deflate ⇒ 尺寸证明不了内容）。
    let frame_before = ui.capture().expect("复制前的帧");
    let frame_before_again = ui.capture().expect("复制前的帧（第二次）");
    assert!(
        frame_diff(&frame_before, &frame_before_again).is_none(),
        "命题①: 同一状态的两次抓帧必须逐字节相同（默认帧可复现）"
    );
    let (before_png, before_evidence) =
        encode_with_evidence(&frame_before, DEFAULT_MAX_PNG_BYTES).expect("默认帧必须可编码");
    let before_digest = yeban_model::ids::ContentHash::of_bytes(&before_png);
    report_line(&format!(
        "[duplicate-pixel] 默认外观（未注入按键）: {}x{} PNG {} 字节 / non_black={} / 颜色 {} 种 / \
         指纹 {:016x} / sha256={}（PNG 字节的 sha256）",
        frame_before.width(),
        frame_before.height(),
        before_png.len(),
        before_evidence.non_black_pixels,
        before_evidence.distinct_colors,
        before_evidence.fingerprint,
        before_digest.as_str()
    ));

    // ---- ② 注入 `Cmd+D`：N → N + k，副本右移一个网格 ----
    // `.slint` 表达不了修饰键 chord ⇒ 走 `key-action` 回调那一格（与 D1 的 `Cmd+Z` 同款）。
    assert!(
        ui.ui()
            .invoke_key_action("d".into(), false, true, false, false),
        "② `Cmd+D` 必须被消费（`accept`）"
    );
    ui.apply_project(&port.project()).expect("复制后重抓树");
    let after_model = model_note_count(&port.project());
    let after_tree = note_rects(&ui.tree_snapshot());
    assert_eq!(after_model, before_model + k, "② 模型条目数必须 = N + k");
    assert_eq!(
        after_tree,
        before_tree + k,
        "② 树里的音符元素数必须 = N + k"
    );
    assert_eq!(
        ui.ui().get_note_ulids().row_count(),
        before_tree + k,
        "② 宿主自己的重投影必须立刻让界面音符数组长出 k 条"
    );
    assert_eq!(
        port.display().commit_count,
        before_commits + 1,
        "② 一次按键 = **一次**提交（不是 k 次；这是原子性的模型读数）"
    );

    // 副本 = 出现在复制后、但不在复制前的那些三元组。
    let originals: BTreeSet<(u64, u8, String)> = before_keys.iter().cloned().collect();
    let after_keys = model_note_keys(&port.project());
    let copies: Vec<&(u64, u8, String)> = after_keys
        .iter()
        .filter(|key| !originals.contains(*key))
        .collect();
    assert_eq!(copies.len(), k, "② 恰好 k 条**新**条目（原文必须还在）");
    let mut copy_pairs: Vec<(u64, u8)> = copies
        .iter()
        .map(|(tick, pitch, _)| (*tick, *pitch))
        .collect();
    copy_pairs.sort_unstable();
    assert_eq!(
        copy_pairs,
        // 原文起点 `0 / 960 / 1920 / 2880` 各加一个网格（`0 + GRID` 就是 `GRID`）。
        vec![
            (GRID, 60),
            (960 + GRID, 64),
            (1920 + GRID, 67),
            (2880 + GRID, 72)
        ],
        "② 副本必须是原文**右移一个吸附网格**、音高不变（算术见文档）"
    );
    // 原文与副本的 tick 集合不相交 ⇒ 二者可区分（不是"覆盖掉原文"）。
    let original_ticks: BTreeSet<u64> = before_keys.iter().map(|(tick, _, _)| *tick).collect();
    let copy_ticks: BTreeSet<u64> = copy_pairs.iter().map(|(tick, _)| *tick).collect();
    assert!(
        original_ticks.is_disjoint(&copy_ticks),
        "② 原文起点 {original_ticks:?} 与副本起点 {copy_ticks:?} 必须不相交"
    );
    // 身份全新且互不相同：复制后 8 条, 身份也必须是 8 个不同的字符串。
    let distinct_ids: BTreeSet<&String> = after_keys.iter().map(|(_, _, id)| id).collect();
    assert_eq!(
        distinct_ids.len(),
        after_keys.len(),
        "② 复制后全部身份必须互不相同（不复用原文的 id）"
    );

    // 命题②: **复制后画面必然变**（多了 k 个音符矩形）。默认外观那一次抓帧是基线。
    let frame_after_dup = ui.capture().expect("复制后的帧");
    let diff = frame_diff(&frame_before, &frame_after_dup)
        .unwrap_or_else(|| panic!("② 复制后画面必须变（模型多了 {k} 个音符）, 实际逐字节相同"));
    report_line(&format!(
        "[duplicate-pixel] 复制后: 差异像素 {} / 包围盒 x={} y={} w={} h={}（复制前后各抓一帧逐像素比 RGB）",
        diff.count, diff.bbox.x, diff.bbox.y, diff.bbox.width, diff.bbox.height
    ));

    // ---- ③ 一次 `Cmd+Z`：回到 N（证明整批是一个 `Op::Batch`）----
    assert!(
        ui.ui()
            .invoke_key_action("z".into(), false, true, false, false),
        "③ `Cmd+Z` 必须被消费"
    );
    ui.apply_project(&port.project()).expect("撤销后重抓树");
    assert_eq!(
        model_note_count(&port.project()),
        before_model,
        "③ 一次撤销必须回到 N（k 条副本一起消失 ⇒ 原子）"
    );
    assert_eq!(
        note_rects(&ui.tree_snapshot()),
        before_tree,
        "③ 树里的音符元素数也必须回到 N"
    );
    assert_eq!(
        model_note_keys(&port.project()),
        before_keys,
        "③ (tick, pitch, id) 三元组必须逐项回到起点"
    );

    // ---- ④ 空选区按 `Cmd+D`：**不消费**且一位不变 ----
    select_ulids(ui.ui(), &[]);
    ui.apply_project(&port.project()).expect("重抓树（空选区）");
    // "一位不变"的探针: 注入**紧邻**前后各抓一帧（选区状态相同 ⇒ 唯一变量就是那次按键）。
    let frame_empty_before = ui.capture().expect("空选区注入前的帧");
    let consumed_empty = ui
        .ui()
        .invoke_key_action("d".into(), false, true, false, false);
    assert!(
        !consumed_empty,
        "④ 空选区按 `Cmd+D` 必须**不消费**（`reject`, 与 `DeleteSelection` 同一取向）"
    );
    let frame_empty_after = ui.capture().expect("空选区注入后的帧");
    assert!(
        frame_diff(&frame_empty_before, &frame_empty_after).is_none(),
        "④ 空选区按 `Cmd+D` 之后画面必须逐字节不变"
    );
    assert_eq!(
        model_note_count(&port.project()),
        before_model,
        "④ 空选区按 `Cmd+D` 之后模型条目数一位不变"
    );
    assert_eq!(
        model_note_keys(&port.project()),
        before_keys,
        "④ 空选区按 `Cmd+D` 之后 (tick, pitch, id) 三元组一位不变"
    );
    assert_eq!(
        port.display().commit_count,
        before_commits + 1,
        "④ 空选区按 `Cmd+D` 不得新增提交（仍是那一次复制的提交）"
    );

    report_line(&format!(
        "[duplicate-selection] 默认装配: 注入前 音符条目 N={before_model}（选中 k={k}）; \
         注入 `Cmd+D` 后 N+k={after_model}（副本起点 {copy_ticks:?}, 原文起点 {original_ticks:?}）; \
         `Cmd+Z` 后 N={}; 空选区按 `Cmd+D` 消费={consumed_empty}（要求 false）",
        model_note_count(&port.project())
    ));
}

/// 判据 D2（删除选区, `in-process-mcp`）：同一次按键的删除落在**唯一权威**上,
/// 且 `Cmd+Z` 从**同一个**权威回来（`ROAD-M4-008` 选项 (a)）。
///
/// 与判据 D1 的差别是**结构性的**：这里没有本地会话, 工程的唯一来源是控制面 `Domain`；
/// 因此"删除进的是同一个会话"由 `apply_revision` 的前进来见证, 而不是由本地端口自证。
#[cfg(feature = "in-process-mcp")]
#[test]
fn a_gui_delete_lands_in_the_same_authority_and_one_undo_restores_all() {
    use std::rc::Rc;
    use yeban_app::mcp_mount::{InProcessMcp, SessionSource};
    use yeban_app::undo::UndoPort;
    use yeban_ui_test_port::port::KeyCode;

    let project = yeban_model::samples::filled_project();
    let mount = InProcessMcp::start_for_project(
        true,
        project,
        SessionSource::InMemory(PathBuf::from("sample:delete-selection")),
    )
    .expect("挂载决策")
    .expect("运行期开关打开时必须真的挂载");
    let authority = mount.project_authority();
    let port = Rc::new(UndoPort::from_authority(authority.clone()));
    let wiring = LiveWiringOptions {
        permission: Permission::Interactive,
        console_tab: 0,
        save_path: None,
        engine_quanta: 0,
        undo: Some(Rc::clone(&port)),
    };
    let mut ui = live::build_live_ui_from_authority_with(&authority, &wiring).expect("装配");

    let before_revision = authority.apply_revision();
    let before_tree = note_rects(&ui.tree_snapshot());
    let before_model = model_note_count(&authority.project().expect("权威有活跃工程"));
    // 选区 = 运行时树里的全部音符元素（与 `note_rects` 同一份集合）。
    let selected = note_ulids_in_tree(&ui.tree_snapshot());
    let k = selected.len();
    assert!(k >= 1, "夹具必须至少有一个可见音符");
    ui.ui()
        .set_selected_ulids(slint::ModelRc::new(slint::VecModel::from(
            selected
                .iter()
                .map(|id| slint::SharedString::from(id.as_str()))
                .collect::<Vec<_>>(),
        )));
    ui.ui()
        .set_selected_note_count(i32::try_from(k).unwrap_or(i32::MAX));

    // ① → ②：注入 `Backspace` ⇒ 权威工程少 k 个、修订号 +1。
    ui.dispatch_key_press(KeyCode::Backspace)
        .expect("注入 Backspace");
    assert_eq!(
        authority.apply_revision(),
        before_revision + 1,
        "GUI 的删除必须推进**同一个**权威的施加修订号"
    );
    assert_eq!(
        model_note_count(&authority.project().expect("权威有活跃工程")),
        before_model - k,
        "② 权威工程里的音符必须少 k 个"
    );
    assert_eq!(
        ui.sync_authority().expect("刷新投影"),
        AuthoritySync::Reprojected {
            revision: before_revision + 1
        }
    );
    let after_tree = note_rects(&ui.tree_snapshot());
    assert_eq!(
        after_tree,
        before_tree - k,
        "② 树里的音符元素数必须 = N − k"
    );

    // ③：`Cmd+Z` ⇒ 权威修订号再 +1、k 个音符全部回来。
    assert!(
        ui.ui()
            .invoke_key_action("z".into(), false, true, false, false),
        "`Cmd+Z` 必须被消费"
    );
    assert_eq!(
        authority.apply_revision(),
        before_revision + 2,
        "撤销也必须落在**同一个**权威上"
    );
    assert_eq!(
        model_note_count(&authority.project().expect("权威有活跃工程")),
        before_model,
        "③ 一次撤销必须让 k 个音符全部回到权威工程"
    );
    assert_eq!(
        ui.sync_authority().expect("刷新投影"),
        AuthoritySync::Reprojected {
            revision: before_revision + 2
        }
    );
    let restored_tree = note_rects(&ui.tree_snapshot());
    assert_eq!(restored_tree, before_tree, "③ 树里的音符元素数必须回到 N");

    report_line(&format!(
        "[delete-selection-authority] 注入前 音符元素 N={before_tree}; 注入 `Backspace` 后 \
         N−k={after_tree}（k={k}）; `Cmd+Z` 后 N={restored_tree}; 权威修订号 \
         {before_revision}→{}→{}",
        before_revision + 1,
        before_revision + 2
    ));
}

// =====================================================================================
// 纯视图态的四条回调（本切片）：`toggle-view` / `toggle-sidebar` / `toggle-ai-drawer`
// / `open-musical-pr` —— 判据证明"入口真的能用"，不是"控件存在"。
//
// 两半，缺一不可（两条 `#[test]`，因为 Tier-1 平台是线程局部的：
// `slint::platform::set_platform` 每个测试各装一次，同一个测试里不能装第二遍）：
//
// | 判据 | 回答的问题 |
// | :--- | :--- |
// | [`the_view_state_callbacks_really_change_the_view_state`] | **真实点击**（§12.4 指针注入）⇒ 视图态属性真的翻了 |
// | [`the_view_state_properties_drive_the_runtime_tree`] | 属性 ⇒ **运行时控件树**里 `visible` 真的换了（`ui/tree` 的元素数与互斥画布） |
// =====================================================================================

/// 一条视图态判据的装配：真实界面 + 可注入的控制面（`Interactive` 才允许指针注入）。
///
/// **不需要额外接线**：装配路径（`src/live_surface.rs` 的 `build_live_ui_with`）与产品进程
/// 调的是**同一个** `host::wire_view_callbacks`（`src/main.rs` 的 `wire_callbacks` 逐字相同），
/// 因此这里注入的点击与用户点击走同一条链。
fn assemble_view_state_callbacks() -> (yeban_app::ui::MainWindow, LiveControlPlane) {
    let project = yeban_model::samples::filled_project();
    let ui = build_live_ui(&project, Permission::Interactive).expect("装配");
    let window = slint::ComponentHandle::clone_strong(ui.ui());
    let plane = ui.into_control_plane(Permission::Interactive);
    (window, plane)
}

/// 一个语义元素的**绝对**包围盒（经 `ui/node` 读，不写死像素 —— 布局一改坐标跟着走）。
fn element_bounds(plane: &mut LiveControlPlane, seq: i64, element_id: &str) -> serde_json::Value {
    let node = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        seq,
        "ui/node",
        Some(serde_json::json!({ "elementId": element_id })),
    ));
    assert!(
        !node.is_error(),
        "`{element_id}` 必须在运行时树里: {node:?}"
    );
    let bounds = node.result.expect("有 result")["node"]["bounds"].clone();
    assert!(!bounds.is_null(), "`{element_id}` 必须有几何包围盒");
    bounds
}

/// 一次**真实**点击：按下 → 移到同一点 → 松手。
///
/// 三步都走 §12.4 的注入方法（真实指针事件 → Slint 命中测试 → `.slint` 的 `TouchArea`），
/// 因此它命中的是**用户点的那一下**，而不是"直接调 Rust 回调"
/// （与 `the_track_header_drag_gesture_changes_the_row_geometry_and_ends_cleanly` 同手法）。
fn click_element(plane: &mut LiveControlPlane, seq: i64, element_id: &str) {
    use serde_json::json;
    use yeban_ui_mcp::methods::{METHOD_DISPATCH_POINTER_DOWN, METHOD_DISPATCH_POINTER_MOVE};

    let bounds = element_bounds(plane, seq, element_id);
    let x_offset = bounds["width"].as_f64().expect("width") / 2.0;
    let y_offset = bounds["height"].as_f64().expect("height") / 2.0;
    let absolute_x = bounds["x"].as_f64().expect("x") + x_offset;
    let absolute_y = bounds["y"].as_f64().expect("y") + y_offset;

    let down = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        seq + 1,
        METHOD_DISPATCH_POINTER_DOWN,
        Some(json!({
            "elementId": element_id,
            "xOffset": x_offset,
            "yOffset": y_offset,
            "button": "left"
        })),
    ));
    assert!(
        !down.is_error(),
        "`{element_id}` 按下必须落到真实窗口: {down:?}"
    );
    let moved = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        seq + 2,
        METHOD_DISPATCH_POINTER_MOVE,
        Some(json!({ "x": absolute_x, "y": absolute_y })),
    ));
    assert!(
        !moved.is_error(),
        "`{element_id}` 移动必须落到真实窗口: {moved:?}"
    );
    let up = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        seq + 3,
        "ui/dispatch_pointer_up",
        Some(json!({ "button": "left" })),
    ));
    assert!(
        !up.is_error(),
        "`{element_id}` 松手必须落到真实窗口: {up:?}"
    );
}

/// `ui/property` 读一个元素的属性（读数一律经控制面）。
fn read_property(
    plane: &mut LiveControlPlane,
    seq: i64,
    element_id: &str,
    name: &str,
) -> serde_json::Value {
    let line = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        seq,
        "ui/property",
        Some(serde_json::json!({ "elementId": element_id, "name": name })),
    ));
    assert!(!line.is_error(), "`{element_id}.{name}` 必须可读: {line:?}");
    line.result.expect("有 result")["value"].clone()
}

/// 一条视图态属性在**活窗口**上的字面读数（与判据 16 读 `arrangement-view` 同一手法）。
fn view_state_reading(window: &yeban_app::ui::MainWindow) -> String {
    format!(
        "arrangement-view={} sidebar-collapsed={} ai-drawer-open={} musical-pr-open={}",
        window.get_arrangement_view(),
        window.get_sidebar_collapsed(),
        window.get_ai_drawer_open(),
        window.get_musical_pr_open(),
    )
}

/// 判据（`ADR-0004` S1 同款口径）：**四条纯视图态回调真的被真实点击驱动**。
///
/// ## 每一段都钉住"注入前 ≠ 注入后"的字面读数
///
/// | 段 | 注入的元素 | 读回来的东西 | 期望 |
/// | :--- | :--- | :--- | :--- |
/// | `toggle-view` | `transport-view-toggle-button` | `arrangement-view` + 帧指纹 | `true` → `false`；指纹必须变 |
/// | `toggle-sidebar` | `sidebar-collapse-button` | `ui/property` 读 `sidebar.width` + `sidebar-collapsed` | 240.00 → 36.00 |
/// | `toggle-ai-drawer` | `transport-ai-drawer-button` | `ai-drawer-open` + 帧指纹（`compact=true`） | `false` → `true`；指纹必须变 |
/// | `open-musical-pr` | `ai-rail-musical-pr-button` | `musical-pr-open` + 帧指纹 | `false` → `true`；指纹必须变 |
///
/// ## 两条会变红的注入（都实测过，见账本当轮条目）
///
/// | 注入 | 位置 | 现象 |
/// | :--- | :--- | :--- |
/// | 摘掉 `host::wire_view_callbacks(&window);` | `src/live_surface.rs` | 四段的"注入后"读数与"注入前"逐字相同 ⇒ 四段一起红 |
/// | 把 `root.arrangement-view = !root.arrangement-view;` 加回转发块 | `ui/app.slint` | `.slint` 与宿主各翻转一次 ⇒ 互相抵消 ⇒ `toggle-view` 段红 |
///
/// ## 快照语义**已经取消**（本轮更正 `Round 431` 登记的那条边界）
///
/// `Round 431` 在这里写过一条"如实报告的边界"：`ui/tree` 是**快照**、`LiveAdminSurface`
/// 只在三个动作点重抓树 ⇒"点击改了可见性不会让下一次 `ui/tree` 变新"（那一轮把
/// `树节点 85→85` 原样打了出来）。**那条边界现在不成立了**：读方法在返回前会调
/// `UiSurface::refresh_runtime_tree`（`crates/yeban-ui-mcp/src/service.rs` 的
/// `refresh_before_read`），而 `LiveAdminSurface` 覆写它去重抓活窗口的树。因此判据 1
/// 的 `树节点 N→N` 读数**已经换成**真的变化（见判据
/// `a_tree_read_sees_a_click_that_just_happened` 的字面读数：点击 `toggle-sidebar` ⇒
/// 节点 **85→81**、`sidebar-search-field` 离开树、折叠按钮 `x` **210→6**）。
///
/// ⛔ 这里仍然**不**断言"两次读之间树必须不变"：那是把某一种缓存语义钉成契约。
/// 钉住的是**新鲜度** —— 读必须看见刚发生的变化。
///
/// ## 为什么不断言像素
///
/// 这四条回调**只有点击之后**才改属性；默认外观（未注入任何点击）的帧必须逐字节不变，
/// 那一条是渲染帧 sha256 的 A/B（`live-port-filled-project-unmasked-1920x1080.png`，
/// 6,222,418 字节），见账本当轮条目的"像素影响"。
#[test]
fn the_view_state_callbacks_really_change_the_view_state() {
    use serde_json::json;
    use yeban_ui_mcp::methods::METHOD_SCREENSHOT;

    let (window, mut plane) = assemble_view_state_callbacks();

    /// 抓一帧，返回（指纹, 非黑像素数）—— 像素是**活**读数，因此不经过快照树。
    ///
    /// `ui/screenshot` 的载荷是**平铺**的（`shot_payload` 的键序固定），所以
    /// `fingerprint` / `nonBlackPixels` 直接在 `result` 上（`crates/yeban-ui-mcp/src/service.rs:765`）。
    fn frame(plane: &mut LiveControlPlane, seq: i64) -> (String, u64) {
        let line = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
            seq,
            METHOD_SCREENSHOT,
            Some(json!({ "maskDynamic": false })),
        ));
        assert!(!line.is_error(), "抓帧必须成功: {line:?}");
        let shot = line.result.expect("有 result");
        (
            shot["fingerprint"].as_str().expect("指纹").to_owned(),
            shot["nonBlackPixels"].as_u64().expect("nonBlackPixels"),
        )
    }

    // ------------------------------------------------- ① toggle-view
    let before = view_state_reading(&window);
    let (frame_before, non_black_before) = frame(&mut plane, 300);
    let tree_before = plane.plane().tree().expect("ui/tree").0.count;
    click_element(&mut plane, 302, "transport-view-toggle-button");
    let after = view_state_reading(&window);
    let (frame_after, non_black_after) = frame(&mut plane, 306);
    let tree_after = plane.plane().tree().expect("ui/tree").0.count;
    report_line(&format!(
        "[view-state] toggle-view: 注入前 [{before}] → 注入后 [{after}]; \
         帧指纹 {frame_before}→{frame_after}; 非黑 {non_black_before}→{non_black_after}; \
         `ui/tree` 快照节点 {tree_before}→{tree_after}（快照语义，见判据文档）"
    ));
    assert!(
        !window.get_arrangement_view(),
        "① `toggle-view` 必须把 `arrangement-view` 翻成 false"
    );
    assert!(
        frame_before != frame_after,
        "① 视图真的换了 ⇒ 帧字节必须不同（同一执行面同一帧 → 同一指纹）"
    );

    // ------------------------------------------------- ② toggle-sidebar
    let sidebar_before = read_property(&mut plane, 310, "sidebar", "width");
    assert_eq!(
        sidebar_before.as_str(),
        Some("240.00"),
        "默认左资源栏宽 = `Tokens.left-rail-width`"
    );
    click_element(&mut plane, 312, "sidebar-collapse-button");
    let sidebar_after = read_property(&mut plane, 316, "sidebar", "width");
    report_line(&format!(
        "[view-state] toggle-sidebar: 注入前 [{}] → 注入后 [{}]; \
         `ui/property` 的 `sidebar.width` {sidebar_before}→{sidebar_after}",
        "sidebar-collapsed=false",
        view_state_reading(&window),
    ));
    assert!(
        window.get_sidebar_collapsed(),
        "② `toggle-sidebar` 必须把 `sidebar-collapsed` 翻成 true"
    );
    assert_eq!(
        sidebar_after.as_str(),
        Some("36.00"),
        "② 折叠后左栏必须是 `Tokens.left-rail-collapsed-width`（几何跟着属性走）"
    );

    // ------------------------------------------------- ③ toggle-ai-drawer
    // 抽屉的 `visible` 是 `root.compact && root.ai-drawer-open` ⇒ 默认（非 compact）下
    // 展开它**不改变任何像素**。为了拿到"切换前后有差异"，这一段先把断点切到 compact
    // （那是投影的既有输入之一，写同一个属性），于是像素只由 `ai-drawer-open` 决定。
    assert!(!window.get_ai_drawer_open(), "默认不展开 AI 抽屉");
    window.set_compact(true);
    let (drawer_frame_before, _) = frame(&mut plane, 320);
    click_element(&mut plane, 322, "transport-ai-drawer-button");
    let (drawer_frame_after, drawer_non_black_after) = frame(&mut plane, 326);
    report_line(&format!(
        "[view-state] toggle-ai-drawer: 注入前 [ai-drawer-open=false, compact=true] → 注入后 [{}]; \
         帧指纹 {drawer_frame_before}→{drawer_frame_after}; 非黑 {drawer_non_black_after}",
        view_state_reading(&window),
    ));
    assert!(
        window.get_ai_drawer_open(),
        "③ `toggle-ai-drawer` 必须把 `ai-drawer-open` 翻成 true"
    );
    assert!(
        drawer_frame_before != drawer_frame_after,
        "③ compact 断点下抽屉是**可见**的 ⇒ 帧字节必须不同"
    );
    window.set_compact(false);

    // ------------------------------------------------- ④ open-musical-pr
    assert!(!window.get_musical_pr_open(), "默认不打开提案抽屉");
    let (pr_frame_before, _) = frame(&mut plane, 330);
    click_element(&mut plane, 332, "ai-rail-musical-pr-button");
    let (pr_frame_after, _) = frame(&mut plane, 336);
    report_line(&format!(
        "[view-state] open-musical-pr: 注入前 [musical-pr-open=false] → 注入后 [{}]; \
         帧指纹 {pr_frame_before}→{pr_frame_after}",
        view_state_reading(&window),
    ));
    assert!(
        window.get_musical_pr_open(),
        "④ `open-musical-pr` 必须把 `musical-pr-open` 写成 true"
    );
    assert!(
        pr_frame_before != pr_frame_after,
        "④ 抽屉是 360px 宽的覆盖层 ⇒ 帧字节必须不同"
    );
}

/// 判据：**属性 ⇒ 运行时控件树**（`visible` 真的跟着那四个属性走）。
///
/// 这一条与上一条合起来才是完整的链：**真实点击 ⇒ 属性 ⇒ 可见性 ⇒ 树**。
/// 上一条证明前半段（点击 ⇒ 属性），这一条证明后半段（属性 ⇒ 树）。
///
/// ## 它为什么不是"自己写进去再自己读回来"的空转
///
/// 断言的**方向**是 `.slint` 的 `visible:` 绑定，不是宿主的能力：
/// `ArrangementView` / `SessionView` 的 `visible` 互斥、两个 `ai-rail` 实例的
/// `visible: root.compact && root.ai-drawer-open`、`MusicalPrDrawer` 的
/// `visible: root.musical-pr-open`。把任何一处 `visible:` 写坏（或改成常量）⇒ 本条红。
///
/// ## 树为什么要显式重抓
///
/// `ui/tree` 是快照，重抓点是既有的几个动作（见上一条判据的边界说明）。
/// 这里用 `LiveUi::pump_meters()`（既有的"电平轮询腿"：抽干 → 注入 → **重抓树**），
/// 因为它是**唯一**不需要 `UndoPort`、也不改工程的重抓触发点。
#[test]
fn the_view_state_properties_drive_the_runtime_tree() {
    let project = yeban_model::samples::filled_project();
    let mut ui = build_live_ui(&project, Permission::ReadOnly).expect("装配");

    // ---- 默认态（`compact = false`）：编曲画布在树里、矩阵画布不在、没有提案抽屉；
    //      `ai-rail` 恰好 **1** 个 —— 那是**常驻右栏**（`visible: !root.compact`），
    //      不是抽屉。两种断点下都恰好 1 个，所以"数 `ai-rail`"本身不是判据，
    //      判别力来自下面"compact 且 `ai-drawer-open: false` ⇒ 0 个"这一格。
    ui.pump_meters();
    let before = ui.tree_snapshot();
    assert!(
        before.contains("workspace-arrangement-canvas"),
        "默认必须有编曲画布"
    );
    assert!(
        !before.contains("workspace-session-canvas"),
        "默认不该有矩阵画布"
    );
    assert!(!before.contains("musical-pr-drawer"), "默认不该有提案抽屉");
    let rail_before = before.ids().filter(|id| *id == "ai-rail").count();
    assert_eq!(
        rail_before, 1,
        "非 compact ⇒ 常驻右栏那一个 `ai-rail` 实例在树里"
    );

    // ---- 切到 compact 断点、抽屉**关着**：这一格把两个 `ai-rail` 实例都从树里排除
    //      （常驻栏被 `!compact` 关掉，抽屉被 `ai-drawer-open` 关掉）。
    ui.ui().set_compact(true);
    ui.pump_meters();
    let compact_closed = ui.tree_snapshot();
    let rail_compact_closed = compact_closed.ids().filter(|id| *id == "ai-rail").count();
    assert_eq!(
        rail_compact_closed, 0,
        "`compact && !ai-drawer-open` ⇒ 两个 `ai-rail` 实例都不在树里"
    );

    // ---- 写入四个视图态属性（**与四条回调写的是同一个属性**），再让既有的重抓点重抓。
    ui.ui().set_arrangement_view(false);
    ui.ui().set_sidebar_collapsed(true);
    ui.ui().set_ai_drawer_open(true);
    ui.ui().set_musical_pr_open(true);
    ui.pump_meters();
    let after = ui.tree_snapshot();

    let rail_after = after.ids().filter(|id| *id == "ai-rail").count();
    let proposal_items = after
        .ids()
        .filter(|id| id.starts_with("musical-pr-proposal-"))
        .count();
    report_line(&format!(
        "[view-state] 属性 ⇒ 树: 节点 {}→{}; `workspace-arrangement-canvas` {}→{}; \
         `workspace-session-canvas` {}→{}; `ai-rail`（compact 且抽屉关）{rail_compact_closed}→{rail_after}; \
         `musical-pr-drawer` {}→{}; `musical-pr-proposal-*-item` 0→{proposal_items}",
        before.len(),
        after.len(),
        before.contains("workspace-arrangement-canvas"),
        after.contains("workspace-arrangement-canvas"),
        before.contains("workspace-session-canvas"),
        after.contains("workspace-session-canvas"),
        compact_closed.contains("musical-pr-drawer"),
        after.contains("musical-pr-drawer"),
    ));

    assert!(
        !after.contains("workspace-arrangement-canvas")
            && after.contains("workspace-session-canvas"),
        "`arrangement-view=false` ⇒ 两条画布必须在树里互换"
    );
    assert_eq!(
        rail_after, 1,
        "`compact && ai-drawer-open` ⇒ 抽屉那一个 `ai-rail` 实例进树（关着时是 0）"
    );
    assert!(
        after.contains("musical-pr-drawer"),
        "`musical-pr-open=true` ⇒ 提案抽屉进树"
    );
    // ⚠ 这一格 2026-10-08 由 `3` 改成 `0`：抽屉里的三条「提案」是**编造的演示数据**
    // （见 `tests/live_ui_mcp.rs` 的 `the_musical_pr_drawer_shows_an_honest_empty_state`
    // 与 `src/elements.rs` 的 `musical_pr_drawer_declares_no_demo_proposals`）。
    // 写成 `3` 等于把"界面显示假数据"钉成契约。
    assert_eq!(
        proposal_items, 0,
        "零提案时抽屉里不该有任何 `musical-pr-proposal-*-item`（那三条是编造的）"
    );
    assert!(
        after.contains("musical-pr-empty-state"),
        "零提案时抽屉里必须有那句用户可见的空态"
    );
    // ⚠ **不断言总节点数增加**：这一格里节点数由 85 降到 78（实测），因为同时发生的
    // 还有两件**减少**节点的事 —— `compact` 关掉常驻右栏那整棵子树、`arrangement-view=false`
    // 把编曲画布换成元素更少的矩阵画布。判别力因此来自**集合**（`contains`）与
    // 那个 0→1 的 `ai-rail`，不是来自计数。这一行是有意的**不**断言：把"变多"写成
    // 契约，下一轮任何让画布变大的切片都会无谓地红。

    // ---- 反证：把抽屉关回去 ⇒ 它必须离开树（否则上面那条 `contains` 恒真）。
    ui.ui().set_musical_pr_open(false);
    ui.pump_meters();
    let closed = ui.tree_snapshot();
    report_line(&format!(
        "[view-state] 反证: `musical-pr-open=false` ⇒ `musical-pr-drawer` 在树里 = {}; 节点 {}",
        closed.contains("musical-pr-drawer"),
        closed.len()
    ));
    assert!(
        !closed.contains("musical-pr-drawer"),
        "反证失败：抽屉关掉之后仍在树里 ⇒ 上面那条断言没有判别力"
    );
}

// =====================================================================================
// 判据：**抽屉不说谎** —— 零提案的抽屉里没有编造的数据，只有一句用户可见的真话
//
// 这是"AI 提案抽屉"的诚实性契约的**运行时那一半**；文本层与属性层那一半在
// `crates/yeban-app/src/elements.rs` 的 `musical_pr_drawer_declares_no_demo_proposals`
// 与 `no_host_writes_the_musical_pr_proposal_properties`。三条路径共用**同一份**假字面量
// （`yeban_app::elements::DEMO_PROPOSAL_LITERALS`），因此不存在"两份清单漂移"。
// =====================================================================================

/// 判据：打开抽屉 ⇒ 运行时树里**零条**提案、零条编造置信度，且有一句可见的空态。
///
/// | 路径 | 量什么 | 期望 |
/// | :--- | :--- | :--- |
/// | 属性 → 树 | `musical-pr-drawer` 节点的 `value`（= `accessible-value` 原文） | `"0"` |
/// | 树（ID） | `musical-pr-proposal-*-item` 的条数 | `0` |
/// | 树（标签） | 任何节点的 `accessible-label` / `accessible-value` 命中 6 条假字面量 | `0` 处 |
/// | 树（空态） | `musical-pr-empty-state` 的 `label` | 逐字等于界面上那句话 |
/// | 树（假控件） | `musical-pr-accept-button` / `-reject-button` | 都不在树里 |
/// | 像素 | 打开抽屉前后两帧的差异像素数与包围盒 | 必须变（不碰 golden） |
///
/// **为什么 `value` 这一格是"属性 ⇒ 树"**：`accessible-value` 是抽屉从 `proposal-count`
/// 算出来的文本，运行时树读的是**活控件**（`crates/yeban-ui-test-port/src/inspect.rs:186`）。
/// 把 `proposal-count` 改回演示常量 ⇒ 这一格立刻变成 `"3"`。
#[test]
fn the_musical_pr_drawer_shows_an_honest_empty_state() {
    let project = yeban_model::samples::filled_project();
    let mut ui = build_live_ui(&project, Permission::ReadOnly).expect("装配");

    // ---- 默认（抽屉关着）: 先抓一帧，作为"打开抽屉改变了画面"的对照
    ui.pump_meters();
    let before_tree = ui.tree_snapshot();
    assert!(
        !before_tree.contains("musical-pr-drawer"),
        "默认不打开提案抽屉"
    );
    let before_frame = ui.capture().expect("默认帧");

    // ---- 打开抽屉
    ui.ui().set_musical_pr_open(true);
    ui.pump_meters();
    let after = ui.tree_snapshot();
    assert!(after.contains("musical-pr-drawer"), "抽屉必须在树里");

    // ---- 属性 ⇒ 树: `accessible-value` 就是 `proposal-count` 的文本
    let drawer = after
        .find_by_id("musical-pr-drawer")
        .expect("`musical-pr-drawer` 节点");
    let count_reading = drawer.value.clone().unwrap_or_default();
    report_line(&format!(
        "[musical-pr] 属性 ⇒ 树: `musical-pr-drawer` 的 accessible-value = {count_reading:?} \
         （= 抽屉头 `AI 编曲提案 (N)` 里的 N）"
    ));
    assert_eq!(
        count_reading, "0",
        "零提案时抽屉必须如实报 0 条（`accessible-value` 是 `proposal-count` 的文本）"
    );

    // ---- 树（多路径探针）: 假数据不许回来
    let proposal_ids: Vec<&str> = after
        .ids()
        .filter(|id| id.starts_with("musical-pr-proposal-"))
        .collect();
    let mut leaks: Vec<String> = Vec::new();
    for node in after.iter() {
        for literal in yeban_app::elements::DEMO_PROPOSAL_LITERALS {
            if node.label.contains(literal) {
                leaks.push(format!(
                    "`{}` 的 accessible-label 命中 `{literal}`",
                    node.id
                ));
            }
            if node
                .value
                .as_deref()
                .is_some_and(|value| value.contains(literal))
            {
                leaks.push(format!(
                    "`{}` 的 accessible-value 命中 `{literal}`",
                    node.id
                ));
            }
        }
    }
    report_line(&format!(
        "[musical-pr] 运行时树: 节点 {}; `musical-pr-proposal-*-item` {} 条 {proposal_ids:?}; \
         假字面量命中 {} 处 {leaks:?}; `musical-pr-accept-button` 在树里 = {}; \
         `musical-pr-reject-button` 在树里 = {}",
        after.len(),
        proposal_ids.len(),
        leaks.len(),
        after.contains("musical-pr-accept-button"),
        after.contains("musical-pr-reject-button"),
    ));
    assert!(
        leaks.is_empty(),
        "抽屉里出现了编造的提案数据（文案或置信度）:\n  {}",
        leaks.join("\n  ")
    );
    assert!(
        proposal_ids.is_empty(),
        "零提案时不该有任何提案条目, 实测 {proposal_ids:?}"
    );
    // 一个可点的「采纳」在空列表上是**假控件**（按下去只关抽屉 + 一行 stderr）。
    // 上一轮已裁定 `Shift+Enter → 采纳 AI 建议` 不许假装实现, 这里同款。
    assert!(
        !after.contains("musical-pr-accept-button"),
        "零提案时「采纳」按钮不许在树里（它是假控件）"
    );
    assert!(
        !after.contains("musical-pr-reject-button"),
        "零提案时「放弃」按钮不许在树里（它是假控件）"
    );

    // ---- 空态: 用户此刻看到的那句话, 逐字读回来
    let empty = after
        .find_by_id("musical-pr-empty-state")
        .expect("零提案时必须有空态节点");
    report_line(&format!(
        "[musical-pr] 运行时树文本: `musical-pr-empty-state` role={} label={:?}",
        empty.role.as_str(),
        empty.label
    ));
    assert_eq!(
        empty.label, "还没有 AI 提案。这条链路尚未接线。",
        "空态的 `accessible-label` 必须就是界面上那句话"
    );

    // ---- 像素: 打开抽屉必须真的改变画面（同一次运行的两帧, 不碰 golden）
    let after_frame = ui.capture().expect("抽屉打开帧");
    let diff = frame_diff(&before_frame, &after_frame).expect("打开抽屉必须改变像素");
    report_line(&format!(
        "[musical-pr] 像素: 默认帧 vs 抽屉打开帧 = 差异 {} px, 包围盒 x{}..{} × y{}..{}（帧 {}×{}）",
        diff.count,
        diff.bbox.x,
        diff.bbox.right(),
        diff.bbox.y,
        diff.bbox.bottom(),
        before_frame.width(),
        before_frame.height(),
    ));
    assert!(
        diff.count > 0,
        "打开抽屉之后画面必须改变（否则抽屉没有被渲染）"
    );
    // 抽屉是 `x: root.width - 360px` 的右侧覆盖层（`ui/app.slint`）⇒ 差异必须伸进
    // 右起 360px 那一条带。这一格防的是"差异来自别处、抽屉其实没画出来"。
    let drawer_band_left = i32::try_from(after_frame.width().saturating_sub(360)).unwrap_or(0);
    assert!(
        diff.bbox.right() > drawer_band_left,
        "差异包围盒右界 {} 没有伸进抽屉那条带（x ≥ {drawer_band_left}）⇒ 画的不是抽屉",
        diff.bbox.right()
    );
}

// =====================================================================================
// 判据（诚实性审计 2026-10-08）：时光机画**真实提交链**；AI 徽章不再声称假状态
//
// 两条判据的"文本层 + 写者层"那一半住在新文件 `tests/undo_tree_honesty.rs`（本机不
// 渲染 Slint 也能跑）。这里只做运行时那一半：控件树 + 属性 + 真实点击 + 像素。
// =====================================================================================

/// 探针：运行时树的 `accessible-label` / `accessible-value` 里一个假历史节点名都没有。
///
/// 返回命中的字面量（空 = 干净）。**两个字段都探** —— 把假标签从 `label` 挪到
/// `value` 是一样糟的假数据。
fn demo_undo_literal_hits(tree: &ControlTree) -> Vec<String> {
    let mut hits = Vec::new();
    for node in tree.iter() {
        for literal in yeban_app::elements::DEMO_UNDO_TREE_LITERALS {
            if node.label.contains(literal) {
                hits.push(format!(
                    "`{}` 的 accessible-label 命中 `{literal}`",
                    node.id
                ));
            }
            if node
                .value
                .as_deref()
                .is_some_and(|value| value.contains(literal))
            {
                hits.push(format!(
                    "`{}` 的 accessible-value 命中 `{literal}`",
                    node.id
                ));
            }
        }
    }
    hits
}

/// 判据（**甲：真接**）：时光机画的节点 = **权威提交图谱**里的真实提交，一条不多一条不少。
///
/// ## 端到端链（每一步都是字面读数）
///
/// | 步 | 动作 | 断言 |
/// | :--- | :--- | :--- |
/// | ① | 打开真实 `UndoSession`（根提交消息 = `open <label>`）→ 装配活界面 → `host::apply_undo` | `undo-node-labels` == `["open <判据>"]`（**1** 条，不是 6 条）；`undo-node-count` == **1** |
/// | ② | 打开时光机 → 读运行时树 | 树里**有** `undo-tree-node-0`，其 `accessible-label` **逐字**等于那条真实提交消息；`undo-tree-node-1` **不在**树里；空态节点不在树里 |
/// | ③ | 经端口再提交 **7** 次真 op | 链条长 **8** ⇒ 画出最近 **6** 个 + `undo-node-hidden` == **2**；`undo-tree-node-6` 不在树里；截断报数节点在树里，其标签里的数字 == `2` |
/// | ④ | 假字面量探针 | 运行时树的 `label` / `value` 一处都不命中 [`DEMO_UNDO_TREE_LITERALS`] |
///
/// 为什么"节点数 == 图谱读数"是**算术**而不是巧合：`node-count` 由
/// `chain.len().min(UNDO_TREE_MAX_NODES)` 得到，`node-hidden` 由同一长度的
/// `saturating_sub` 得到 ⇒ 两个读数的和必须**恰好**等于 `CommitGraph::ancestry` 的长度。
#[test]
fn the_undo_tree_shows_the_real_commit_chain() {
    use std::rc::Rc;

    // `ModelRc::iter`（读 `[string]` 属性）来自 `slint::Model`（本文件既有的写法）。
    use slint::Model as _;
    use yeban_app::undo::undo_session::wiring_fixture;
    use yeban_app::undo::{UndoPort, UndoSession};

    /// 会话打开时刻（与既有判据同一个夹具常量）。
    const UNDO_TREE_NOW: u64 = 1_760_000_000_000;

    let project = yeban_model::samples::filled_project();
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<判据>", "yeban-app", project.clone(), UNDO_TREE_NOW).expect("打开"),
    ));
    // 根提交的原话（`UndoSession::open` 写的是 `format!("open {label}")`）。
    let genesis = port
        .graph()
        .commits
        .values()
        .map(|commit| commit.message.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        genesis,
        vec!["open <判据>".to_owned()],
        "新会话必须恰好有一条根提交，消息逐字可预期"
    );

    let mut ui = build_live_ui_with(
        &project,
        &LiveWiringOptions {
            permission: Permission::ReadOnly,
            console_tab: 0,
            save_path: None,
            engine_quanta: 0,
            undo: Some(Rc::clone(&port)),
        },
    )
    .expect("装配");
    // 生产 GUI 的注入点（`src/main.rs` 的 `host::apply_undo(&ui, &undo_port)`）。
    yeban_app::host::apply_undo(ui.ui(), &port);

    // ---- ① 属性：真实链条（1 条），不是编造的 6 条 ----
    let window = slint::ComponentHandle::clone_strong(ui.ui());
    let labels_1: Vec<String> = window
        .get_undo_node_labels()
        .iter()
        .map(|label| label.to_string())
        .collect();
    report_line(&format!(
        "[undo-tree] ① 图谱读数: `ancestry(head)` 长度 {} / 属性 `undo-node-labels` {:?} / \
         `undo-node-count` {} / `undo-node-hidden` {}",
        genesis.len(),
        labels_1,
        window.get_undo_node_count(),
        window.get_undo_node_hidden(),
    ));
    assert_eq!(
        labels_1,
        vec!["open <判据>".to_owned()],
        "属性里的节点标签必须**逐字**等于根提交的消息"
    );
    assert_eq!(window.get_undo_node_count(), 1, "链长 1 ⇒ 画 1 个节点");
    assert_eq!(window.get_undo_node_hidden(), 0, "链长 1 ⇒ 没有截断");

    // ---- ② 打开时光机 ⇒ 运行时树 ----
    // ⚠ 必须走**端口**打开：`undo-tree-open` 的唯一写者是 `host::apply_undo`
    // （它每一跳都从 `port.undo_tree_open()` 回写）。直接 `window.set_undo_tree_open(true)`
    // 会在下一次 `apply_undo` 时被宿主的读数**覆盖回 false** —— 这正是"唯一写者"的证据。
    port.perform(yeban_app::undo::UiAction::OpenUndoTree);
    yeban_app::host::apply_undo(ui.ui(), &port);
    ui.pump_meters();
    let tree_1 = ui.tree_snapshot();
    assert!(
        tree_1.contains("undo-tree-modal"),
        "`undo-tree-open=true` ⇒ 时光机进树"
    );
    let node_0 = tree_1
        .find_by_id("undo-tree-node-0")
        .expect("真实链有 1 条提交 ⇒ 必须有 `undo-tree-node-0`");
    report_line(&format!(
        "[undo-tree] ② 运行时树: 节点 {}; `undo-tree-node-0` role={} label={:?}; \
         `undo-tree-node-1` 在树里 = {}; 空态在树里 = {}; 截断报数在树里 = {}",
        tree_1.len(),
        node_0.role.as_str(),
        node_0.label,
        tree_1.contains("undo-tree-node-1"),
        tree_1.contains("undo-tree-empty-state"),
        tree_1.contains("undo-tree-truncation-note"),
    ));
    assert_eq!(
        node_0.label, "open <判据>",
        "树的 `accessible-label` 必须**逐字**等于真实提交消息（编造的版本名会在这里红）"
    );
    assert!(
        !tree_1.contains("undo-tree-node-1"),
        "链上只有 1 条提交 ⇒ 第 2 个节点**不许**存在（改动前这里画着 6 个编造节点）"
    );
    assert!(
        !tree_1.contains("undo-tree-empty-state"),
        "链非空 ⇒ 空态不许在树里"
    );
    assert!(
        !tree_1.contains("undo-tree-truncation-note"),
        "链比画布短 ⇒ 截断报数不许在树里"
    );
    let hits_1 = demo_undo_literal_hits(&tree_1);
    assert!(
        hits_1.is_empty(),
        "运行时树里读到了编造的版本节点名:\n  {}",
        hits_1.join("\n  ")
    );

    // ---- ③ 再提交 7 次 ⇒ 链条 8 条，画布只放得下 6 个 ----
    // 每次提交的 `old_vel` 必须是**上一步写进去的值**（模型校验前后态），因此这里
    // 逐次滚动地构造 7 条互不相同的真 op —— 不是把同一条 op 重复 7 次。
    let fixture = wiring_fixture(&port.project()).expect("夹具：工程里必须有可改的音符");
    let mut old_velocity = fixture.old_velocity;
    for step in 0..7_u32 {
        let new_velocity = u8::try_from(40 + step).expect("力度落在 u8 内");
        port.commit_ops(
            UNDO_TREE_NOW + u64::from(step) + 1,
            &format!("审计夹具 {step}"),
            vec![yeban_model::Op::ModifyNoteVelocity {
                track_id: fixture.track_id,
                clip_id: fixture.clip_id,
                note_id: fixture.note_id,
                old_vel: old_velocity,
                new_vel: new_velocity,
            }],
        )
        .expect("提交必须被接受");
        old_velocity = new_velocity;
    }
    yeban_app::host::apply_undo(ui.ui(), &port);
    let chain_len = port
        .graph()
        .ancestry(&port.display().head.expect("有活跃分支头"))
        .expect("祖先链")
        .len();
    let hidden = window.get_undo_node_hidden();
    let rendered = window.get_undo_node_count();
    let labels_8: Vec<String> = window
        .get_undo_node_labels()
        .iter()
        .map(|label| label.to_string())
        .collect();
    report_line(&format!(
        "[undo-tree] ③ 提交 7 次之后: `ancestry` 长度 {chain_len} = 画出 {rendered} + 截断 {hidden} \
         （算术: {rendered} + {hidden} = {}）; 最近的标签 {:?}; 最旧的标签 {:?}",
        rendered + hidden,
        labels_8.first(),
        labels_8.last(),
    ));
    assert_eq!(chain_len, 8, "1 条根提交 + 7 次提交 = 8");
    assert_eq!(rendered, 6, "画布只放得下 6 个节点");
    assert_eq!(hidden, 2, "8 - 6 = 2 个更早的版本被截断");
    assert_eq!(
        i32::try_from(chain_len).expect("链长") - rendered,
        hidden,
        "两个读数必须与链条长度闭合（这就是端到端的算术）"
    );
    assert_eq!(
        labels_8.first().map(String::as_str),
        Some("审计夹具 6"),
        "最新在前：下标 0 必须是最后一次提交的消息"
    );
    assert_eq!(
        labels_8.last().map(String::as_str),
        Some("审计夹具 1"),
        "画出的 6 个是**最近**的 6 个"
    );

    ui.pump_meters();
    let tree_8 = ui.tree_snapshot();
    let undo_ids: Vec<&str> = tree_8
        .ids()
        .filter(|id| id.starts_with("undo-tree-"))
        .collect();
    report_line(&format!(
        "[undo-tree] ③ 树里的 `undo-tree-*` 节点（{} 条）: {undo_ids:?}",
        undo_ids.len()
    ));
    let note = tree_8
        .find_by_id("undo-tree-truncation-note")
        .expect("链条比画布长 ⇒ 必须有截断报数节点");
    report_line(&format!(
        "[undo-tree] ③ 运行时树: 节点 {}; `undo-tree-node-6` 在树里 = {}; \
         截断报数 label={:?}",
        tree_8.len(),
        tree_8.contains("undo-tree-node-6"),
        note.label,
    ));
    assert!(
        !tree_8.contains("undo-tree-node-6"),
        "画布画不下第 7 个节点 ⇒ `undo-tree-node-6` 不许在树里（注册表也只登记 0..6）"
    );
    assert_eq!(
        tree_8.find_by_id("undo-tree-node-0").expect("头节点").label,
        "审计夹具 6",
        "树里的头节点标签必须是真实提交消息"
    );
    assert!(
        note.label == "更早的 2 个版本未画出",
        "截断报数必须带**真实**的截断条数（实测 2），实际 {note:?}"
    );
    let hits_8 = demo_undo_literal_hits(&tree_8);
    assert!(
        hits_8.is_empty(),
        "运行时树里读到了编造的版本节点名:\n  {}",
        hits_8.join("\n  ")
    );
}

/// 判据（**乙：如实表达"没有"**）：没有宿主写者时，时光机画**空态**，不画任何节点。
///
/// 这一条装配**没有**撤销端口（`LiveWiringOptions::undo = None`）—— `host::wire_undo` /
/// `host::apply_undo` 因此从未跑过 ⇒ 界面拿到的就是 `node-count` / `node-labels` /
/// `node-hidden` 的**属性默认值**（`0` / `[]` / `0`）。此时打开时光机，运行时树里必须有
/// `undo-tree-empty-state`，且**一个** `undo-tree-node-*` 都没有。
///
/// 改动前这里画着 **6** 个编造的版本名（含 `"c5 AI 提案"`）—— 那是**没有任何写者**的
/// 属性默认值，也就是用户看到的东西。
///
/// ## 像素
///
/// 命题①（**默认帧**，时光机关着）的读数是 PNG 字节的 sha256（本仓 PNG 是存储式
/// deflate，1920×1080 恒为 6,222,418 字节 ⇒ 尺寸证明不了内容）。命题②（打开时光机之后）
/// 是同一帧的差异像素数与包围盒 —— 两个命题分开陈述，不可互相代替。
#[test]
fn unwritten_undo_tree_shows_an_honest_empty_state_at_runtime() {
    // ---- 没有写者 ⇒ 属性停在默认值 ----
    let project = yeban_model::samples::filled_project();
    let mut ui = build_live_ui(&project, Permission::ReadOnly).expect("装配（无撤销端口）");
    let window = slint::ComponentHandle::clone_strong(ui.ui());

    // ---- 像素命题①：**默认帧**（时光机关着）的读数 ----
    let frame_closed = ui.capture().expect("默认帧");
    let (closed_png, _closed_evidence) =
        encode_with_evidence(&frame_closed, DEFAULT_MAX_PNG_BYTES).expect("默认帧必须可编码");
    let closed_digest = yeban_model::ids::ContentHash::of_bytes(&closed_png);
    let closed_artifact = write_artifact_bytes("undo-tree-closed-1920x1080.png", &closed_png);
    report_line(&format!(
        "[undo-tree-pixel] 命题①默认帧（时光机关着）: {} 字节 / sha256={} / artifact={closed_artifact}",
        closed_png.len(),
        closed_digest.as_str(),
    ));

    // 没有撤销端口 ⇒ 没有任何宿主写者会回写 `undo-tree-open`，因此这里直接写属性。
    window.set_undo_tree_open(true);
    ui.pump_meters();
    let tree = ui.tree_snapshot();
    let node_ids: Vec<&str> = tree
        .ids()
        .filter(|id| id.starts_with("undo-tree-node-"))
        .collect();
    let empty = tree
        .find_by_id("undo-tree-empty-state")
        .expect("没有宿主写者时必须有空态节点");
    report_line(&format!(
        "[undo-tree] 空态: `undo-tree-node-*` {} 条 {node_ids:?}; \
         `undo-tree-empty-state` role={} label={:?}; 节点总数 {}",
        node_ids.len(),
        empty.role.as_str(),
        empty.label,
        tree.len(),
    ));
    assert!(
        node_ids.is_empty(),
        "没有宿主写者时一个图谱节点都不许出现（改动前这里画着 6 个编造版本），实际 {node_ids:?}"
    );
    assert_eq!(
        empty.label, "还没有任何版本提交。撤销历史为空。",
        "空态的 `accessible-label` 必须就是界面上那句话"
    );
    let hits = demo_undo_literal_hits(&tree);
    assert!(
        hits.is_empty(),
        "运行时树里读到了编造的版本节点名:\n  {}",
        hits.join("\n  ")
    );

    // ---- 像素命题②：打开时光机之后画面必须真的变（与命题①的默认帧对照）----
    let frame_open = ui.capture().expect("时光机打开帧");
    let diff = frame_diff(&frame_closed, &frame_open).expect("打开时光机必须改变像素");
    report_line(&format!(
        "[undo-tree-pixel] 命题②: 差异 {} px, 包围盒 x{}..{} × y{}..{}（帧 {}×{}）",
        diff.count,
        diff.bbox.x,
        diff.bbox.right(),
        diff.bbox.y,
        diff.bbox.bottom(),
        frame_closed.width(),
        frame_closed.height(),
    ));
    assert!(
        diff.count > 0,
        "打开时光机之后画面必须改变（否则弹窗没有被渲染）"
    );
}

/// 判据（**乙的运行时那一半**）：AI 徽章不再声称"有 1 条待审查提案"，且它**真的能点**。
///
/// 改动前这一处是三连假：`accessible-role: button` 没有点击源；`accessible-value: "1"`
/// 与可见文案「AI提案 (待审查)」声称有 1 条待审查提案，而**没有任何宿主写者**
/// （`git grep -n 'ai-proposal-badge'` 只命中语义注册表与测试）。三件事各自独立：
///
/// 1. 运行时树的 `accessible-label` 是**动作**描述（不是状态声明），`accessible-value`
///    **不存在**（`null`，改前是 `"1"`）；
/// 2. `label` / `value` 都不含假状态字面量「待审查」；
/// 3. 一次**真实指针点击**（按下 → 移动 → 松手 → Slint 命中测试 → `.slint` 的 `TouchArea`
///    → `open-ai-proposals` → `app.slint` → `open-musical-pr` → 宿主写 `musical-pr-open`）
///    真的打开提案抽屉 —— 因此 `accessible-role: button` 不再是空承诺。
#[test]
fn the_ai_badge_has_a_real_action_and_no_fake_state_at_runtime() {
    use serde_json::json;

    /// 徽章上被删掉的假状态字面量（与 `tests/undo_tree_honesty.rs` 同一批）。
    const BADGE_FALSE_LITERALS: [&str; 1] = ["待审查"];

    // 这一条用 Interactive 权限装配一条**真实指针注入**的面（与 `click_element` 同一手法）。
    let (window, mut plane) = assemble_view_state_callbacks();
    let badge = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        900,
        yeban_ui_mcp::methods::METHOD_NODE,
        Some(json!({ "elementId": "transport-ai-proposal-badge" })),
    ));
    assert!(!badge.is_error(), "徽章必须在运行时树里: {badge:?}");
    let badge_node = badge.result.expect("有 result")["node"].clone();
    report_line(&format!(
        "[ai-badge] 运行时树: id={} role={} label={:?} value={}",
        badge_node["id"], badge_node["role"], badge_node["label"], badge_node["value"],
    ));
    let label = badge_node["label"].as_str().unwrap_or_default();
    assert!(
        !BADGE_FALSE_LITERALS
            .iter()
            .any(|literal| label.contains(literal)),
        "徽章的 `accessible-label` 仍在声称假状态: {label:?}"
    );
    assert_eq!(
        label, "打开 AI 编曲提案审核抽屉",
        "徽章的标签必须是**动作描述**（它现在真的能点）"
    );
    assert!(
        badge_node["value"].is_null(),
        "徽章**不许**声明 `accessible-value`（改前是编造的 `\"1\"`），实际 {}",
        badge_node["value"]
    );

    // 真实点击 ⇒ 抽屉真的打开（一次交互 = 一次明确动作）。
    assert!(!window.get_musical_pr_open(), "默认不打开提案抽屉");
    click_element(&mut plane, 910, "transport-ai-proposal-badge");
    assert!(
        window.get_musical_pr_open(),
        "点徽章必须真的打开提案抽屉（`open-ai-proposals` → `open-musical-pr`）"
    );
    let opened = plane.plane().tree().expect("ui/tree").0;
    report_line(&format!(
        "[ai-badge] 真实点击之后: musical-pr-open={} / 抽屉在树里 = {}",
        window.get_musical_pr_open(),
        opened.find("musical-pr-drawer").is_some(),
    ));
    assert!(
        opened.find("musical-pr-drawer").is_some(),
        "点击徽章之后提案抽屉必须进树"
    );
}

// =====================================================================================
// 判据：钢琴卷帘的工具矩阵**点得动**（台账 R16）
// =====================================================================================

/// 判据：五个工具按钮各自被一次**真实指针注入**点中之后，`active-tool` 必须变成那一行的行号。
///
/// ## 缺陷的形状（为什么这条判据有判别力）
///
/// 改动前 `ui/console/piano_roll.slint` 的五个工具按钮是纯 `Rectangle` + `UiText`：
/// 声明了 `accessible-role: button` 与 `accessible-checked`，但**没有** `TouchArea`、
/// 也**没有**回调 ⇒ 用户在界面上点它**没有任何反应**，`active-tool` 只由键盘写。
/// 本判据走的是 §12.4 的真实指针注入（按下 → 移动 → 松手 → Slint 命中测试 →
/// `.slint` 的输入面 → 回调 → 宿主 `Action::SelectTool` 臂写属性），
/// 与"用户点那一下"是同一条链。因此把 `TouchArea` 换回 `Rectangle`（或删掉那条回调）
/// 一定会红 —— 那时点击落到卷帘的滚动手势面上，`active-tool` 一位不变。
///
/// ## 为什么五个都点（而不是只点一个）
///
/// 只点一个的话，"行号 → 工具"的换算错了（例如 `tool_index` 与 `tool_index + 1` 混用）
/// 有一半的组合看不出来。五行逐个点 ⇒ 行号口径与 `Tool::digit`（`1`..`5`）的对账是完整的。
#[test]
fn the_piano_roll_tool_buttons_change_the_active_tool_at_runtime() {
    let (window, mut plane) = assemble_view_state_callbacks();

    // 面板的默认标签必须是卷帘 —— 否则工具按钮 `visible: false`，注入会如实地在树里找不到它。
    assert_eq!(
        window.get_console_tab(),
        0,
        "默认底部标签必须是卷帘（工具按钮只在那时可见）"
    );
    assert_eq!(window.get_active_tool(), 1, "默认工具是选择（矩阵第 1 行）");

    let before = (window.get_arrangement_view(), window.get_musical_pr_open());

    let mut read_back: Vec<(&str, i32)> = Vec::new();
    for (row, name) in yeban_app::scene::TOOL_NAMES.iter().enumerate() {
        let tool_digit = i32::try_from(row).expect("行号") + 1;
        let element_id = format!("piano-roll-tool-{name}-button");
        click_element(
            &mut plane,
            970 + 10 * i64::try_from(row).expect("行号"),
            &element_id,
        );
        let observed = window.get_active_tool();
        read_back.push((name, observed));
        assert_eq!(
            observed, tool_digit,
            "点 `{element_id}` 之后 `active-tool` 必须是矩阵第 {tool_digit} 行，实际 {observed}"
        );
    }

    let after = (window.get_arrangement_view(), window.get_musical_pr_open());
    report_line(&format!(
        "[piano-roll-tools] 真实点击五行 ⇒ active-tool 读数 {read_back:?}；\
         工具选择**不该**动别的视图态: 双视图 / 抽屉 改前 {before:?} → 改后 {after:?}"
    ));
    assert_eq!(
        after, before,
        "工具选择是**视图状态**：它只改 `active-tool`，不得顺手改双视图 / 抽屉"
    );
}

// =====================================================================================
// 判据：在**编曲视图**点静音 ⇒ 撤销树的节点标签必须如实（台账 R17）
// =====================================================================================

/// 判据：编曲视图轨道头的静音开关点一下之后，撤销树里多出来的那条提交消息**不得**声称
/// 动作来自"混音台"。
///
/// ## 缺陷的形状（为什么这条判据有判别力）
///
/// 编曲视图的轨道头开关与调音台通道条的开关**共用同一个**宿主面
/// （`ui/workspace/arrangement_view.slint` 的 `track-mute-toggle` → `app.slint` →
/// `MainWindow.mixer-mute-toggle` → `host::wire_mixer_switch`）。那条 `commit_ops` 的
/// `message` 就是撤销树的**节点标签**（`Commit::message`）。改动前它写死
/// `"mixer: toggle track mute"` ⇒ 用户在编曲视图点一下，历史里却写着"混音台"。
///
/// 本判据走的是 §12.4 的真实指针注入（按下 → 移动 → 松手 → Slint 命中测试 → `.slint`
/// 的 `TouchArea` → 回调 → `Op::SetTrackMute` → `commit_ops`），因此它读到的标签是**真的
/// 那一条**，不是判据自己拼的字符串。
///
/// ## 为什么同时断言"工程真的变了"
///
/// 只看标签的话，"点击根本没生效"也能让标签断言通过（历史里那条消息还是旧提交的）。
/// 工程侧读数（模型里的 `TrackV3::mute`）证明这一击真的落了地 ⇒ 标签确实是这一击写下的。
#[test]
fn a_mute_click_in_the_arrangement_view_writes_an_honest_history_label() {
    use std::rc::Rc;

    use yeban_app::undo::{UndoPort, UndoSession};

    /// 会话打开时刻（与既有判据同一个夹具常量）。
    const NOW: u64 = 1_760_000_000_000;

    /// 视图名的黑名单：标签里出现它们就是把"哪个视图"写进了历史（本缺陷的字面形态）。
    const VIEW_NAMES: [&str; 5] = ["mixer", "arrangement", "session", "console", "workspace"];

    let project = yeban_model::samples::filled_project();
    let view = ViewState::from_project(&project).expect("投影");
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<判据:R17 编曲静音>", "yeban-app", project.clone(), NOW).expect("打开"),
    ));
    // 装配带撤销端口的活界面：`host::wire_mixer_switch` 是**生产形态**的接线
    // （`build_live_ui_with` 的 `options.undo` 交给 `host::wire_mixer_edit`）。
    let live = build_live_ui_with(
        &project,
        &LiveWiringOptions {
            permission: Permission::Interactive,
            console_tab: 0,
            save_path: None,
            engine_quanta: 0,
            undo: Some(Rc::clone(&port)),
        },
    )
    .expect("装配");
    let window = slint::ComponentHandle::clone_strong(live.ui());
    // 切到编曲视图（与 `Tab` / `F6` 写的是**同一个**属性）：不切的话轨道头不在树里。
    window.set_arrangement_view(true);
    let mut plane = live.into_control_plane(Permission::Interactive);

    let before = mixer_readings(&port.project(), &view, 0);
    assert!(!before.2, "0 号轨起点不静音（与既有混音判据同一个夹具）");

    click_element(&mut plane, 990, "track-0-mute-button");

    let after = mixer_readings(&port.project(), &view, 0);
    assert!(
        after.2,
        "编曲视图点静音之后模型里的 `mute` 必须真的是 true（否则标签断言没有意义）"
    );
    let last = port
        .graph()
        .commits
        .values()
        .next_back()
        .expect("点击之后图谱必须有提交")
        .message
        .clone();
    report_line(&format!(
        "[r17-history-label] 编曲视图点静音: mute {} → {}；最新提交消息 = {last:?}",
        before.2, after.2
    ));
    for name in VIEW_NAMES {
        assert!(
            !last.contains(&format!("{name}:")),
            "撤销树的节点标签不得声称动作来自 `{name}` —— 这一击来自**编曲视图**，\
             而宿主面与调音台共用（R17），实际标签 {last:?}"
        );
    }
    assert!(
        last.contains("mute"),
        "标签必须带上它真正改的字段名 `mute`（中性不等于含糊），实际 {last:?}"
    );
}

// =====================================================================================
// 判据：**读方法看见当下的界面**（`ui/tree` / `ui/node` / `ui/property` 的新鲜度一致）
//
// 背景：`ui/tree` / `ui/node` 读的是执行面持有的**运行时树缓存**，而 `ui/property`
// 与 `ui/screenshot` 读的是**活窗口**。两种读法的"新鲜度"因此不一致 ⇒
// 「人点一下按钮、AI 立刻读 `ui/tree`」看不见那一击。本判据把这条链钉住：
// **注入前**的读数（那时刻树是旧的）与**点击后**的读数必须一起报出来。
// =====================================================================================

/// 判据：一次**真实点击**之后，下一次 `ui/tree` / `ui/node` / `ui/property` 必须看见它。
///
/// | 段 | 做什么 | 期望 |
/// | :--- | :--- | :--- |
/// | ① 改前 | 注入前读 `ui/property {"sidebar","width"}` / `ui/tree` / `ui/node` | `"240.00"`；树里**有** `sidebar-search-field`；折叠按钮 `bounds.x = 210` |
/// | ② 点击 | `ui/dispatch_pointer_*` 点 `sidebar-collapse-button` | 活窗口 `sidebar-collapsed=false→true`（属性读数证明那一击真的生效） |
/// | ③ 改后 | 读同样三件 | `"36.00"`；树里**没有** `sidebar-search-field`；折叠按钮 `bounds.x = 6` |
///
/// ## 判别力从哪来（为什么不是"我写进去所以我说它对"）
///
/// ① 与 ③ 的三件读数全部**经控制面**取回（`ui/property` / `ui/tree` / `ui/node`），
/// 没有任何一处直接读 `MainWindow` 的 Rust getter。② 走的是 §12.4 的真实指针注入
/// （按下 → 移动 → 松手 → Slint 命中测试 → `.slint` 的 `TouchArea` → 回调 → 宿主写属性），
/// 与用户点那一下是同一条链。
#[test]
fn a_tree_read_sees_a_click_that_just_happened() {
    use yeban_ui_mcp::methods::METHOD_TREE;

    let (window, mut plane) = assemble_view_state_callbacks();

    /// `ui/tree` 一次读的线上节点数 + 是否含某个语义 ID + 这次调用的墙钟（毫秒）。
    ///
    /// ⚠ 耗时是**代理指标**：它数的是"一次 `ui/tree` 请求从服务层进到出"的墙钟，
    /// 含 JSON 编码与请求包装，**不是**纯重抓耗时。
    fn tree_read(plane: &mut LiveControlPlane, seq: i64, needle: &str) -> (usize, bool, f64) {
        let started = std::time::Instant::now();
        let line =
            plane
                .plane()
                .try_line(&yeban_ui_mcp::live::request_line(seq, METHOD_TREE, None));
        let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
        assert!(!line.is_error(), "`ui/tree` 必须成功: {line:?}");
        let result = line.result.expect("有 result");
        let count = result["filter"]["totalCount"].as_u64().expect("totalCount") as usize;
        let json = result["tree"].to_string();
        (count, json.contains(needle), elapsed_ms)
    }

    /// `ui/node` 的一次读数（几何包围盒，经控制面）。
    fn node_bounds(plane: &mut LiveControlPlane, seq: i64, id: &str) -> serde_json::Value {
        let node = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
            seq,
            "ui/node",
            Some(serde_json::json!({ "elementId": id })),
        ));
        assert!(!node.is_error(), "`ui/node {id}` 必须成功: {node:?}");
        node.result.expect("有 result")["node"]["bounds"].clone()
    }

    let needle = "sidebar-search-field";

    // ---------------------------------------------------------------- ① 改前
    let width_before = read_property(&mut plane, 400, "sidebar", "width");
    let (nodes_before, has_before, ms_before) = tree_read(&mut plane, 401, needle);
    let collapse_before = node_bounds(&mut plane, 402, "sidebar-collapse-button");
    assert_eq!(
        width_before.as_str(),
        Some("240.00"),
        "默认左资源栏宽 = `Tokens.left-rail-width`"
    );
    assert!(
        has_before,
        "注入前 `{needle}` 必须在树里（否则本判据的对照不成立）"
    );

    // ---------------------------------------------------------------- ② 真实点击
    click_element(&mut plane, 410, "sidebar-collapse-button");
    let collapsed = window.get_sidebar_collapsed();
    assert!(collapsed, "② 点击必须把 `sidebar-collapsed` 翻成 true");

    // ---------------------------------------------------------------- ③ 改后
    let width_after = read_property(&mut plane, 420, "sidebar", "width");
    let (nodes_after, has_after, ms_after) = tree_read(&mut plane, 421, needle);
    let collapse_after = node_bounds(&mut plane, 422, "sidebar-collapse-button");

    report_line(&format!(
        "[fresh-read] `toggle-sidebar` 点击: `ui/property sidebar.width` {width_before}→{width_after}; \
         `ui/tree` 节点 {nodes_before}→{nodes_after}、含 `{needle}` {has_before}→{has_after}; \
         `ui/node sidebar-collapse-button` x {}→{}; 一次 `ui/tree` 墙钟 {ms_before:.2}ms→{ms_after:.2}ms（代理）",
        collapse_before["x"], collapse_after["x"]
    ));

    assert_eq!(
        width_after.as_str(),
        Some("36.00"),
        "属性读数必须跟着点击走（`ui/property` 与 `ui/tree` 的新鲜度不许分叉）"
    );
    assert!(
        !has_after,
        "点击之后 `{needle}` 必须离开运行时树（`visible: !compact`）—— \
         它还在树里就说明这一次 `ui/tree` 读的是**旧快照**，看不见刚发生的变化"
    );
    assert!(
        collapse_after["x"].as_f64() < collapse_before["x"].as_f64(),
        "折叠按钮必须跟着左栏变窄：x {} → {}",
        collapse_before["x"],
        collapse_after["x"]
    );

    // ---------------------------------------------------------------- ④ 反证：再点一次必须回去
    //
    // 少了这一段，③ 只证明"某个方向变了"，证明不了"读的是**当下**"——
    // 一个恒读旧快照的实现也会让 ③ 的读数停在另一侧。反向点击把这条堵死。
    click_element(&mut plane, 430, "sidebar-collapse-button");
    assert!(
        !window.get_sidebar_collapsed(),
        "④ 第二次点击必须把 `sidebar-collapsed` 翻回 false"
    );
    let width_back = read_property(&mut plane, 440, "sidebar", "width");
    let (nodes_back, has_back, _) = tree_read(&mut plane, 441, needle);
    let collapse_back = node_bounds(&mut plane, 442, "sidebar-collapse-button");
    report_line(&format!(
        "[fresh-read] 反证（展开回去）: `ui/property sidebar.width` {width_after}→{width_back}; \
         `ui/tree` 节点 {nodes_after}→{nodes_back}、含 `{needle}` {has_after}→{has_back}; \
         `ui/node sidebar-collapse-button` x {}→{}",
        collapse_after["x"], collapse_back["x"]
    ));
    assert_eq!(width_back.as_str(), Some("240.00"));
    assert!(has_back, "展开之后 `{needle}` 必须回到运行时树里");
    assert_eq!(nodes_back, nodes_before, "展开回去 ⇒ 节点数回到起点");
    assert_eq!(
        collapse_back["x"], collapse_before["x"],
        "展开回去 ⇒ 折叠按钮回到原来的 x"
    );
}

/// 判据：**读元数据不得渲染一帧**（`ui/tree` / `ui/node` / `ui/property` /
/// `ui/dynamic_regions` / `ui/coverage`），而 `ui/screenshot` **恰好**渲染一帧。
///
/// ## 这条判据在防什么（不是防"读变慢"，是防"读悄悄开始渲染"）
///
/// `refresh_tree` 曾经（`bd9fbb6`）在重抓前先 `capture()` 一次 —— 那是 1920×1080 的
/// Tier-1 全量软件光栅化。读路径现在**每次读**都重抓，因此那一步一旦被加回来，
/// 每一次 `ui/tree` 都会白渲染一张没人看的 6 MB 帧，而**没有任何断言会红**。
/// 本判据把这条不变量钉住：探针是 `Tier1Window::rendered_frames()`
/// （窗口上真的跑过渲染回调的次数），判据读的是 [`LiveControlPlane::rendered_frames`]。
///
/// ⚠ 断言**不是**计时（墙钟随机器浮动，不适合做判据）：它数的是**事件次数**，
/// 因此是确定性的。
#[test]
fn a_tree_read_does_not_rasterize_a_frame() {
    use serde_json::json;
    use yeban_ui_mcp::methods::{METHOD_NODE, METHOD_PROPERTY, METHOD_TREE};

    let (_window, mut plane) = assemble_view_state_callbacks();
    let baseline = plane.rendered_frames();

    // ---- 五条读元数据的方法：一个像素都不该渲染 ----
    let reads: [(i64, &str, serde_json::Value); 5] = [
        (500, METHOD_TREE, json!({})),
        (501, METHOD_NODE, json!({"elementId": "sidebar"})),
        (
            502,
            METHOD_PROPERTY,
            json!({"elementId": "sidebar", "name": "width"}),
        ),
        (503, "ui/dynamic_regions", json!({})),
        (504, "ui/coverage", json!({"ids": ["sidebar"]})),
    ];
    for (seq, method, params) in reads {
        let line =
            plane
                .plane()
                .try_line(&yeban_ui_mcp::live::request_line(seq, method, Some(params)));
        assert!(!line.is_error(), "`{method}` 必须成功: {line:?}");
        assert_eq!(
            plane.rendered_frames(),
            baseline,
            "`{method}` 读的是**元数据** ⇒ 不得渲染一帧（渲染的是 `capture()`，它只属于 `ui/screenshot`）"
        );
    }

    // ---- 反证：`ui/screenshot` 必须**恰好**渲染一帧 ----
    let shot = plane.plane().try_line(&yeban_ui_mcp::live::request_line(
        510,
        "ui/screenshot",
        Some(json!({"maskDynamic": false})),
    ));
    assert!(!shot.is_error(), "`ui/screenshot` 必须成功: {shot:?}");
    assert_eq!(
        plane.rendered_frames(),
        baseline + 1,
        "`ui/screenshot` 必须恰好渲染一帧（多一帧 = 白渲染；少一帧 = 像素来源可疑）"
    );
}

/// 判据：**一次重抓的代价 = 一次全树内省，不是一次全量渲染**。
///
/// 这条判据存在的原因：`refresh_tree` 曾经（`bd9fbb6`）在重抓前先 `capture()` 一次，
/// 而读路径现在**每次读**都要重抓 ⇒ 那一步会把 1920×1080 的 Tier-1 全量软件光栅化
/// 挂在每一次 `ui/tree` 上。本判据把两个数字并排打出来，让"重抓很便宜、渲染很贵"
/// 这件事有可复跑的读数（**只报不判时间**：墙钟是机器相关的代理指标，不适合做断言；
/// 断言只落在"两条路都真的产出了东西"上）。
///
/// | 量什么 | 怎么量 | 单位 |
/// | :--- | :--- | :--- |
/// | Tier-1 全量渲染一帧 | `LiveUi::capture()` 一次（先预热一次剔除首次初始化） | ms（墙钟） |
/// | 重抓一次运行时树 | `LiveUi::pump_meters()` 一次（抽干 → 注入 → 全树内省） | ms（墙钟） |
/// | 树规模 | `tree_snapshot().len()` | 节点数 |
#[test]
fn a_tree_regrab_costs_one_introspection_not_one_full_render() {
    let project = yeban_model::samples::filled_project();
    let mut ui = build_live_ui(&project, Permission::ReadOnly).expect("装配");

    // 预热一次：`capture()` 的首次调用含平台/缓冲的初始化成本，混进去会污染对照。
    let warm = ui.capture().expect("预热帧必须成功");
    assert!(!warm.is_all_black(), "预热帧不能是全黑（[MUST-GATE-015]）");

    let before_capture = ui.rendered_frames();
    let started = std::time::Instant::now();
    let frame = ui.capture().expect("对照帧必须成功");
    let capture_ms = started.elapsed().as_secs_f64() * 1000.0;
    let after_capture = ui.rendered_frames();

    let started = std::time::Instant::now();
    let _ = ui.pump_meters();
    let regrab_ms = started.elapsed().as_secs_f64() * 1000.0;
    let after_regrab = ui.rendered_frames();

    let nodes = ui.tree_snapshot().len();
    report_line(&format!(
        "[refresh-cost] Tier-1 全量渲染一帧 `capture()` = {capture_ms:.2}ms（1920x1080 软件光栅化）; \
         重抓一次树 `pump_meters()` = {regrab_ms:.2}ms（含电平注入 + 全树内省, {nodes} 个节点）; \
         ⇒ 重抓比渲染一帧便宜 {:.2}ms; 探针读数：渲染帧数 {before_capture}→{after_capture}→{after_regrab}\
         （两次墙钟都是**代理**指标）",
        capture_ms - regrab_ms
    ));

    assert!(!frame.is_all_black(), "对照帧不能是全黑（[MUST-GATE-015]）");
    assert_eq!(
        after_capture,
        before_capture + 1,
        "`capture()` 必须恰好渲染一帧（探针数的是真的跑过渲染回调的次数）"
    );
    assert_eq!(
        after_regrab, after_capture,
        "重抓运行时树（全树内省）不得渲染一帧：capture() 之后 {after_capture} 帧，\
         pump_meters() 之后 {after_regrab} 帧"
    );
    assert!(nodes >= 40, "运行时控件树只有 {nodes} 个节点");
}

// ===========================================================================
// 横向缩放（`Z` / `Shift+Z`）：真实事件源 → 投影参数 → 像素
// ===========================================================================

/// 从**模型**算选区的 tick 跨度（`min start_tick`, `max start_tick + duration_ticks`）。
///
/// 这是**独立 oracle**：它走 `clip_pool` 的模型数据，不走 `ViewState`。
/// 于是"投影算错跨度"会在判据里显形（拿投影自己的读数当 oracle 是自我确认）。
/// 返回 `None` = 选中的身份一个都不在工程里（空选区 / 陈旧身份）。
fn model_span_of(project: &YebanProjectV1, selected_ulids: &[String]) -> Option<(u64, u64)> {
    let wanted: std::collections::BTreeSet<&str> =
        selected_ulids.iter().map(String::as_str).collect();
    let mut span: Option<(u64, u64)> = None;
    for entry in project.clip_pool.values() {
        let Some(notes) = entry.content.notes() else {
            continue;
        };
        for note in notes.values() {
            let id = note.id.to_canonical_string();
            if !wanted.contains(id.as_str()) {
                continue;
            }
            let end = note.start_tick + note.duration_ticks;
            span = Some(match span {
                None => (note.start_tick, end),
                Some((start, best_end)) => (start.min(note.start_tick), best_end.max(end)),
            });
        }
    }
    span
}

/// 从**模型**算工程的**内容末端 tick**：全部剪辑摆放 `start + duration` 与全部段落
/// `end_tick` 的最大值（`Shift+Z` 的独立 oracle）。
fn model_content_end(project: &YebanProjectV1) -> u64 {
    let clip_end = project
        .tracks
        .values()
        .flat_map(|track| track.clips.values())
        .map(|placement| placement.start_tick + placement.duration_ticks)
        .max()
        .unwrap_or(0);
    let section_end = project
        .sections
        .values()
        .map(|section| section.end_tick)
        .max()
        .unwrap_or(0);
    clip_end.max(section_end)
}

/// 把一批音符身份写进选区的**唯一事实源** `selected-ulids`（并把计数同步过去）。
fn select_ulids(window: &yeban_app::ui::MainWindow, ids: &[String]) {
    window.set_selected_ulids(slint::ModelRc::new(slint::VecModel::from(
        ids.iter()
            .map(|id| slint::SharedString::from(id.as_str()))
            .collect::<Vec<_>>(),
    )));
    window.set_selected_note_count(i32::try_from(ids.len()).unwrap_or(i32::MAX));
}

/// 判据 Z1（横向缩放，**默认构建**）：`Z` ⇒ 选区撑满视口；`Shift+Z` ⇒ 全曲总览；
/// **空选区按 `Z` ⇒ 没有变化且键未被消费**；缩放**不进撤销栈**。
///
/// ## 量什么 / 怎么量 / 单位
///
/// | 量 | 怎么量 | 单位 |
/// | :--- | :--- | :--- |
/// | 当前缩放 | `MainWindow.roll-ticks-per-pixel`（`host::roll_ticks_per_pixel` 是唯一读点） | tick / 逻辑像素 |
/// | 滚动偏移 | `MainWindow.roll-scroll-x` | 逻辑像素 |
/// | 选区跨度 | [`model_span_of`]（**模型**侧 oracle） | tick |
/// | 内容末端 | [`model_content_end`]（**模型**侧 oracle） | tick |
/// | 视口宽 | `slint::Window::size().width`（与 `apply_view` / 裁剪同一个读数） | 逻辑像素 |
/// | 画面变化 | [`frame_diff`] 的差异**像素数** + 包围盒（逐像素比 RGB） | 像素 |
///
/// ## 算术（全部整数、**向上取整**；期望值在这里手算，不引用被测函数）
///
/// - 夹具（`filled_project`）：4 个音符，起点 `0 / 960 / 1920 / 2880`，
///   时值各 `480` ⇒ 终点 `480 / 1440 / 2400 / 3360`。工程内容末端 **`15360`**
///   （两个段落 `Intro = 0..7680`、`Drop = 7680..15360` 的较大者；剪辑末端 `3840` 更短）
///   —— 出处：`crates/yeban-model/src/samples.rs` 的 `filled_project`。
/// - 视口 `1920` px。夹紧上下界是 `1` / `960`。
/// - 全选（并集跨度 `3360 − 0 = 3360`）⇒ `ceil(3360 / 1920) = 2`，滚动 `0 / 2 = 0`。
/// - 只选**最后一个**音符（跨度 `3360 − 2880 = 480`）⇒ `ceil(480 / 1920) = 1`，
///   滚动 `2880 / 1 = 2880`。
/// - `Shift+Z`（内容末端 `15360`）⇒ `ceil(15360 / 1920) = 8`，滚动 `0`。
///
/// ## 怎么变红（每条都实测过）
///
/// | 注入 | 位置 | 现象 |
/// | :--- | :--- | :--- |
/// | 把 `ZoomToSelection` / `ZoomToFit` 放回 `action_has_implementation` 的 `!matches!` | `src/host.rs` | 注入之后缩放一位不变（本判据第一段红），`cli_contract.rs` 的 B11d 也红 |
/// | 跨度只取**一个**音符的区间（不取并集） | `src/bridge.rs` 的 `selection_tick_span` | 全选那一步的期望值 `2` 变成 `1` ⇒ 红 |
/// | 换算改成**向下取整**（`span / px`） | `src/bridge.rs` 的 `ticks_per_pixel_to_fit` | 全选那一步 `3360 / 1920` 的向下取整 = `1`，期望 `2` ⇒ 红（**这一步是"向上取整"的牙**；`3841 / 1920` 那条边界由 `bridge` 的单元判据钉住） |
/// | 选区总览不写 `scroll-x` | `src/host.rs` 的 `zoom_action` | 最后一步 `roll-scroll-x` 期望 `2880`、实得 `0` ⇒ 红 |
#[test]
fn zoom_shortcuts_reach_the_projection_and_an_empty_selection_is_not_consumed() {
    use std::rc::Rc;
    use yeban_app::undo::{UndoPort, UndoSession};
    use yeban_ui_test_port::port::KeyCode;

    /// 缩放的上下界（`bridge::MIN/MAX_TICKS_PER_PIXEL`）—— 写成**字面量**做独立 oracle；
    /// 常量本身由 `bridge` 的单元判据钉住。单位：tick / 逻辑像素。
    const MIN_TPP: u64 = 1;
    const MAX_TPP: u64 = 960;

    const NOW: u64 = 1_760_000_000_000;
    let project = yeban_model::samples::filled_project();
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<判据:横向缩放>", "yeban-app", project.clone(), NOW)
            .expect("打开撤销会话"),
    ));
    let wiring = LiveWiringOptions {
        permission: Permission::Interactive,
        console_tab: 0,
        save_path: None,
        engine_quanta: 0,
        undo: Some(Rc::clone(&port)),
    };
    let mut ui = build_live_ui_with(&project, &wiring).expect("真实界面 + Tier-1 执行面");

    // ---- 起点读数 ----
    let viewport_px = slint::ComponentHandle::window(ui.ui()).size().width;
    assert_eq!(viewport_px, 1920, "Tier-1 装配的视口宽 = DemoScene 的 1920");
    let vw = u64::from(viewport_px);
    let fit = |span: u64| span.div_ceil(vw).clamp(MIN_TPP, MAX_TPP);
    assert_eq!(
        ui.ui().get_roll_ticks_per_pixel(),
        30,
        "起点缩放 = `DEFAULT_TICKS_PER_PIXEL`（属性默认值）"
    );
    assert_eq!(ui.ui().get_roll_scroll_x(), 0.0, "起点偏移 = 0");
    assert_eq!(port.display().undoable, 0, "起点不该有可撤销的编辑");

    // 默认帧（**未注入任何按键**）：同一状态两次抓帧逐字节相同 = 命题①（确定性）。
    // sha256 是这次运行的读数（逐字节；本仓 PNG 是存储式 deflate ⇒ 尺寸证明不了内容）。
    let frame_default = ui.capture().expect("默认外观帧");
    let frame_default_again = ui.capture().expect("默认外观帧（第二次）");
    assert!(
        frame_diff(&frame_default, &frame_default_again).is_none(),
        "同一状态的两次抓帧必须逐字节相同（命题①：默认帧可复现）"
    );
    let (default_png, default_evidence) =
        encode_with_evidence(&frame_default, DEFAULT_MAX_PNG_BYTES).expect("默认帧必须可编码");
    let default_digest = yeban_model::ids::ContentHash::of_bytes(&default_png);
    report_line(&format!(
        "[zoom-pixel] 默认外观（未注入按键）: {}x{} PNG {} 字节 / non_black={} / 颜色 {} 种 / \
         指纹 {:016x} / sha256={}（帧字节的 sha256，从 PNG 字节读）",
        frame_default.width(),
        frame_default.height(),
        default_png.len(),
        default_evidence.non_black_pixels,
        default_evidence.distinct_colors,
        default_evidence.fingerprint,
        default_digest.as_str()
    ));

    // ---- 选区 = 运行时树里**全部**可见音符身份（与删除判据同一份集合口径）----
    let all_selected = note_ulids_in_tree(&ui.tree_snapshot());
    assert!(
        all_selected.len() >= 2,
        "夹具必须至少有两个可见音符（否则「并集跨度」与「单音符跨度」无法区分）: {all_selected:?}"
    );
    let model_notes = model_note_count(&project);

    // ================= ① `Z`：全选 ⇒ 撑满视口 =================
    select_ulids(ui.ui(), &all_selected);
    let (span_start, span_end) =
        model_span_of(&project, &all_selected).expect("全选的身份必须都能在模型里解析出来");
    let span_all = span_end - span_start;
    let expected_all = fit(span_all);
    assert_ne!(
        expected_all,
        fit(model_span_of(&project, &all_selected[..1])
            .expect("第一个音符")
            .1
            - model_span_of(&project, &all_selected[..1])
                .expect("第一个音符")
                .0),
        "夹具要能让「并集跨度」与「单音符跨度」给出**不同**的期望值（否则这条判据没有牙）"
    );
    // **无头 Tier-1 端口的真实事件源**：注入未修饰的 `z`。
    ui.dispatch_key_press(KeyCode::Character('z'))
        .expect("注入 `Z`");
    assert_eq!(
        ui.ui().get_roll_ticks_per_pixel(),
        i32::try_from(expected_all).expect("tpp 落在 i32 内"),
        "① `Z` 之后缩放 = ceil({span_all} / {vw}) = {expected_all}"
    );
    assert_eq!(
        ui.ui().get_roll_scroll_x(),
        0.0,
        "① 选区从 tick 0 开始 ⇒ 滚动 0 / {expected_all} = 0"
    );

    // ================= ② `Z`：只选最后一个音符 ⇒ 撑满 + 把选区滚到左沿 =================
    let last_only = vec![all_selected.last().expect("最后一个").clone()];
    select_ulids(ui.ui(), &last_only);
    let (last_start, last_end) = model_span_of(&project, &last_only).expect("最后一个音符");
    let span_last = last_end - last_start;
    let expected_last = fit(span_last);
    ui.dispatch_key_press(KeyCode::Character('z'))
        .expect("注入 `Z`（单音符选区）");
    assert_eq!(
        ui.ui().get_roll_ticks_per_pixel(),
        i32::try_from(expected_last).expect("tpp 落在 i32 内"),
        "② 单音符跨度 ceil({span_last} / {vw}) = {expected_last}"
    );
    assert_eq!(
        ui.ui().get_roll_scroll_x(),
        last_start as f32,
        "② 选区左沿（tick {last_start} / {expected_last} = {}px）必须滚到视口左沿",
        last_start / expected_last
    );

    // ================= ③ `Shift+Z`：全曲总览 =================
    //
    // 端口表达不了修饰键 chord（`KeyCode` 只有 `ShiftEnter` 一个组合变体，新增变体要动
    // `yeban-ui-test-port`，不在本切片的改动面）⇒ 走 `.slint` 回调那一格，
    // 与既有判据 16 的第 ④ 步同款。
    let content_end = model_content_end(&project);
    let expected_fit = fit(content_end);
    assert!(
        ui.ui()
            .invoke_key_action("z".into(), true, false, false, false),
        "`Shift+Z` 必须被消费"
    );
    assert_eq!(
        ui.ui().get_roll_ticks_per_pixel(),
        i32::try_from(expected_fit).expect("tpp 落在 i32 内"),
        "③ `Shift+Z` 之后缩放 = ceil({content_end} / {vw}) = {expected_fit}"
    );
    assert_eq!(
        ui.ui().get_roll_scroll_x(),
        0.0,
        "③ 全曲总览必须回到原点（滚动 0）"
    );

    // ---- 命题②：缩放之后画面**真的变了**（不同像素数 + 包围盒）----
    //
    // 这一条与命题①（可复现）是两件事：① 只证明"同一状态同一帧"，② 证明"注入之后
    // 画面跟着投影动"。判据不碰 golden（比的是**同一次运行**的两帧）。
    let frame_zoomed = ui.capture().expect("全曲总览帧");
    let diff = frame_diff(&frame_default, &frame_zoomed).unwrap_or_else(|| {
        panic!("`Shift+Z` 真的改了投影参数，但两帧逐字节相同 —— 界面没有跟着投影动")
    });
    assert!(
        diff.count > 1000,
        "缩放改动的时间轴区域必须有大片像素差: {} 个像素，包围盒 {:?}",
        diff.count,
        diff.bbox
    );
    report_line(&format!(
        "[zoom-pixel] 注入 `Z` / `Shift+Z` 之后: 差异 {} 像素, 包围盒 {}x{} @({},{}) \
         （命题②：画面变了；命题①的 sha256 见上）",
        diff.count, diff.bbox.width, diff.bbox.height, diff.bbox.x, diff.bbox.y
    ));

    // ================= ④ 空选区按 `Z` ⇒ 没有变化 且 键未被消费 =================
    select_ulids(ui.ui(), &[]);
    let before_tpp = ui.ui().get_roll_ticks_per_pixel();
    let before_scroll = ui.ui().get_roll_scroll_x();
    let frame_before_empty = ui.capture().expect("空选区之前那一帧");
    // 先读**返回值**（消费与否），再走端口的真实事件源确认状态一位没动。
    assert!(
        !ui.ui()
            .invoke_key_action("z".into(), false, false, false, false),
        "空选区按 `Z` 必须**不消费**（返回 false）—— 没有可作用的对象"
    );
    ui.dispatch_key_press(KeyCode::Character('z'))
        .expect("注入 `Z`（空选区）");
    assert_eq!(
        ui.ui().get_roll_ticks_per_pixel(),
        before_tpp,
        "④ 空选区按 `Z` 之后缩放必须**一位不变**"
    );
    assert_eq!(
        ui.ui().get_roll_scroll_x(),
        before_scroll,
        "④ 空选区按 `Z` 之后滚动必须**一位不变**"
    );
    let frame_after_empty = ui.capture().expect("空选区之后那一帧");
    assert!(
        frame_diff(&frame_before_empty, &frame_after_empty).is_none(),
        "④ 空选区按 `Z` 之后画面必须逐字节不变（不消费的可见后果）"
    );
    // 陈旧身份（工程里不存在）与空选区同义。
    select_ulids(ui.ui(), &["01J8Z5Q0R7K3M9X2V4B6N8P0PZ".to_owned()]);
    assert!(
        !ui.ui()
            .invoke_key_action("z".into(), false, false, false, false),
        "选中的身份一个都不在工程里时，`Z` 同样不消费"
    );

    // ================= ⑤ 缩放是**视图态**：不进撤销栈、不动工程 =================
    assert_eq!(
        port.display().undoable,
        0,
        "缩放**不进撤销栈**（撤销记的是工程内容 `Op`，缩放只是这一帧画多宽）"
    );
    assert_eq!(
        model_note_count(&port.project()),
        model_notes,
        "缩放一位没动工程（音符数不变）"
    );

    report_line(&format!(
        "[zoom] `Z`: 全选跨度 {span_all} ⇒ tpp {expected_all}（滚动 0）; 单音符跨度 {span_last} ⇒ \
         tpp {expected_last}（滚动 {last_start}）; `Shift+Z`: 内容末端 {content_end} ⇒ tpp \
         {expected_fit}（滚动 0）; 空选区 `Z` ⇒ 返回 false 且读数不变; 撤销栈深度 0"
    ));
}

// ===========================================================================
// AI 建议采纳（`Shift+Enter`）与试听主线（`[`）：能力缺失 ⇒ 如实拒绝且一位不变
// ===========================================================================

/// 判据 B11f 的**行为侧**（`[UI-A11Y-001]` / `[ARCH-RT-005]`）：`Shift+Enter` 与 `[`
/// 在**真实界面**上被如实拒绝，而且**一位不变**（不是"吞掉键却什么都不做"）。
///
/// ## 为什么要有这一条（它不是"什么都没做"的空判据）
///
/// 它证明的是**否定命题**："这两个动作没有落地实现" —— 这一点只能靠"按键之后没有任何
/// 可观测变化"来证，而"没有任何变化"必须先证明**探针能看见变化**，否则断言是空的。
/// 因此本判据自带一个**阳性对照**：注入已落地且已绑定的 `B`（箭头 ⇄ 铅笔）⇒ 同一个读数
/// 面必须真的变。阳性对照不过 ⇒ 本判据的"一位不变"不算证据。
///
/// ## 量什么 / 怎么量 / 单位
///
/// | 步 | 量什么 | 怎么量 | 单位 |
/// | :--- | :--- | :--- | :--- |
/// | ① | 工具矩阵行号 | `MainWindow.active-tool` | 行号（1–5） |
/// | ② | 视图档 | `MainWindow.arrangement-view` | bool |
/// | ③ | 走带显示态 | `MainWindow.playing` | bool |
/// | ④ | 横向缩放 / 滚动 | `MainWindow.roll-ticks-per-pixel` / `roll-scroll-x` | tick/px、tick |
/// | ⑤ | 工程音符条目数 | `model_note_count(UndoPort::project())` | 条目 |
/// | ⑥ | 可撤销步数 / 提交数 | `UndoPort::display().undoable` / `.commit_count` | 步、次 |
/// | ⑦ | 帧内容 | `LiveControlPlane::capture()` 的**差异像素数**与 **PNG 字节的 sha256** | 像素、哈希 |
///
/// ## 怎么变红
///
/// - 把 `Action::AcceptAiSuggestion` 或 `Action::AuditionMain` 从
///   `host::action_has_implementation` 的 `!matches!` 移出而没真接上能力 ⇒ ①②…⑦ 里
///   至少一项变化（键被消费 ⇒ `invoke_key_action` 返回 `true`）。
/// - 把这两个键从 `input.rs` 的策略表里删掉 ⇒ `invoke_key_action` 仍然返回 `false`，
///   但 `tests/cli_contract.rs` 的判据 B11f 会红（它要求键仍绑定）。
/// - 把阳性对照用的 `B` 摘掉 ⇒ 本判据的阳性对照步变红（说明探针失灵，而不是"引擎安静"）。
#[test]
fn the_ai_suggestion_and_audition_keys_are_rejected_without_any_state_change() {
    use std::rc::Rc;
    use yeban_app::undo::{UndoPort, UndoSession};
    use yeban_ui_test_port::port::KeyCode;

    const NOW: u64 = 1_760_000_000_000;
    let project = yeban_model::samples::filled_project();
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<判据:AI建议与试听>", "yeban-app", project.clone(), NOW)
            .expect("打开撤销会话"),
    ));
    let wiring = LiveWiringOptions {
        permission: Permission::Interactive,
        console_tab: 0,
        save_path: None,
        engine_quanta: 0,
        undo: Some(Rc::clone(&port)),
    };
    let mut ui = build_live_ui_with(&project, &wiring).expect("真实界面 + Tier-1 执行面");

    // ---- 默认帧（**未注入任何按键**）：命题①（确定性）＝同一状态两次抓帧逐字节相同 ----
    let frame_before = ui.capture().expect("默认外观帧");
    let frame_before_again = ui.capture().expect("默认外观帧（第二次）");
    assert!(
        frame_diff(&frame_before, &frame_before_again).is_none(),
        "命题①: 同一状态的两次抓帧必须逐字节相同（默认帧可复现）"
    );
    let (before_png, before_evidence) =
        encode_with_evidence(&frame_before, DEFAULT_MAX_PNG_BYTES).expect("默认帧必须可编码");
    let before_digest = yeban_model::ids::ContentHash::of_bytes(&before_png);

    // ---- 起点读数（每一条的单位见本判据的文档表） ----
    let tool_before = ui.ui().get_active_tool();
    let view_before = ui.ui().get_arrangement_view();
    let playing_before = ui.ui().get_playing();
    let tpp_before = ui.ui().get_roll_ticks_per_pixel();
    let scroll_before = ui.ui().get_roll_scroll_x();
    let notes_before = model_note_count(&port.project());
    let undoable_before = port.display().undoable;
    let commits_before = port.display().commit_count;

    // ---- ① `.slint` 回调那一格（`host::wire_keys` 的唯一落点）：两条都必须 `reject` ----
    assert!(
        !ui.ui()
            .invoke_key_action("\n".into(), true, false, false, false),
        "`Shift+Enter → 采纳 AI 建议` 的能力今天不存在 ⇒ 必须如实返回 false（不得假装采纳了）"
    );
    assert!(
        !ui.ui()
            .invoke_key_action("[".into(), false, false, false, false),
        "`[ → 试听主线` 的能力今天不存在（`[ARCH-RT-005]` 未实现）⇒ 必须如实返回 false"
    );
    assert!(
        !ui.ui()
            .invoke_key_action("]".into(), false, false, false, false),
        "`] → 试听 AI 提案分支` 与 `[` 同一件事的另一半 ⇒ 同样必须如实返回 false"
    );

    // ---- ② 端口**真实事件源**注入（不是直接调回调） ----
    ui.dispatch_key_press(KeyCode::ShiftEnter)
        .expect("端口注入 `Shift+Enter`");
    ui.dispatch_key_press(KeyCode::Character('['))
        .expect("端口注入 `[`");
    ui.dispatch_key_press(KeyCode::Character(']'))
        .expect("端口注入 `]`");

    // ---- ③ 被拒绝的键**不得有副作用**：逐项回读 ----
    assert_eq!(
        ui.ui().get_active_tool(),
        tool_before,
        "被拒绝的键不得换工具（active-tool 必须一位不变）"
    );
    assert_eq!(
        ui.ui().get_arrangement_view(),
        view_before,
        "被拒绝的键不得切视图"
    );
    assert_eq!(
        ui.ui().get_playing(),
        playing_before,
        "被拒绝的键不得驱动走带（`playing` 一位不变）"
    );
    assert_eq!(
        ui.ui().get_roll_ticks_per_pixel(),
        tpp_before,
        "被拒绝的键不得缩放（roll-ticks-per-pixel 一位不变）"
    );
    assert_eq!(
        ui.ui().get_roll_scroll_x(),
        scroll_before,
        "被拒绝的键不得滚动（roll-scroll-x 一位不变）"
    );
    assert_eq!(
        model_note_count(&port.project()),
        notes_before,
        "被拒绝的键不得改工程内容（音符条目数一位不变）"
    );
    assert_eq!(
        port.display().undoable,
        undoable_before,
        "被拒绝的键不得进撤销栈（可撤销步数一位不变）"
    );
    assert_eq!(
        port.display().commit_count,
        commits_before,
        "被拒绝的键不得产生提交（commit_count 一位不变）"
    );

    // ---- ④ 画面也一位不变（命题①的可见后果） ----
    let frame_after = ui.capture().expect("注入被拒绝的键之后的帧");
    assert!(
        frame_diff(&frame_before, &frame_after).is_none(),
        "被拒绝的键的可见后果必须是**逐字节不变** —— 有差异就说明某个键真的被消费了"
    );

    // ---- ⑤ 阳性对照：探针不是瞎的 ----
    //
    // 注入一条**已落地且已绑定**的键（`B` = 箭头 ⇄ 铅笔）。它必须被消费、并且真的改读数。
    // 这一步不过 ⇒ 上面那些"一位不变"不构成证据。
    assert!(
        ui.ui()
            .invoke_key_action("b".into(), false, false, false, false),
        "阳性对照: `B` 必须被消费"
    );
    let tool_after_b = ui.ui().get_active_tool();
    assert_ne!(
        tool_after_b, tool_before,
        "阳性对照: `B` 必须真的换工具（否则本判据的读数面对变化不敏感）"
    );
    let frame_after_b = ui.capture().expect("阳性对照帧");
    let positive_diff = frame_diff(&frame_before, &frame_after_b);
    // 画面差异只**报告**、不断言：工具高亮是逐像素可判的，但它不是本判据要证的那件事
    // （本判据要证的是"被拒绝的键零变化"）。报告出来是为了让读者知道探针能看见变化。
    report_line(&format!(
        "[unimplemented-keys-pixel] 默认外观（未注入按键）: {}x{} PNG {} 字节 / non_black={} / \
         颜色 {} 种 / 指纹 {:016x} / sha256={}（PNG 字节的 sha256, 本机本次运行的读数）",
        frame_before.width(),
        frame_before.height(),
        before_png.len(),
        before_evidence.non_black_pixels,
        before_evidence.distinct_colors,
        before_evidence.fingerprint,
        before_digest.as_str()
    ));
    report_line(&format!(
        "[unimplemented-keys] 拒绝侧: `Shift+Enter` / `[` / `]` 经回调与端口**两条**路径注入 ⇒ \
         回调返回 false, active-tool={tool_before}（不变）、arrangement-view={view_before}（不变）、\
         playing={playing_before}（不变）、tpp={tpp_before}（不变）、scroll={scroll_before}（不变）、\
         音符条目={notes_before}（不变）、undoable={undoable_before}（不变）、\
         commit_count={commits_before}（不变）、帧差异=0 像素; 阳性对照 `B` ⇒ active-tool \
         {tool_before}→{tool_after_b}, 帧差异 {} 像素（本判据关心的是前者为 0）",
        positive_diff.as_ref().map_or(0, |diff| diff.count)
    ));
}

// =====================================================================================
// 判据：`Escape` 关闭**全屏时光机**（`[UI-A11Y-003]` §7.3 / `.slint` 的 "Esc 关闭" 那句）
// =====================================================================================

/// 判据：时光机开着时，`Escape` 经**真事件源**注入必须把它关掉，并且**不碰工程**。
///
/// ## 缺陷的形状（为什么这条判据有判别力）
///
/// `ui/dialogs/undo_tree_modal.slint` 的底部提示行写着 `… · Esc 关闭`，而
/// `host::apply_action` 的 `Action::Cancel` 分支此前**只**问两条拖拽手势
/// （`cancel_track_height_drag` 与窗口回调 `mixer-cancel-gesture`）。时光机开着时
/// 两条都不在手 ⇒ 返回 `false`（`reject`）⇒ 弹窗**关不掉**，那句话是假的。
/// 更强的证据：全仓**没有**任何调用点读 `UiAction::CloseUndoTree`
/// （改动前只有 `undo.rs` 的变体定义、它的 `name()` 与它自己的自判据）。
///
/// ## 为什么关弹窗**必须**走 `UndoPort`
///
/// `undo-tree-open` 的**唯一写者**是 `host::apply_undo`，它每一跳都从
/// `port.undo_tree_open()` 回写 ⇒ 直接 `window.set_undo_tree_open(false)` 会在下一次
/// 回写时被宿主的读数覆盖。因此这条判据断言的是"端口读数 + 运行时树"两处同时变化，
/// 而不是只断言一个可以被覆盖的属性。
///
/// ## 端到端链（每一步都是字面读数）
///
/// | 步 | 动作 | 断言 |
/// | :--- | :--- | :--- |
/// | ① | 经端口打开时光机 → `apply_undo` | 端口读数 `true`；运行时树里**有** `undo-tree-modal` |
/// | ② | `ui/dispatch_key_press` 注入 `Escape` | 注入成功；端口读数 `false`（弹窗关掉） |
/// | ③ | 读运行时树 | `undo-tree-modal` **不在**树里（`.slint` 的 `visible` 归位） |
/// | ④ | 动作日志 + 工程读数 | 最后一条记录的动作名 == `"close-undo-tree"`；撤销栈与提交数一位未动 |
///
/// ## 注入（负向实测）
///
/// 把 `host.rs` 的 `Action::Cancel` 分支里那段 `CloseUndoTree` 落点删掉（或把
/// `undo_tree_open()` 的判定改成恒 `false`）⇒ ② 的端口读数与 ③ 的树读数**同时**变红。
#[test]
fn escape_closes_the_time_machine_modal_for_real() {
    use std::rc::Rc;

    use yeban_app::undo::{UndoPort, UndoSession};
    use yeban_ui_test_port::port::KeyCode;

    /// 会话打开时刻（与既有判据同一个夹具常量）。
    const NOW: u64 = 1_760_000_000_000;

    let project = yeban_model::samples::filled_project();
    let port = Rc::new(UndoPort::new(
        UndoSession::open("<判据:Esc 关时光机>", "yeban-app", project.clone(), NOW).expect("打开"),
    ));
    let mut ui = build_live_ui_with(
        &project,
        &LiveWiringOptions {
            permission: Permission::Interactive,
            console_tab: 0,
            save_path: None,
            engine_quanta: 0,
            undo: Some(Rc::clone(&port)),
        },
    )
    .expect("装配");
    yeban_app::host::apply_undo(ui.ui(), &port);
    ui.pump_meters();

    // ---- ① 经**端口**打开 ⇒ 弹窗进树 ----
    port.perform(yeban_app::undo::UiAction::OpenUndoTree);
    yeban_app::host::apply_undo(ui.ui(), &port);
    ui.pump_meters();
    assert!(
        port.undo_tree_open(),
        "① 经端口打开之后端口读数必须是 `true`"
    );
    let tree_open = ui.tree_snapshot();
    assert!(
        tree_open.contains("undo-tree-modal"),
        "① 时光机开着 ⇒ 它必须进运行时树（否则本判据的 ③ 步没有判别力）"
    );
    let (undoable_before, commits_before) = {
        let display = port.display();
        (display.undoable, display.commit_count)
    };
    report_line(&format!(
        "[escape-close-tree] ① 打开: 端口读数={} / 树里有 `undo-tree-modal`={} / \
         可撤销 {undoable_before} 步 / 提交 {commits_before} 条",
        port.undo_tree_open(),
        tree_open.contains("undo-tree-modal"),
    ));

    // ---- ② `Escape` 经**真事件源**注入 ----
    ui.dispatch_key_press(KeyCode::Escape)
        .expect("② `Escape` 注入必须落到真实窗口（Interactive 档）");
    assert!(
        !port.undo_tree_open(),
        "② `Escape` 必须让端口读数归 `false` —— 这是 `Action::Cancel` 真的读到了 \
         `UiAction::CloseUndoTree` 的证据（改动前这里恒 `true`）"
    );

    // ---- ③ 运行时树：模态面板必须消失 ----
    ui.pump_meters();
    let tree_closed = ui.tree_snapshot();
    assert!(
        !tree_closed.contains("undo-tree-modal"),
        "③ `undo-tree-open=false` ⇒ 时光机**不许**再在运行时树里（`.slint` 的 `visible` 归位）"
    );

    // ---- ④ 动作日志 + 工程读数 ----
    let last = port
        .records()
        .last()
        .cloned()
        .expect("② 之后必须有动作记录");
    assert_eq!(
        last.action, "close-undo-tree",
        "④ 最后一条动作记录必须逐字是 `close-undo-tree`（走的是那一个唯一下发点）"
    );
    let display_after = port.display();
    assert_eq!(
        (display_after.undoable, display_after.commit_count),
        (undoable_before, commits_before),
        "④ 关闭弹窗**只改界面运行态**：可撤销步数与提交数一位都不许动"
    );
    report_line(&format!(
        "[escape-close-tree] ②③④ 注入 `Escape`: 端口读数={} / 树里有 `undo-tree-modal`={} / \
         最后一条动作={:?} / 可撤销 {undoable_before}→{} / 提交 {commits_before}→{}",
        port.undo_tree_open(),
        tree_closed.contains("undo-tree-modal"),
        last.action,
        display_after.undoable,
        display_after.commit_count,
    ));
}
