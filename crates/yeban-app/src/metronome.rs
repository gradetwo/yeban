//! **节拍器的控制面落点**（`[ARCH-RT-001]` 的宿主侧那一半）。
//!
//! ## 为什么这里必须"换一代引擎"（⛔ 不是一个运行时开关）
//!
//! ⭐ 引擎的节拍器计划**住在快照里**：`EngineSnapshot::from_project` 在**投影期**读
//! `project.transport.metronome_enabled`（`snapshot.rs:515` 的 `with_metronome(..)`），
//! 实时侧读的是 `SnapshotArmed::armed_metronome_enabled`（`rt.rs:1515`）—— ⛔ **不是**
//! `Transport` 上的可变位。⇒ **改工程字段必须重装快照**（`EngineHost::reload`）。
//!
//! ## ⚠ 换引擎的代价，以及本模块**必须**补回的两件事
//!
//! `EngineHost::reload` ⛔ **不保走带位置**（`engine_host.rs:301` 逐字："新引擎沿用
//! **自由跑默认值** ⇒ 通常 `Playing` / tick 0"）⇒ 若只换不补，拨一次节拍器开关就会
//! **把播放位置打回 0、并把"停"变成"跑"** —— 那是**破坏性**的。
//!
//! ⇒ 本模块按负责人裁决 (A) 补齐三件：
//! 1. ⭐ **先 `stop`**（⛔ 不从"跑"直接换）；
//! 2. ⭐ 换后 `seek(原位置)`；
//! 3. ⭐ **恢复原走带状态**（原先停 ⇒ 还停；原先跑 ⇒ 还跑）。
//!
//! ## ⚠ ⭐ **如实登记的边界：位置无法"精确"恢复，只能"封顶"**
//!
//! 控制驱动形态（无声卡）下，**每一条走带命令都要 `pump(1)` 才生效**，而 `pump` 推进
//! 走带位置（`engine_host.rs:1075` 逐字："控制驱动的**已知代价**：走带位置会因此前进一个
//! 运行时量子（128 帧）"）。本模块一次拨动要发 3 条命令（`Stop` / `Play`? / `SeekTicks`）
//! ⇒ 位置必然漂移**几个 tick**：⛔ 这不是本模块的缺陷，是**那个形态的既有代价**
//! （设备腿没有它）。
//!
//! ⇒ 本模块的承诺是**有界**的：漂移必须 ≤ [`MAX_RESTORE_DRIFT_TICKS`]，⭐ **如实报出**
//! 实测漂移量，⛔ 不假装"逐 tick 无损"。

use crate::engine_host::{EngineHost, EngineHostError};
use yeban_engine::transport::{TransportReading, TransportState};
use yeban_model::YebanProjectV1;

/// ⭐ **位置恢复的漂移上限**（tick）。
///
/// ⚠ 为什么不为 0：见模块文档的边界段 —— 控制驱动下每条命令都要 `pump(1)`，而 `pump`
/// 推进位置。3 条命令 ⇒ 漂移几个 tick 是**那个形态的既有代价**（设备腿为 0）。
pub const MAX_RESTORE_DRIFT_TICKS: u64 = 16;

/// ⭐ 拨开关之前抓住的走带现场（⭐ 恢复的唯一依据）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransportSnapshot {
    /// 原来的走带状态（⭐ 停/跑/录音三档原样保留）。
    pub state: TransportState,
    /// 原来的绝对位置（960 PPQ tick）。
    pub position_ticks: u64,
}

impl TransportSnapshot {
    /// ⭐ 从一份读数抓现场。
    #[must_use]
    pub const fn from_reading(reading: &TransportReading) -> Self {
        Self {
            state: reading.state,
            position_ticks: reading.position_ticks,
        }
    }
}

/// ⭐ 一次拨动的结果（⭐ 恢复的**实测**读数，⛔ 不是"我以为恢复了"）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MetronomeOutcome {
    /// 拨动之前的现场。
    pub before: TransportSnapshot,
    /// 拨动之后的现场（⭐ 必须与 `before` 相等，否则就是这次拨动**破坏了**走带）。
    pub after: TransportSnapshot,
    /// 拨动之后节拍器是否开着（⭐ 读的是**新快照**的投影结果）。
    pub enabled: bool,
    /// ⭐ 恢复的**实测漂移**（tick，绝对值）—— ⚠ 如实报，⛔ 不假装为 0。
    pub restore_drift_ticks: u64,
}

impl MetronomeOutcome {
    /// ⭐ 走带状态是否被恢复（⭐ 这一档**要求精确**：停还停、跑还跑）。
    #[must_use]
    pub const fn state_preserved(self) -> bool {
        self.before.state as u8 == self.after.state as u8
    }

    /// ⭐ 位置是否在**有界漂移**之内恢复（⭐ 见 [`MAX_RESTORE_DRIFT_TICKS`]）。
    #[must_use]
    pub const fn position_preserved_within_bound(self) -> bool {
        self.restore_drift_ticks <= MAX_RESTORE_DRIFT_TICKS
    }
}

/// ⭐ **把工程里的节拍器开关拨到 `enabled`**，并**无损**恢复走带现场。
///
/// 顺序（⭐ 负责人裁决 A 的 1–3 条，逐条对应）：
/// 1. 抓现场 ⇒ **先 `stop`**；
/// 2. 改工程字段 ⇒ `reload`（⭐ 换一代引擎，**这一步就是代价**）；
/// 3. `seek(原位置)` ⇒ **按原状态**恢复"跑"或"停"。
///
/// # Errors
///
/// 快照投影失败（`EngineHostError`）—— ⚠ 此时**旧的一代原样保留**（`reload` 的契约），
/// 但走带已经被第 1 步停住 ⇒ 调用方拿到 `Err` 后应当把走带恢复到 `before`。
pub fn set_enabled(
    host: &mut EngineHost,
    project: &YebanProjectV1,
    enabled: bool,
) -> Result<MetronomeOutcome, EngineHostError> {
    apply_transport_config(host, project, |next| {
        next.transport.metronome_enabled = enabled;
    })
}

/// ⭐ 拨**预备拍**（`transport.count_in_bars`，`u8`）。
///
/// ⚠ 它与 `metronome_enabled` **同在 `TransportConfig` 里、同样投影进快照**
/// ⇒ 走**同一个** [`apply_transport_config`] 内核与**同一套**走带恢复（⛔ 不复制一份）。
///
/// # Errors
///
/// 同 [`set_enabled`]（快照投影失败）。
pub fn set_count_in_bars(
    host: &mut EngineHost,
    project: &YebanProjectV1,
    bars: u8,
) -> Result<MetronomeOutcome, EngineHostError> {
    apply_transport_config(host, project, |next| {
        next.transport.count_in_bars = bars;
    })
}

/// ⭐ **改 `TransportConfig` 的通用内核**（⭐ 通行形状：任何"投影进快照的走带设置"都走它）。
///
/// 顺序（⭐ 负责人裁决 A 的 1–3 条）：① 抓现场 + **先 `stop`** ⇒ ② 改字段 + `reload`
/// ⇒ ③ **先恢复状态、后恢复位置**（⚠ 顺序反了会破坏位置：`play()` 自己会推进；
/// 早期版本把 tick 0 变成了 tick 10，实测踩过）。
fn apply_transport_config(
    host: &mut EngineHost,
    project: &YebanProjectV1,
    mutate: impl FnOnce(&mut YebanProjectV1),
) -> Result<MetronomeOutcome, EngineHostError> {
    let before = TransportSnapshot::from_reading(&host.transport());

    // ① **先 stop**：⛔ 不从"跑"直接换一代。
    host.stop();

    // ② 改字段 ⇒ 换一代引擎（这类设置都在**投影期**进快照，§模块文档）。
    let mut next = project.clone();
    mutate(&mut next);
    host.reload(&next, 0)?;

    // ③ **先恢复状态、后恢复位置**（见上）。
    match before.state {
        // ⭐ 原先在跑 ⇒ 还跑（位置马上在下一行拨回**原位置**）。
        TransportState::Playing => {
            host.play();
        }
        // ⭐ 原先停着 ⇒ 还停：`seek` 只改位置、**不改状态**（`SeekTicks` 的语义），
        // 而第 ① 步已经 `stop` 过 ⇒ 这里**什么都不用做**，状态本来就是停。
        //
        // ⚠ `Recording` 今天**不可达**（`TransportState::Recording` 是预留档、引擎侧没有
        // 入口）⇒ 它落进本支＝"当停档处理"，如实登记，⛔ 不假装支持。
        TransportState::Stopped | TransportState::Recording => {}
    }
    host.seek(before.position_ticks);

    let after_reading = host.transport();
    let after = TransportSnapshot::from_reading(&after_reading);
    Ok(MetronomeOutcome {
        before,
        after,
        enabled: next.transport.metronome_enabled,
        restore_drift_ticks: before.position_ticks.abs_diff(after.position_ticks),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLOCK_FRAMES: usize = 128;

    fn fixture() -> YebanProjectV1 {
        yeban_model::samples::filled_project()
    }

    /// ⭐ 推进 `quanta` 个量子并返回**累加的 tick 数**（⛔ 不读走带、只推进）。
    /// ⚠ **必须拼接全部量子**：早期版本每轮覆盖 ⇒ 只留下**最后一量子**（128 帧），
    /// 而 8 个量子覆盖 1024 帧 > 1 拍（960 tick @48k）⇒ 咔哒声发生在**前面**的量子里，
    /// 于是"开着的输出"看起来与关着的一模一样。实测踩过，⛔ 不要再覆盖。
    fn pump(host: &mut EngineHost, quanta: usize) -> Vec<f32> {
        let mut block = vec![0.0_f32; BLOCK_FRAMES * 2];
        let mut all = Vec::new();
        if let Some(runtime) = host.runtime_for_criteria() {
            for _ in 0..quanta {
                block.fill(0.0);
                runtime.process_quantum(&mut block, 2);
                all.extend_from_slice(&block);
            }
        }
        all
    }

    fn clicks(host: &mut EngineHost) -> u64 {
        host.runtime_for_criteria()
            .map_or(0, |runtime| runtime.stats().metronome_clicks)
    }

    /// ⭐ **判据 ①**：开节拍器 ⇒ `metronome_clicks` 由 0 变 >0，且**音频输出真的变了**。
    ///
    /// ⭐ **判据 ②（控制臂）**：关着 ⇒ 恒为 0。
    ///
    /// ⭐ **判据 ③**：逐字节比较开/关两种输出（⭐ `to_bits()`，⛔ 不是只看计数）。
    #[test]
    fn enabling_the_metronome_clicks_and_changes_the_audio_output() {
        let project = fixture();
        let mut host = EngineHost::new();
        host.reload(&project, 0).expect("建引擎");
        // 走带要在**跑**，否则节拍器不触发（渲染链只在推进时响）。
        host.play();

        // ---- 关着的时候（⭐ 控制臂本身在**独立判据** `the_control_arm_never_clicks`
        //      里，⛔ 不混在本条里 —— 否则控制臂无法被独立演示"仍绿"）----
        let off_audio = pump(&mut host, 8);
        assert!(
            !off_audio.iter().all(|s| *s == 0.0),
            "前置：工程必须有声音（否则'输出变了'就没意义）"
        );

        // ---- 开节拍器（换一代引擎 + 无损恢复走带）----
        let outcome = set_enabled(&mut host, &project, true).expect("开节拍器");
        assert!(outcome.enabled, "新快照的投影结果必须是开");
        let on_audio = pump(&mut host, 8);
        let on_clicks = clicks(&mut host);
        assert!(
            on_clicks > 0,
            "⭐ 判据 ①：开着并推进之后 metronome_clicks 必须 >0（实测 {on_clicks}）"
        );

        // ---- ⭐ 判据 ③：逐字节比较（⭐ to_bits，⛔ 不用 ==）----
        let off_bits: Vec<u32> = off_audio.iter().map(|s| s.to_bits()).collect();
        let on_bits: Vec<u32> = on_audio.iter().map(|s| s.to_bits()).collect();
        assert_ne!(
            off_bits, on_bits,
            "⭐ 判据 ③：开/关的音频输出必须**逐字节**不同（⛔ 不是只看计数）"
        );

        // ⭐ 判据 ③ 的反向控制：**再关一次** ⇒ 咔哒声**不再增加**。
        // ⚠ 不与"原来的关"逐字节比：那一次渲染之后位置已经前进 ⇒ 跨位置比是假判据。
        let _ = set_enabled(&mut host, &project, false).expect("关节拍器");
        let before_off_clicks = clicks(&mut host);
        let _ = pump(&mut host, 8);
        assert_eq!(
            clicks(&mut host),
            before_off_clicks,
            "关回去之后**不许再增咔哒声**（否则开关有残留）"
        );
    }

    /// ⭐ **控制臂（独立判据）**：⛔ **不拨开关** ⇒ `metronome_clicks` 必须恒为 0。
    ///
    /// ⚠ **为什么单独一条**（负责人要求 3、`mod-engine` 的教训逐字："控制臂要能独立
    /// 变红/变绿，否则它只是注释"）：若把控制臂与"开 ⇒ >0"写在**同一条**判据里，
    /// 一旦那条红，就**无法演示**"控制臂仍绿"—— 控制臂作为对照的作用当场消失。
    /// 拆开之后：注入"永远开"⇒ 本条红、① 绿；注入"永远关"⇒ 本条绿、① 红。
    #[test]
    fn the_control_arm_never_clicks() {
        let project = fixture();
        let mut host = EngineHost::new();
        host.reload(&project, 0).expect("建引擎");
        host.play();

        // ⭐ **必须走产品路径**（`set_enabled(false)`），⛔ 不能只 `reload` 原工程 ——
        // 那样测的是"引擎的默认值"，注入"永远写 true"**抓不到**（实测踩过：注入后本条仍绿）。
        let outcome = set_enabled(&mut host, &project, false).expect("显式关");
        assert!(!outcome.enabled, "关之后投影结果必须是关");
        assert_eq!(clicks(&mut host), 0, "前置：显式关 ⇒ 不许已经有咔哒声");
        let _ = pump(&mut host, 8);
        assert_eq!(
            clicks(&mut host),
            0,
            "⭐ 控制臂：工程里 metronome_enabled=false ⇒ metronome_clicks 必须恒为 0"
        );
        // ⭐ 再推进一批（⛔ 不是只看一次）—— 计数必须仍然不动。
        let _ = pump(&mut host, 16);
        assert_eq!(
            clicks(&mut host),
            0,
            "⭐ 控制臂：再推进 16 量子也必须仍然为 0"
        );
    }

    /// ⭐ **判据 ④**：走带现场**无损恢复** —— 位置 ＋ 状态都要一模一样。
    ///
    /// 参数化三种现场：停/tick 0、停在中间、跑在中间。
    #[test]
    fn toggling_the_metronome_preserves_the_transport_exactly() {
        let project = fixture();
        for (label, start_tick, start_playing) in [
            ("停在 0", 0_u64, false),
            ("停在中间", 3_840, false),
            ("跑在中间", 1_920, true),
        ] {
            let mut host = EngineHost::new();
            host.reload(&project, 0).expect("建引擎");
            if start_tick > 0 {
                host.seek(start_tick);
            }
            if start_playing {
                host.play();
            }
            let before = TransportSnapshot::from_reading(&host.transport());
            // ⚠ **前置也带那条代价**：`seek` 自己要 `pump(1)` ⇒ 它落在 start_tick **附近**
            // 而不是精确 start_tick（实测 3840 ⇒ 3845）。本断言只保证"确实到了那附近"。
            assert!(
                before.position_ticks.abs_diff(start_tick) <= MAX_RESTORE_DRIFT_TICKS,
                "{label}：前置位置应当落在 {start_tick} 附近（实测 {}）",
                before.position_ticks
            );

            let outcome = set_enabled(&mut host, &project, true).expect("开");
            // ⭐ **状态要求精确**：停还停、跑还跑。
            assert!(
                outcome.state_preserved(),
                "⭐ {label}：走带状态必须精确恢复（before={:?} after={:?}）",
                outcome.before,
                outcome.after
            );
            assert_eq!(
                outcome.after.state, before.state,
                "⭐ {label}：状态必须精确（停还停、跑还跑）"
            );
            // ⭐ **位置要求有界**（⚠ 不是"精确"：见模块文档的边界段与
            // `MAX_RESTORE_DRIFT_TICKS` 的注释 —— 控制驱动下 `pump` 会推进位置）。
            assert!(
                outcome.position_preserved_within_bound(),
                "⭐ {label}：位置漂移 {} tick 必须 ≤ {}（before={} after={}）",
                outcome.restore_drift_ticks,
                MAX_RESTORE_DRIFT_TICKS,
                before.position_ticks,
                outcome.after.position_ticks
            );
        }
    }
}

#[cfg(test)]
mod key_tests {
    use super::*;
    use crate::input::{Action, Focus, InputContext, LogicalKey, Modifiers, Resolution};

    fn fixture() -> YebanProjectV1 {
        yeban_model::samples::filled_project()
    }

    fn clicks(host: &mut EngineHost) -> u64 {
        host.runtime_for_criteria()
            .map_or(0, |runtime| runtime.stats().metronome_clicks)
    }

    fn pump(host: &mut EngineHost, quanta: usize) {
        let mut block = vec![0.0_f32; 128 * 2];
        if let Some(runtime) = host.runtime_for_criteria() {
            for _ in 0..quanta {
                block.fill(0.0);
                runtime.process_quantum(&mut block, 2);
            }
        }
    }

    /// ⭐ **判据 ①＋②**：`Cmd/Ctrl+Alt+N` 按下 ⇒ `metronome_clicks` 由 0 变 >0；
    /// **再按一次** ⇒ 回 0（往返）。
    ///
    /// ⚠ 判据**必须**同时覆盖"键被解析成哪个动作"与"那个动作真的拨动了引擎"：
    /// 只测前者 ⇒ 证明不了接线；只测后者 ⇒ 证明不了键。
    #[test]
    fn the_metronome_key_starts_and_stops_the_clicks() {
        let project = fixture();
        let mut host = EngineHost::new();
        host.reload(&project, 0).expect("建引擎");
        host.play();

        // ---- ① 键按下（走**真** `InputContext::resolve`）----
        let mut input = InputContext::default();
        input.set_focus(Focus::MainCanvas);
        assert_eq!(
            input.resolve_logical(LogicalKey::Character('n'), Modifiers::ctrl_alt()),
            Resolution::Action(Action::ToggleMetronome),
            "⭐ `Cmd/Ctrl+Alt+N` 必须解析成 `ToggleMetronome`"
        );
        let applied =
            crate::host::apply_action_for_criteria(&mut host, &project, Action::ToggleMetronome);
        assert!(applied, "⭐ 键必须被**消费**（接线成功）");
        pump(&mut host, 8);
        let on = clicks(&mut host);
        assert!(
            on > 0,
            "⭐ 判据 ①：按一次键之后 clicked 必须 >0（实测 {on}）"
        );

        // ---- ② 再按一次（往返，带回当前工程状态）----
        let mut next = project.clone();
        next.transport.metronome_enabled = true;
        let applied =
            crate::host::apply_action_for_criteria(&mut host, &next, Action::ToggleMetronome);
        assert!(applied, "⭐ 第二次按也必须被消费");
        let before = clicks(&mut host);
        pump(&mut host, 16);
        assert_eq!(
            clicks(&mut host),
            before,
            "⭐ 判据 ②：关回去之后⛔ 不许再增咔哒声（往返）"
        );
    }

    /// ⭐ **判据 ③（控制臂，走产品路径；第 46 条）**：⛔ **不按键** ⇒ 恒为 0。
    ///
    /// ⚠ 独立判据（⛔ 不与 ① 同条）：否则 ① 一红就**无法演示**"控制臂仍绿"。
    #[test]
    fn the_key_control_arm_never_clicks() {
        let project = fixture();
        let mut host = EngineHost::new();
        host.reload(&project, 0).expect("建引擎");
        host.play();
        // ⭐ 走产品路径把开关**显式关掉**（⛔ 不能只 reload 原工程 —— 那样测的是引擎默认值，
        // 注入"永远写 true"抓不到它；本仓已有过这个教训）。
        assert!(
            crate::host::apply_action_for_criteria(&mut host, &project, Action::ToggleMetronome),
            "先开一次（走产品路径）"
        );
        // 再走产品路径关掉 ⇒ 之后⛔ 不许再有咔哒声。
        let mut on = project.clone();
        on.transport.metronome_enabled = true;
        assert!(crate::host::apply_action_for_criteria(
            &mut host,
            &on,
            Action::ToggleMetronome
        ));
        let before = clicks(&mut host);
        // ⚠ **必须推进至少一整拍**：节拍器**在拍点立刻响一次**，而 960 tick/拍 ÷ ≈5 tick/量子
        // ≈ **192 量子/拍** ⇒ 早期版本只 `pump(16)`，**根本覆盖不到第二拍**，
        // 于是"注入永远开"也看不出差别（实测踩过）。
        pump(&mut host, 400);
        assert_eq!(
            clicks(&mut host),
            before,
            "⭐ 控制臂：产品路径关掉之后，再推进 ≥2 拍也必须**不再**增咔哒声"
        );
    }

    /// ⭐ **判据 ④**：按键拨动也必须**精确恢复走带状态**（位置按 16-tick 封顶如实报）。
    #[test]
    fn the_metronome_key_preserves_the_transport_state() {
        let project = fixture();
        for (label, start_tick, playing) in [
            ("停在 0", 0_u64, false),
            ("停在中间", 3_840, false),
            ("跑在中间", 1_920, true),
        ] {
            let mut host = EngineHost::new();
            host.reload(&project, 0).expect("建引擎");
            if start_tick > 0 {
                host.seek(start_tick);
            }
            if playing {
                host.play();
            }
            let before = TransportSnapshot::from_reading(&host.transport());
            assert!(
                crate::host::apply_action_for_criteria(
                    &mut host,
                    &project,
                    Action::ToggleMetronome
                ),
                "{label}：键必须被消费"
            );
            let after = TransportSnapshot::from_reading(&host.transport());
            assert_eq!(
                after.state, before.state,
                "⭐ {label}：走带状态必须**精确**恢复（停还停、跑还跑）"
            );
            let drift = before.position_ticks.abs_diff(after.position_ticks);
            assert!(
                drift <= MAX_RESTORE_DRIFT_TICKS,
                "⭐ {label}：位置漂移 {drift} tick 必须 ≤ {MAX_RESTORE_DRIFT_TICKS}"
            );
        }
    }
}
