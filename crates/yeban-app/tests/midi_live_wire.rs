//! `--midi-in` 的**实时腿**（连续性）在**无 MIDI 硬件**条件下的判据。
//!
//! ## 这条腿接在哪（⭐ 判据要判的就是它）
//!
//! ```text
//!   工作线程（会话开着）: midi_in::monitor_session  ⇒  每 tick: midi_live::pump_once_with
//!                          ├─ 谱面那一半: 每一条原始行记进 CapturedMessages（照旧）
//!                          └─ 声音那一半: 映射成 EngineEvent ⇒ midi_in::LiveLane（Send）
//!   UI 线程（60Hz 心跳）: host::MidiInState::publish_live ⇒ EngineHost::send_engine_events
//!                          ⇒ 引擎的音符路径 ⇒ 合成器
//! ```
//!
//! ⭐ **本文件为什么不需要硬件**：
//! 1. 会话循环对"源"的要求收成了一个 trait（[`midi_live::LiveEventSource`]）⇒ ⭐ 本文件喂一个
//!    **时间脚本化的桩源**，驱动的却是**产品用的那一个**函数（[`midi_in::monitor_session`]）；
//! 2. UI 线程那一半（[`MidiInState::publish_live`]）本身就不碰硬件 ⇒ ⭐ 直接判。
//!
//! ⛔ 本文件**不**打开端口、⛔ 不依赖真设备。⚠ 剩下那一段（`midir` 真回调线程）**只能**在
//! 有 MIDI 硬件的机器上验 ⇒ 见回报里的硬件清单（⛔ 不在本文件里假装判过）。

use yeban_app::engine_host::EngineHost;
use yeban_app::host::MidiInState;
use yeban_engine::ring::EngineEvent;
use yeban_model::project::{ClipContent, YebanProjectV1};

/// 渲染 / 发布的声道数（与 `midi_input_notes.rs` 同口径）。
const CHANNELS: u16 = 2;

/// ⭐ 一个**没有音符**的工程：音轨 / 乐器 / 摆放都在，只是片段里**一个音符都没有**。
///
/// 为什么要它：引擎若自带排程音符，"这些事件造成了声音"就**不再可判**（⭐ 判据会空转）。
fn without_notes() -> YebanProjectV1 {
    let mut project = yeban_app::bridge::demo_project();
    for entry in project.clip_pool.values_mut() {
        if let ClipContent::Midi { notes } = &mut entry.content {
            notes.clear();
        }
    }
    project
}

/// ⭐ **一个发布批**的读数：距离会话起点的毫秒数、这一批几条、发完之后合成器触发过的**音符号**。
///
/// ⚠ `cfg` 门与 `session` 模块一致：默认档里连"已解析的 MIDI 事件"这个类型都不在依赖图上
/// ⇒ 也没人读这个别名（⛔ 不留一个只被 cfg 掉的代码使用的别名）。
#[cfg(feature = "midi-input")]
type BatchReading = (u128, usize, u64);

/// ⭐ 判据 ①（**UI 线程那一半：车道 ⇒ 引擎的音符路径**，行为；⭐ 默认档就跑）。
///
/// 它判的是**产品路径上真实的发布点**（[`MidiInState::publish_live`] —— `main.rs` 的 60Hz
/// 心跳里调的就是它），不是本文件里另写的一段：
///
/// * ⭐ **控制臂先滚**：车道空 ⇒ 发布 0 条、合成器读数**一位不动**（否则"事件造成了触发"
///   不可判 ⇒ 判据会空转）；
/// * ⭐ 事件臂：会话线程往车道放 2 条 `NoteOn` ⇒ 一次发布**全部**进引擎环（`2/2`）＋ 车道清空
///   ＋ 渲染后合成器器件计数 **+2** ＋ 输出**非静音**。
#[test]
fn the_live_lane_publishes_into_the_engines_note_path() {
    let project = without_notes();
    let track = yeban_app::midi_notes::default_track(&project).expect("演示夹具必须有音轨");
    let state = MidiInState::new();
    let lane = state.live_lane();

    let mut host = EngineHost::new();
    host.reload(&project, 0).expect("引擎必须能建起来");
    let block = EngineHost::quantum_frames();
    let mut scratch = vec![0.0_f32; block * usize::from(CHANNELS)];
    // ⭐ 预热量子：与批次路同一条引擎行为（"先出队事件、后武装快照"⇒ 冷引擎第一块会丢事件）。
    host.render_into(&mut scratch, CHANNELS, 1);
    let before = host
        .engine_stats()
        .expect("控制驱动档必须有引擎读数")
        .poly_notes_triggered;

    // ---- ① 控制臂：车道空 ----
    assert_eq!(
        state.publish_live(&mut host),
        0,
        "空车道 ⇒ 一条都不许发布（否则这个判据只是在测'发布函数被调用过'）"
    );
    assert_eq!(
        host.engine_stats()
            .expect("控制驱动档必须有引擎读数")
            .poly_notes_triggered,
        before,
        "⭐ 控制臂：没有事件 ⇒ 合成器**一位不动**"
    );

    // ---- ② 事件臂：会话线程（这里由判据扮演）把**映射好的**事件放进车道 ----
    let accepted = lane.push(&[
        EngineEvent::NoteOn {
            track,
            pitch: 60,
            velocity: 100,
        },
        EngineEvent::NoteOn {
            track,
            pitch: 64,
            velocity: 90,
        },
    ]);
    assert_eq!(accepted, 2, "车道装得下两条");
    assert_eq!(state.publish_live(&mut host), 2, "两条都必须进引擎环");
    assert_eq!(
        lane.pending(),
        0,
        "发布之后车道必须**清空**（否则下一条心跳会把同一批再发一次）"
    );

    // ---- ③ 引擎真的消费了它们（⭐ 器件侧计数 = 见证，不是'我调用过发布'）----
    let mut out = vec![0.0_f32; block * usize::from(CHANNELS)];
    assert_eq!(host.render_into(&mut out, CHANNELS, 1), 1);
    let after = host
        .engine_stats()
        .expect("控制驱动档必须有引擎读数")
        .poly_notes_triggered;
    assert_eq!(
        after.saturating_sub(before),
        2,
        "⭐ 两条 `NoteOn` 必须各触发一次合成器器件（读数为 0 ⇒ 车道没接上引擎的音符路径）"
    );
    let nonzero = out.iter().filter(|sample| **sample != 0.0).count();
    let peak = out.iter().fold(0.0_f32, |max, s| max.max(s.abs()));
    println!(
        "[midi-live-wire] 车道 ⇒ 引擎: published=2/2 · poly-notes-triggered {before}→{after} · \
         nonzero={nonzero} peak={peak:.6} · 控制臂 poly 稳在 {before}"
    );
    assert!(
        nonzero > 0 && peak > 0.0,
        "⭐ 弹一下必须**有声音**（非零样本 {nonzero} / 峰值 {peak}）"
    );
}

/// ⭐ 判据 ②–③（**会话循环：到达即泵**，行为）—— ⚠ 只在 `midi-input` 特性下存在。
///
/// ⚠ **为什么有 `cfg` 门**：`yeban-midi-io` 是 app 的**可选依赖**（`midi-input` 才拉进来）
/// ⇒ 默认档里连"已解析的 MIDI 事件"这个类型都不在依赖图上 ⇒ 这些判据在默认档里**不存在**
/// （⛔ 不是"被跳过"）。
#[cfg(feature = "midi-input")]
mod session {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::time::{Duration, Instant};

    use yeban_app::midi_in::{self, CapturedMessages};
    use yeban_app::midi_live::{LiveEventSource, LivePumpReport, MidiEvent};
    use yeban_model::EntityId;

    use super::{BatchReading, CHANNELS, EngineHost};

    /// ⭐ **时间脚本化的桩源**：每条事件带一个"到期毫秒"，⛔ 没到期就**不**给。
    ///
    /// ⭐ 它就是"真 `midir` 回调线程"在本机的替身 —— 而 `pop` 的另一侧（泵身、映射、批量、
    /// 顺序、计数）**全是产品代码**（[`midi_in::monitor_session`] 直接调
    /// `midi_live::pump_once_with`）。
    struct ScriptedSource {
        /// `(到期毫秒, 事件)`，按到期时间升序。
        script: Vec<(u64, MidiEvent)>,
        /// 会话起点。
        started: Instant,
        /// 下一条还没给出的事件下标。
        cursor: usize,
    }

    impl ScriptedSource {
        fn new(started: Instant, script: Vec<(u64, MidiEvent)>) -> Self {
            Self {
                script,
                started,
                cursor: 0,
            }
        }
    }

    impl LiveEventSource for ScriptedSource {
        fn pop(&mut self) -> Option<MidiEvent> {
            let (due_ms, event) = *self.script.get(self.cursor)?;
            if Instant::now() < self.started + Duration::from_millis(due_ms) {
                return None;
            }
            self.cursor += 1;
            Some(event)
        }

        fn dropped_events(&self) -> u64 {
            // ⭐ 桩源不丢东西（真源的"环满 ⇒ 计数"由 `midi_live::tests` 与
            // `midi_input_notes.rs` 的判据⑪钉住）。
            0
        }

        fn unsupported_messages(&self) -> u64 {
            0
        }
    }

    fn event(status: u8, data1: u8, data2: u8) -> MidiEvent {
        MidiEvent {
            timestamp_us: 0,
            status,
            data1,
            data2,
        }
    }

    /// ⭐ 时间脚本（⚠ 间隔 25 ms ≫ 控制 tick 2 ms ⇒ 每条事件都落在**不同**的 tick 上）：
    ///
    /// * `0 ms` C4 按下（力度 100）⇒ 触发 1 号器件；
    /// * `25 ms` E4 按下（力度 90）⇒ 触发 2 号器件；
    /// * `50 ms` C4 松开（**`0x90` 力度 0** ⇒ 必须是 NoteOff，⛔ 不是第三条 NoteOn）；
    /// * `75 ms` CC7（**非音符** ⇒ 计数忽略）；
    /// * `100 ms` E4 松开（`0x80`）。
    ///
    /// ⚠ 窗口 = 200 ms ⇒ 脚本在窗口**之内**就全部到期（⭐ 判据要判的正是"窗口**之内**"）。
    fn script() -> Vec<(u64, MidiEvent)> {
        vec![
            (0, event(0x90, 60, 100)),
            (25, event(0x90, 64, 90)),
            (50, event(0x90, 60, 0)),
            (75, event(0xB0, 7, 100)),
            (100, event(0x80, 64, 0)),
        ]
    }

    /// ⭐ 驱动**产品的那一个**会话循环（`midi_in::monitor_session`）：桩源 ＋ 真 `EngineHost`
    /// 当发布落点（`publish` 闭包 = `EngineHost::send_engine_events` ＋ 推一个量子让引擎**真的
    /// 消费**这一批），并把每一次发布的读数记下来。
    fn drive(
        host: &mut EngineHost,
        track: EntityId,
        script: Vec<(u64, MidiEvent)>,
        seconds: f32,
    ) -> (CapturedMessages, LivePumpReport, Vec<BatchReading>, f32) {
        let started = Instant::now();
        let mut source = ScriptedSource::new(started, script);
        let log: Rc<RefCell<Vec<BatchReading>>> = Rc::new(RefCell::new(Vec::new()));
        let peak = Rc::new(Cell::new(0.0_f32));
        let block = EngineHost::quantum_frames();
        let mut scratch = vec![0.0_f32; block * usize::from(CHANNELS)];
        let log_sink = Rc::clone(&log);
        let peak_sink = Rc::clone(&peak);
        let (captured, report) = midi_in::monitor_session(
            &mut source,
            track,
            seconds,
            |batch| {
                let accepted = host.send_engine_events(batch);
                // ⭐ 设备回调在本机不存在 ⇒ 控制侧推**一个**量子，让这一批在**这一拍**就作用到合成器
                // （口径与 `midi_input_notes.rs` 的实时形状判据一致）。
                let _ = host.render_into(&mut scratch, CHANNELS, 1);
                let poly = host
                    .engine_stats()
                    .map_or(0, |stats| stats.poly_notes_triggered);
                let local_peak = scratch.iter().fold(0.0_f32, |max, s| max.max(s.abs()));
                peak_sink.set(peak_sink.get().max(local_peak));
                log_sink
                    .borrow_mut()
                    .push((started.elapsed().as_millis(), batch.len(), poly));
                accepted
            },
            |_timestamp_us, _status, _data1, _data2| {
                // ⭐ 本判据只关心引擎那一半 ⇒ 旁路是**空**的（音符车道另有判据）。
            },
        );
        (captured, report, log.borrow().clone(), peak.get())
    }

    /// ⭐ 判据 ②：**到达即泵**（⭐ 本票的**核心**判据）。
    ///
    /// 读的是"发布**在窗口之内**发生了几次、每次之后合成器触发了几号器件"：
    ///
    /// * ⭐ 每一次发布都带 `poly` ⇒ 合成器是在**窗口之内**被触发的（⛔ 不是窗口结束时一次性
    ///   才听见）；
    /// * ⭐ 发布次数 ≥ 3（⭐ 若实现退化成"窗口结束再泵一次"，这里是 **1**）；
    /// * ⭐ 第一次发布的时刻 ≪ 窗口长度（⭐ 退化成批次的实现会把唯一一次发布放在**窗口末尾**）；
    /// * ⭐ 谱面那一半拿到**每一条**原始消息（`0x90` 力度 0 也算一条）。
    #[test]
    fn the_session_reaches_the_synth_while_the_window_is_open() {
        let project = super::without_notes();
        let track = yeban_app::midi_notes::default_track(&project).expect("有音轨");
        let mut host = EngineHost::new();
        host.reload(&project, 0).expect("引擎必须能建起来");
        let block = EngineHost::quantum_frames();
        let mut warmup = vec![0.0_f32; block * usize::from(CHANNELS)];
        host.render_into(&mut warmup, CHANNELS, 1);
        let poly_before = host
            .engine_stats()
            .expect("控制驱动档必须有引擎读数")
            .poly_notes_triggered;

        let window_ms = 200_u128;
        let (captured, report, log, peak) = drive(&mut host, track, script(), 0.2);
        let poly_after = host
            .engine_stats()
            .expect("控制驱动档必须有引擎读数")
            .poly_notes_triggered;

        let summary = report.summary();
        println!(
            "[midi-live-wire] 会话循环: {summary} | 窗口 {window_ms}ms · 发布 {} 次 {log:?} | \
             poly-notes-triggered {poly_before}→{poly_after} | peak={peak:.6} | \
             谱面行数={}",
            log.len(),
            captured.messages.len()
        );

        // ---- (a) 映射与计数（⭐ 逐条，不是只看总数）----
        assert_eq!(report.drained, 5, "{summary}");
        assert_eq!(report.note_ons, 2, "{summary}");
        assert_eq!(
            report.note_offs, 2,
            "⭐ `0x90` 力度 0 与 `0x80` **都**是 NoteOff: {summary}"
        );
        assert_eq!(
            report.ignored, 1,
            "⭐ CC7 必须被**计数**，⛔ 不静默丢: {summary}"
        );
        assert_eq!(report.published, report.mapped(), "{summary}");
        assert_eq!(report.refused, 0, "{summary}");

        // ---- (b) ⭐ 到达即泵：窗口**之内**就发布了 ----
        assert!(
            log.len() >= 3,
            "⭐ 每条事件到期后**各自**发布（≥3 次）；发布 {} 次 ⇒ 形状已退化成批次: {log:?}",
            log.len()
        );
        assert_eq!(
            log.len(),
            report.publishes,
            "⭐ `publishes` 必须等于真的发布调用次数: {summary}"
        );
        let first_ms = log.first().expect("至少发布过一次").0;
        assert!(
            first_ms * 2 <= window_ms,
            "⭐ 第一次发布必须发生在窗口**前半段**（实测 {first_ms} ms / 窗口 {window_ms} ms）\
             —— 退化成'窗口结束再泵'的实现会把它放在末尾: {log:?}"
        );

        // ---- (c) ⭐ 合成器在窗口之内就被触发了（⭐ 这是"声音有反应"的见证）----
        assert_eq!(
            log.first().expect("发布过").2,
            1,
            "⭐ 第一批发布之后合成器必须已经触发过 1 号器件: {log:?}"
        );
        assert_eq!(
            log.get(1).expect("至少发布两次").2,
            2,
            "⭐ 第二批之后必须已经触发 2 号器件（⭐ 两条 NoteOn，⛔ 不是 `0x90` 力度 0 被当成第三条）: {log:?}"
        );
        assert_eq!(
            poly_after.saturating_sub(poly_before),
            2,
            "⭐ 两条 `NoteOn` ⇒ 合成器器件侧恰好 +2: {log:?}"
        );
        assert!(peak > 0.0, "⭐ 弹一下必须**有声音**（峰值 {peak}）");

        // ---- (d) 谱面那一半：**每一条**原始消息都在（⭐ 与本票之前逐字一样）----
        assert_eq!(
            captured.messages.len(),
            5,
            "谱面那一半必须拿到每一条消息（含 CC7）: {:?}",
            captured.messages
        );
        assert_eq!(
            captured
                .messages
                .iter()
                .map(|row| (row.1, row.2, row.3))
                .collect::<Vec<_>>(),
            vec![
                (0x90, 60, 100),
                (0x90, 64, 90),
                (0x90, 60, 0),
                (0xB0, 7, 100),
                (0x80, 64, 0),
            ],
            "⭐ 到达顺序必须原样保留"
        );
    }

    /// ⭐ 判据 ③（**反向对照**）：同一个函数、同一次装配、**一条事件都不给** ⇒
    /// 一次发布都没有、合成器**一位不动**、输出**全零**。
    ///
    /// ⭐ 它是判据 ② 的**空转闸门**：少了它，"发布了好几次 + 有声音"有可能只是引擎自己在响
    /// （那正是本仓记过的"看起来有"的错误结论）。
    #[test]
    fn the_session_publishes_nothing_and_stays_silent_without_events() {
        let project = super::without_notes();
        let track = yeban_app::midi_notes::default_track(&project).expect("有音轨");
        let mut host = EngineHost::new();
        host.reload(&project, 0).expect("引擎必须能建起来");
        let block = EngineHost::quantum_frames();
        let mut warmup = vec![0.0_f32; block * usize::from(CHANNELS)];
        host.render_into(&mut warmup, CHANNELS, 1);
        let poly_before = host
            .engine_stats()
            .expect("控制驱动档必须有引擎读数")
            .poly_notes_triggered;

        let (captured, report, log, peak) = drive(&mut host, track, Vec::new(), 0.1);

        println!(
            "[midi-live-wire] 对照档: {} | 发布 {} 次 | poly {poly_before}→{} | peak={peak}",
            report.summary(),
            log.len(),
            host.engine_stats()
                .expect("控制驱动档必须有引擎读数")
                .poly_notes_triggered
        );
        assert!(log.is_empty(), "⭐ 没有事件 ⇒ ⛔ 一次都不许发布: {log:?}");
        assert_eq!(report.publishes, 0, "{}", report.summary());
        assert_eq!(report.published, 0, "{}", report.summary());
        assert_eq!(report.drained, 0, "{}", report.summary());
        assert!(captured.messages.is_empty(), "谱面那一半也必须是空的");
        assert_eq!(
            host.engine_stats()
                .expect("控制驱动档必须有引擎读数")
                .poly_notes_triggered,
            poly_before,
            "⭐ 对照档：合成器**一位不动**"
        );
        assert_eq!(peak, 0.0, "⭐ 对照档：输出必须**全零**");
    }

    /// ⭐ **收口判据**：走**产品的那一个**会话循环（`midi_in::monitor_session`），
    /// 一条消息都**不漏**地喂给旁路，且**主路不变**。
    ///
    /// 为什么必须有：接线前的行为是"消息只进谱面/引擎"，接线后多了一个旁路消费者。
    /// 本判据证明两件事：① 旁路**看到了每一条**（`drained` == 消息数）；
    /// ② 主路（`messages`）与**不使用旁路**时的结果**逐条相同** ⇒ 旁路不影响主路。
    ///
    /// ⚠ 旁路是**泛型闭包**（⛔ 不是 `Box<dyn FnMut>`）⇒ 会话线程上不多一次分配。
    #[test]
    fn the_session_bypass_sees_every_message_and_does_not_change_the_main_path() {
        use yeban_app::midi_notes::IncrementalNoteRecorder;

        // ---- 参考臂：旁路是空的 ----
        let project = super::without_notes();
        let track = yeban_app::midi_notes::default_track(&project).expect("夹具必须有音轨");
        let reference = {
            let mut host = EngineHost::new();
            host.reload(&project, 0).expect("引擎必须能建起来");
            let started = Instant::now();
            let mut source = ScriptedSource::new(started, script());
            let (captured, _report) = midi_in::monitor_session(
                &mut source,
                track,
                0.5,
                |batch| host.send_engine_events(batch),
                |_timestamp_us, _status, _data1, _data2| {},
            );
            captured.messages
        };

        // ---- 实测臂：旁路＝真记录器（产品的那一个） ----
        let mut host = EngineHost::new();
        host.reload(&project, 0).expect("引擎必须能建起来");
        let started = Instant::now();
        let mut source = ScriptedSource::new(started, script());
        let drained = Rc::new(Cell::new(0_usize));
        let sink = Rc::clone(&drained);
        let mut recorder = IncrementalNoteRecorder::new(0);
        let (captured, report) = midi_in::monitor_session(
            &mut source,
            track,
            0.5,
            |batch| host.send_engine_events(batch),
            |timestamp_us, status, data1, data2| {
                sink.set(sink.get() + 1);
                let _ = recorder.consume(timestamp_us, status, data1, data2);
            },
        );

        // ① 旁路看到了**每一条**（⭐ 与泵自己的 drained 逐数相同）。
        assert_eq!(
            drained.get(),
            report.drained,
            "旁路必须与泵看到同样多的消息"
        );
        assert!(drained.get() > 0, "本判据的脚本必须真的产出消息");
        // ② 主路**逐条不变**（音符那半）。
        assert_eq!(
            captured.messages, reference,
            "接线后 `CapturedMessages` 必须与不接线时逐条相同"
        );
        assert_eq!(captured.seconds, 0.5);
    }
}
