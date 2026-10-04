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

use live::{LiveControlPlane, LiveWiringError, build_live_ui};

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
