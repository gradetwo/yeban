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

// 判据 17（`ROAD-M4-008` 选项 (a)）：以进程内控制面会话为**唯一可变权威**的装配入口
// 与刷新点。它们只存在于非默认 feature `in-process-mcp` 下 —— 默认构建里没有
// `yeban-mcp`（`tree -p yeban-app -e normal` 命中 0），因此这里也必须 cfg。
#[cfg(feature = "in-process-mcp")]
use live::{AuthoritySync, build_live_ui_from_authority};

// 判据 17 的**同案差分像素**（`ADR-0005` S1）：只被 non-default feature 下的那一条
// 用例用到，因此连同常量 / 助手一起 cfg —— 否则默认构建里它们是 dead_code（`-D warnings`）。
#[cfg(feature = "in-process-mcp")]
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
#[cfg(feature = "in-process-mcp")]
struct FrameDiff {
    /// 差异像素数。
    count: u64,
    /// 差异像素的整数包围盒（含边界；单位 = 像素）。
    bbox: Rect,
}

/// 逐像素比较两帧；**逐字节相同** ⇒ `None`（"界面一位没动"）。
#[cfg(feature = "in-process-mcp")]
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
