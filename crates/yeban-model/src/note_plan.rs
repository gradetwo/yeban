//! `[UI-NOTE-003]` "在吸附网格处画一个音符"的**领域表示**与它到 `Op` 的映射。
//!
//! 为什么住在 `yeban-model` 而不是界面层（账本第 261 轮）：MCP 与 UI **两侧必须共用同一实现**，
//! 而 `yeban-mcp` 的依赖方向是 `yeban-mcp -> yeban-model`（它**不**依赖 `yeban-app`）。
//! 这两个是纯模型域的东西，放在这里两个消费者都能用，且**不需要**写第二份构造。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// `[UI-NOTE-003]` 一次"画音符"的**领域计划**（起点 / 音高 / 时值）。
///
/// 它与 `[UI-NOTE-002]` 的坐标映射**无关**：映射把点击变成 tick 与泳道，这里只承载结果。
pub struct NotePlan {
    /// 起始 tick（已按吸附网格对齐）。
    pub start_tick: u64,
    /// 音高（由点击所在泳道反推）。
    pub pitch: u8,
    /// 时值：规范规定"默认 1 拍" ⇒ 取工程的 `ppq`。
    pub duration_ticks: u64,
}

/// 把 [`NotePlan`] 变成一个**可撤销的**领域操作（`Op::AddNote`，自包含逆操作）。
///
/// 这是 UI 与 MCP 的**共用构造**：两侧都调它, 因此不存在"第二个 AddNote 构造"（账本第 261 轮）。
pub fn plan_to_add_note(
    plan: NotePlan,
    track_id: crate::ids::EntityId,
    clip_id: crate::ids::EntityId,
    note_id: crate::ids::EntityId,
) -> crate::ops::Op {
    crate::ops::Op::AddNote {
        track_id,
        clip_id,
        note: crate::music::MidiNote::new(
            note_id,
            plan.start_tick,
            plan.pitch,
            plan.duration_ticks,
        ),
    }
}
