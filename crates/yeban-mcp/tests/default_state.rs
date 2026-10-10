//! **R86 的延伸：本 crate 手写的 `Default` 必须是"文档里那一个状态"**（第六批）。
//!
//! 为什么需要它：`#[derive(Default)]` 与手写 `impl Default` 都是**出厂状态**，
//! 而"出厂状态"是后续所有行为的**初始条件**（撤销游标、种子工程、读数缺席）。
//! R86 的形态：**"合法替代状态"注入** —— 把一个字段换成一个**同样合法**的值
//! （不是非法值），看判据抓不抓得住。
//!
//! 本判据覆盖三个**手写** `Default`（`#[derive]` 的那些由 golden_table_probes 的枚举探针与
//! 各模块自己的判据兜底）：
//!
//! | 类型 | 出厂状态 |
//! | :--- | :--- |
//! | [`UndoState::default`] | `Self::new("yeban")` ⇒ 活跃分支 `main`、游标在头、不可重做 |
//! | [`CreateConfig::default`] | 1 条 MIDI 轨 ＋ 4 个种子音符 ＋ 片段名 `Motif` |
//! | [`EngineReadings::default`] | **全部读数缺席**（`None`/0）—— 与"测得静音"区分开 |
//!
//! 注入（实测红）：把 `clip_name: "Motif"` 换成另一个合法名字、把 `track_count` 换成 2、
//! 把 `UndoState::default()` 的分支换成 `master`、把某个 LUFS 读数换成 `Some(0.0)` ⇒ 对应行红。

use yeban_mcp::domain::engine_state::EngineReadings;
use yeban_mcp::domain::project_create::{CreateConfig, DEFAULT_SEED_NOTES, DEFAULT_TRACK_COUNT};
use yeban_mcp::undo_session::{MAIN_BRANCH, UndoState};

#[test]
fn every_handwritten_default_is_the_documented_state() {
    // ① `UndoState::default()` = `Self::new("yeban")`。
    let state = UndoState::default();
    assert_eq!(state.branch(), MAIN_BRANCH, "出厂状态的活跃分支");
    assert_eq!(MAIN_BRANCH, "main", "分支名是已发布的字面量");
    assert_eq!(state.undone(), 0, "游标在头上");
    assert!(!state.can_redo(), "没有历史 ⇒ 不可重做");

    // ② `CreateConfig::default()`：轨数、种子音符、片段名。
    // ⭐ R119/R120：下面拿 `DEFAULT_SEED_NOTES.len()` 当分母 ⇒ **分母必须非空**。
    assert!(
        !DEFAULT_SEED_NOTES.is_empty(),
        "登记表不得为空（否则'音符数等于登记表长度'是空断言）"
    );
    let config = CreateConfig::default();
    assert_eq!(config.track_count, DEFAULT_TRACK_COUNT);
    assert_eq!(DEFAULT_TRACK_COUNT, 1);
    assert_eq!(
        config.notes.len(),
        DEFAULT_SEED_NOTES.len(),
        "种子音符数必须等于登记表长度（⛔ 不靠名字推）"
    );
    assert_eq!(config.notes, DEFAULT_SEED_NOTES.to_vec());
    assert_eq!(
        config.clip_name, "Motif",
        "出厂片段名是 `Motif`（与 edit_notes 的 `Clip` 不同）"
    );
    assert!(config.title.is_empty(), "标题出厂为空 ⇒ 模型用 `Untitled`");
    assert_eq!(config.bpm, None, "速度出厂为 None ⇒ 模型默认 120");
    assert_eq!(
        config.time_signature,
        yeban_model::TimeSignature::default(),
        "拍号出厂 = 模型默认"
    );

    // ③ `EngineReadings::default()`：**全部读数缺席**（`None`/0），与"测得静音"区分开。
    let readings = EngineReadings::default();
    assert_eq!(readings.sample_rate, 0);
    assert_eq!(readings.buffer_frames, 0);
    assert_eq!(readings.integrated_lufs, None);
    assert_eq!(readings.momentary_lufs, None);
    assert_eq!(readings.short_term_lufs, None);
    assert_eq!(readings.loudness_range_lu, None);
    assert_eq!(readings.true_peak_dbfs, None);
    // ⭐ 缺席 ≠ 静音：静音是**测得**的 `Some(f32::NEG_INFINITY)`/很低的读数，
    //    而 `None` 是"还没测"。这条不等把两者钉开。
    assert_ne!(
        readings.integrated_lufs,
        Some(0.0_f32),
        "缺席不得被当成 0 LUFS 的读数"
    );
}
