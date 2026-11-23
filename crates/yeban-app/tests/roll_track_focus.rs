//! **选轨 → 钢琴卷帘精准联动**回归判据（用户需求）。
//!
//! 需求：点一条轨道时，若钢琴卷帘开着，卷帘应
//! ① 横向滚到该轨有内容的片段（不用手动拖）；
//! ② 纵向把音域对齐到该轨音符范围（低音轨自动显示低音区，音符可见）。
//! 「其它地方类似处理」= 选轨（`on_track_select`）与选片段（`on_clip_select`）统一联动。
//!
//! 实现：`host::roll_focus_on_track`（选轨回调里调用）——
//! 遍历该轨 `clips` 的 MIDI 音符求 pitch min/max 与**最早音符** `start_tick`（摆放起点 + 音符 tick），
//! 跨度 < 16 半音时 base = lo - min(2, 余量)（全部可见）；
//! 跨度 ≥ 16 时居中并如实提示「超出 16 行窗口，可纵向滚动」。
//!
//! ⚠ 纯 `include_str!` 源码判据（本机编不了 Slint ⇒ 行为由 CI 的 yeban-app 腿真跑）；
//! 它钉住的是「接线 + 关键分支」，不是像素。

const HOST: &str = include_str!("../src/host.rs");

fn code_lines(src: &str) -> String {
    src.lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// ① 选轨回调真的调了联动函数（不是只设 index）。
#[test]
fn track_select_calls_roll_focus() {
    let h = code_lines(HOST);
    assert!(
        h.contains("roll_focus_on_track(&ui, &project, track_id.as_str())"),
        "on_track_select 的 Ok(view) 分支必须调 roll_focus_on_track"
    );
    assert!(
        h.contains("fn roll_focus_on_track("),
        "必须存在 roll_focus_on_track 实现"
    );
}

/// ② 联动函数同时改「横向滚动」与「纵向音域」。
#[test]
fn roll_focus_sets_scroll_and_pitch_window() {
    let h = code_lines(HOST);
    assert!(
        h.contains("set_roll_scroll_x(roll_scroll_for_tick(ui, start_tick))"),
        "必须横向滚到该轨首个片段起点（复用 roll_scroll_for_tick）"
    );
    assert!(
        h.contains("apply_roll_pitch_range(ui, roll_base_for_pitch_range(lo, hi))"),
        "必须把 roll_base_pitch 对齐到该轨音区（否则低音轨音符仍在窗外）"
    );
    // 纵向两档：窗口内全可见 / 超窗居中
    assert!(
        h.contains("span < lane_count"),
        "必须有「跨度小于窗口」分支"
    );
    assert!(
        h.contains("lane_count / 2"),
        "跨度超出 16 行时必须居中（否则只露一端）"
    );
    // 音频轨 / 空轨守卫：没有 MIDI 音域可对齐 ⇒ 一位不动
    assert!(
        h.contains("let (Some(start_tick), Some(lo), Some(hi)) = (first_note_tick, lo, hi)"),
        "必须有「无 MIDI 音符 ⇒ return false」的守卫"
    );
    // ⭐ T-FIX-193 D4：横向落点取**首个音符** tick（摆放 start 加音符片段内坐标），
    // ⛔ 不是摆放起点 —— 尼龙·扫弦摆放 start=0 而首音符在 80636。
    assert!(
        h.contains("placement.start_tick.saturating_add(note.start_tick)"),
        "横向落点必须由摆放起点 + 音符 tick 得出"
    );
}

/// ③ 「同轨再点直接 return」不得回来 —— 它会把联动一并吞掉。
#[test]
fn same_track_reclick_still_focuses() {
    assert!(
        !HOST.contains("同一条轨再点一次 = 一位不动（⛔ 不白重投影一帧）"),
        "旧的「同轨直接 return」会把精准联动吞掉（on_track_selected 先设 index 再 invoke）"
    );
    assert!(
        code_lines(HOST).contains("let same_track ="),
        "同轨判断必须改成「仍联动、只是不重复写身份」"
    );
}

/// ④ 选片段联动只做**横向**（精确到该片段），音域只按轨改一份（不出现第二真相源）。
#[test]
fn clip_select_focuses_roll_horizontally_without_touching_pitch() {
    let h = code_lines(HOST);
    assert!(
        h.contains("roll_focus_on_clip(&ui, &project, clip_ulid.as_str())"),
        "on_clip_select 必须调 roll_focus_on_clip（点片段也要横向联动）"
    );
    assert!(
        h.contains("fn roll_focus_on_clip("),
        "必须存在 roll_focus_on_clip 实现"
    );
    assert!(
        h.contains("roll_scroll_for_tick(ui, placement.start_tick)"),
        "片段联动必须横向滚到**该片段**的 start_tick（不是整轨首个片段）"
    );
    // 音域只由 roll_focus_on_track 改一份；clip 再改会与按轨口径冲突，
    // 并破坏依赖默认 base=60 的既有卷帘判据（实测 aether_piano_roll lane15=C4 曾红）。
    assert_eq!(
        h.matches("apply_roll_pitch_range(ui, roll_base_for_pitch_range(lo, hi))")
            .count(),
        1,
        "apply_roll_pitch_range 只能出现在 roll_focus_on_track 一处（音域单一真相源）"
    );
}
