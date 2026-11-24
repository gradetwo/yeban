//! **层与出口的守卫**（2026-11-19 用户截图点破的问题 → 变成机械检查）。
//!
//! 背景：四条现场缺陷（救援面板压住音源管理、下载看不见进度、换好音源后红字不散、
//! 横幅中英文叠字）当时**没有任何判据**能拦住 —— 因为它们是"两层同时在场""窗口一矮"
//! 这类状态里的问题，而既有判据都是一层一层单独开。本文件把其中**能从源码钉住的**
//! 三类钉死，剩下的用像素截图（1200×700 最小窗口）人工过一遍。
//!
//! | 不变量 | 为什么它能防回退 |
//! | :--- | :--- |
//! | ① 音源管理的 `z` **必须**大于救援面板的 `z` | 写反了就是用户截图里那张"音源管理在后面" |
//! | ② 两个弹层的面板尺寸都必须带 `Math.min(... , root.width/height - …)` 上限 | 上限一没，窗口一矮就把底部出口裁掉（"看不见先不管"） |
//! | ③ 打开音源管理之前必须先收起救援面板 | 否则两层叠着，下载进度被盖住 |
//! | ④ 引擎横幅的消息文本必须 `clip` + `elide` | 否则中文提醒与英文按钮抢同一行、叠字 |
//!
//! ## 反向腿
//! - 把 `app.slint` 里两处 `z:` 互换 ⇒ ① 红；
//! - 把 `missing_sounds_dialog.slint` 的 `panel-h` 改回"只按行数算" ⇒ ② 红；
//! - 删掉 `host::open_library_for` 里那句 `set_missing_sounds_open(false)` ⇒ ③ 红；
//! - 把横幅里那层 `clip: true` / `overflow: elide` 去掉 ⇒ ④ 红。

use std::path::Path;

fn read(relative: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
    std::fs::read_to_string(&path).unwrap_or_else(|err| panic!("读 {} 失败：{err}", path.display()))
}

/// ① 两层弹层的层级：音源管理在救援面板**之上**。
#[test]
fn the_sound_library_sits_above_the_rescue_panel() {
    let app = read("ui/app.slint");
    let rescue_z = z_after(&app, "missing-sounds-dialog := MissingSoundsDialog {");
    let library_z = z_after(&app, "sound-library-dialog := SoundLibraryDialog {");
    assert!(
        library_z > rescue_z,
        "音源管理（z={library_z}）必须盖在救援面板（z={rescue_z}）上面 —— \
         写反了就是用户截图里'点击下载声音管理在后面、看不见进度'"
    );
}

fn z_after(source: &str, anchor: &str) -> i64 {
    let start = source
        .find(anchor)
        .unwrap_or_else(|| panic!("app.slint 里找不到挂载点：{anchor}"));
    let tail = &source[start..];
    for line in tail.lines().take(12) {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("z:") {
            let number: String = rest
                .trim()
                .chars()
                .take_while(|ch| ch.is_ascii_digit())
                .collect();
            return number
                .parse()
                .unwrap_or_else(|_| panic!("z: 后面不是整数：{line}"));
        }
    }
    panic!("{anchor} 之后 12 行内没有 `z:`")
}

/// ② 两个弹层的面板尺寸都必须对窗口取上限（小屏 / 分屏也够得着底部出口）。
#[test]
fn both_dialog_panels_are_bounded_by_the_window() {
    let rescue = read("ui/dialogs/missing_sounds_dialog.slint");
    assert!(
        rescue.contains("panel-w: Math.min("),
        "救援面板宽度必须 `Math.min(…, root.width - …)`"
    );
    assert!(
        rescue.contains("panel-h: Math.min(Math.min(520px, root.height - 24px)")
            || (rescue.contains("panel-h:") && rescue.contains("root.height -")),
        "救援面板高度必须对 `root.height` 取上限：\n{}",
        rescue
            .lines()
            .find(|line| line.trim_start().starts_with("property <length> panel-h"))
            .unwrap_or("<找不到 panel-h>")
    );
    let library = read("ui/dialogs/sound_library_dialog.slint");
    // 2026-11-19：面板从固定 600 高改成 `min(720, 窗口高 - 24)` —— 两张列表
    // （28 件内置 / 几十件可下载）在 600 高时各只剩 3 行，等于把内容藏起来。
    // 判据只钉"必须对窗口取上限"，具体档位跟着设计走（宽度不变）。
    assert!(
        library.contains("Math.min(820px, root.width - 32px)")
            && library.contains("Math.min(720px, root.height - 24px)"),
        "音源管理面板的宽/高都必须对窗口取上限"
    );
}

/// ③ 去下载时救援面板必须先让位（否则"下载进度"被它盖住）。
#[test]
fn opening_the_library_first_makes_the_rescue_panel_step_aside() {
    let host = read("src/host.rs");
    let anchor = "fn open_library_for";
    let start = host
        .find(anchor)
        .unwrap_or_else(|| panic!("host.rs 里找不到 {anchor}"));
    let body: Vec<&str> = host[start..].lines().take(40).collect();
    let body = body.join("\n");
    assert!(
        body.contains("set_missing_sounds_open(false)"),
        "open_library_for 必须在打开音源管理之前收起救援面板：\n{body}"
    );
}

/// ④ 引擎横幅：消息进裁剪块 + 省略号，动作按钮同样自缩。
#[test]
fn the_error_banner_lets_its_message_shrink_instead_of_overlapping() {
    let app = read("ui/app.slint");
    let start = app
        .find("accessible-id: \"engine-error-banner\";")
        .expect("app.slint 里必须有引擎横幅");
    let block: Vec<&str> = app[start..].lines().take(70).collect();
    let block = block.join("\n");
    assert!(
        block.contains("overflow: TextOverflow.elide"),
        "横幅消息必须 `overflow: TextOverflow.elide`（否则与右侧按钮叠字）：\n{block}"
    );
    assert!(
        block.contains("clip: true"),
        "横幅消息必须住在一个 `clip: true` 的弹性块里：\n{block}"
    );
}

// ===========================================================================
// ⑤ 运行时控件树在"多行"状态下必须**能构造**（行内 id 唯一）
// ===========================================================================

mod runtime_ids {
    use std::rc::Rc;

    use yeban_app::bridge;
    use yeban_app::bridge::ViewState;
    use yeban_app::host;
    use yeban_app::scene::DemoScene;
    use yeban_app::undo::{UndoPort, UndoSession};
    use yeban_model::project::{TrackKind, YebanProjectV1};
    use yeban_ui_test_port::Size;
    use yeban_ui_test_port::inspect::tree_from_element_root;
    use yeban_ui_test_port::port::Permission;
    use yeban_ui_test_port::render::LivePort;

    const NOW_MS: u64 = 1_760_000_000_000;
    const MISSING: &str = "assets/samples/emily_basic/Programs/emily_basic.sfz";

    /// 两条 MIDI 轨都绑着**同一件不存在**的音源 ⇒ 救援面板 2 行 × 3 候选。
    ///
    /// 这是当初把控件树打挂的那个状态：`DuplicateId { id: "missing-sounds-candidate" }`。
    fn project_with_two_missing_samplers() -> YebanProjectV1 {
        let mut project = bridge::demo_project();
        let mut done = 0;
        for id in project.display_track_ids() {
            if done == 2 {
                break;
            }
            let Some(track) = project.tracks.get_mut(&id) else {
                continue;
            };
            if track.kind != TrackKind::Midi {
                continue;
            }
            track.sampler = Some(yeban_model::SamplerBinding {
                note_octave_override: None,
                path: MISSING.to_owned(),
            });
            done += 1;
        }
        assert_eq!(done, 2, "夹具工程必须有两条 MIDI 轨");
        project
    }

    #[test]
    fn the_runtime_tree_survives_two_rescue_rows() {
        let project = project_with_two_missing_samplers();
        let view = ViewState::from_project(&project).expect("投影");
        let port = Rc::new(UndoPort::new(
            UndoSession::open("<行内 id>", "yeban-app", project.clone(), NOW_MS).expect("会话"),
        ));
        let scene = DemoScene::from_view(&view);
        let live = LivePort::new(Size::new(1200, 700), Permission::Interactive, None, || {
            host::build_main_window_with_console_tab(&view, &scene, 2)
        })
        .expect("Tier-1 平台");
        let ui = live.ui();
        host::wire_sound_library_dialog_with(ui, Some(Rc::clone(&port)));
        let rows = yeban_app::missing_sounds::collect(&port.project(), None);
        assert_eq!(rows.len(), 2, "夹具必须造成 2 行救援");
        host::wire_missing_sounds_dialog(ui, Rc::clone(&port), rows);

        let json = tree_from_element_root(ui, None)
            .expect("2 行 × 3 候选的状态下控件树必须能构造（行内 id 必须唯一）")
            .dump_json();
        let tree = yeban_ui_test_port::tree::ControlTree::from_json(&json)
            .expect("同一套校验必须能读回自己导出的 JSON");
        let total = tree.iter().count();
        let mut ids: Vec<String> = tree.iter().map(|node| node.id.clone()).collect();
        ids.sort();
        let before = ids.len();
        ids.dedup();
        assert_eq!(
            before,
            ids.len(),
            "运行时控件树里出现了重复 id（无障碍树会直接构造失败）：{:?}",
            ids
        );
        assert!(
            total >= 10,
            "树节点数异常少（{total}）—— 树可能被裁了：{:?}",
            ids
        );
        assert!(
            ids.iter()
                .any(|id| id.starts_with("missing-sounds-candidate-")),
            "救援候选的 id 必须带行/槽下标：{:?}",
            ids.iter()
                .filter(|id| id.starts_with("missing-sounds"))
                .collect::<Vec<_>>()
        );
    }
}
