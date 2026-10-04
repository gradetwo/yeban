//! `line/engine-sound` 的端到端判据：**引擎真的出声了**
//! [ARCH-RT-001, ARCH-RT-004, ARCH-DET-001, ROAD-M2-005, ROAD-M2-006]。
//!
//! 全部判据在 **`--no-default-features`（不编译 cpal）** 下运行 —— 这是本机与 CI
//! 的主路径，也是"无设备验证"的具体含义：不需要声卡，只要能渲染量子。
//!
//! 判据编号（`J*`）与 `docs/ledger/engine-sound-notes.md` 的判据表一一对应：
//!
//! | 编号 | 判据 | 怎么变红 |
//! | :--- | :--- | :--- |
//! | J1 | 含音符的工程渲染后**不是全零** | 渲染回退成常量/静音 |
//! | J2 | 同输入两次渲染**逐位相同** | 引入真熵源/未初始化状态 |
//! | J3 | 音符起点/终点对应的样本位置误差 ≤ 1 量子 | tick→sample 换算错、包络不归零 |
//! | J4 | 力度 0 不发声；力度越大有效值越大（严格单调 + 线性比） | 忽略力度 / 力度取反 |
//! | J5 | （独立目标 `synth_rt_zero_alloc`）零分配窗口仍成立 | 实时路径里分配 |
//! | J6 | 空工程/无音符工程/纯音频片段 ⇒ 静音且不 panic | 越界/panic/噪声底 |
//! | J7 | 升八度让零交叉数翻倍（音高正确） | 相位累加每样本重置 |
//! | J8 | 等价快照重新武装**不重触发**、不改变任何样本位 | 切换时重置声部/游标 |
//! | J9 | 模型 → 快照 → 采样位置的覆盖度（手算栅格） | 片段/摆放/微时值投影漏掉 |
//! | J10 | 超过调度表容量时计数丢弃、不 panic | 无界增长 / panic |

mod support;

use support::{
    NoteSpec, audio_clip_project, bare_track_project, empty_project, note_project, render,
    render_with, rms_peak, zero_crossings,
};
use yeban_engine::snapshot::EngineSnapshot;
use yeban_engine::synth::MAX_NOTES_PER_TRACK;

/// 夹具栅格：120 BPM / 48 kHz / 960 PPQ ⇒ 1 tick = 25 样本。
const SAMPLES_PER_TICK: u64 = 25;

/// J1：**含音符的工程 → 渲染 N 个量子 → 输出不是全零**。
///
/// 打印机械证据：非零样本数 + 位模式指纹 + 关键统计。
#[test]
fn project_with_notes_produces_audible_samples() {
    let fixture = note_project(&[
        NoteSpec::at(0, 960, 60, 100),
        NoteSpec::at(960, 960, 64, 100),
        NoteSpec::at(1920, 960, 67, 100),
    ]);
    let rendered = render(&fixture.project, 400);

    println!(
        "[engine-sound] J1 frames={} nonzero={} peak={:.6} fingerprint=0x{:016x} \
         scheduled_notes={} notes_triggered={} rendered_samples={}",
        rendered.frames(),
        rendered.nonzero(),
        rendered.peak(),
        rendered.fingerprint(),
        rendered.stats.scheduled_notes,
        rendered.stats.notes_triggered,
        rendered.stats.rendered_samples,
    );

    assert!(
        rendered.nonzero() > 0,
        "含 3 个 MIDI 音符的工程渲染 400 个量子后必须**不是**全零 —— \
         这正是本次切片要消灭的『Nothing is playable yet』"
    );
    assert!(
        rendered.nonzero() > 10_000,
        "非零样本数太少（{}），信号路径可疑",
        rendered.nonzero()
    );
    assert_eq!(rendered.stats.scheduled_notes, 3, "三个音符都要进调度表");
    assert_eq!(
        rendered.stats.rendered_samples,
        400 * 128,
        "播放头必须与已渲染帧数一致"
    );
    assert_eq!(rendered.stats.voice_steals, 0, "3 个音符远未触及复音上限");
    assert!(
        rendered.peak() > 0.1,
        "峰值 {} 太小，增益路径可疑",
        rendered.peak()
    );
    // 峰值上界只断言"两个声部叠加"的量级：三个音符虽然首尾相接，
    // 但前一个的 50 ms 释放尾会与后一个的起音**重叠**，于是样本瞬时相加。
    // 因此 1.0058 这种"刚刚越满刻度"是**预期之内**的 —— 本切片**没有母线限制器**
    // （`ARCH-DSP-001` 的限幅器尚未接入，见 notes 的 pending），
    // 上界取 2.0（两个声部各自 ≤ 1.0）。这条断言抓的是"增益/包络量级写错"。
    assert!(
        rendered.peak() <= 2.0,
        "峰值 {} 超过两个声部叠加的上界 2.0",
        rendered.peak()
    );
}

/// J2：同输入两次渲染**逐位相同**（L1 契约的实现证据）。
#[test]
fn same_input_renders_byte_identical() {
    let fixture = note_project(&[
        NoteSpec::at(0, 480, 60, 127),
        NoteSpec::at(960, 960, 67, 64),
        NoteSpec::at(2400, 480, 72, 32),
    ]);
    let first = render(&fixture.project, 200);
    let second = render(&fixture.project, 200);

    assert!(first.nonzero() > 0, "对照渲染必须真的出声");
    assert_eq!(
        first.left_bits(),
        second.left_bits(),
        "两次独立装配 + 渲染必须逐位相同"
    );
    assert_eq!(first.right, second.right, "右声道同样必须逐位相同");
    assert_eq!(
        first.fingerprint(),
        second.fingerprint(),
        "指纹必须一致: 0x{:016x} vs 0x{:016x}",
        first.fingerprint(),
        second.fingerprint()
    );
}

/// J3：一个音符的**起点/终点**对应到样本位置的误差 ≤ 1 个量子。
///
/// 手算栅格：起点 tick 960 → 24000 样本；时值 480 tick → 12000 样本 ⇒ 终点 36000。
#[test]
fn note_onset_and_offset_land_within_one_quantum() {
    let fixture = note_project(&[NoteSpec::at(960, 480, 69, 127)]);
    let rendered = render(&fixture.project, 400);

    let onset = 960 * SAMPLES_PER_TICK;
    let end = onset + 480 * SAMPLES_PER_TICK;
    // 释放上界：`Adsr` 的 release 系数让 `release_s` 走完 6 个时间常数
    // （0.05 s @48k = 2400 帧 ⇒ τ = 400 帧），包络值 < 1e-4 时归零；
    // 从 sustain 0.7 起算需 ≈8.85 τ ⇒ 取 10 τ + 1 个量子作为"必然归零"的位置。
    let release_tail = 10 * (0.05 * 48_000.0 / 6.0) as usize;
    let tail = end as usize + release_tail + 128;

    assert_eq!(onset, 24_000);
    assert_eq!(end, 36_000);

    assert!(
        rendered.left[..onset as usize].iter().all(|s| *s == 0.0),
        "起点之前必须**逐位**静音（起点误差必须 ≤ 1 量子）"
    );
    let onset_window = &rendered.left[onset as usize..(onset + 128) as usize];
    assert!(
        onset_window.iter().any(|s| *s != 0.0),
        "起点必须落在同一个量子（128 帧）之内"
    );
    let held_window = &rendered.left[(end as usize - 128)..end as usize];
    assert!(
        held_window.iter().any(|s| *s != 0.0),
        "终点之前仍在发声（音符不是提前结束）"
    );
    assert!(
        rendered.left[tail..].iter().all(|s| *s == 0.0),
        "释放走完（终点 + 50 ms + 1 量子）之后必须**逐位**归零"
    );
}

/// J4：力度 0 不发声；力度越大有效值越大（**严格单调**），且与力度成**线性**比。
///
/// 容差与理由：增益路径在实数上是精确线性的（`velocity/127` 一次乘），
/// 因此 RMS 之比应当等于力度之比；这里给 **1e-3 相对容差**，
/// 吸收 `f32` 增益乘法的舍入与 `f64` 求和的累加误差（实测相对偏差 < 1e-6，
/// 容差比它宽 3 个数量级，而"忽略力度"这种注入会让比值变成 1.0 ⇒ 照样红）。
#[test]
fn velocity_gate_and_monotonic_loudness() {
    let velocities = [0u8, 1, 32, 64, 127];
    let mut rms = Vec::new();
    for velocity in velocities {
        let fixture = note_project(&[NoteSpec::at(0, 960, 69, velocity)]);
        let rendered = render(&fixture.project, 120);
        let (value, _) = rms_peak(&rendered.left);
        rms.push(value);
    }

    println!("[engine-sound] J4 velocity={velocities:?} rms={rms:?}");
    assert_eq!(rms[0], 0.0, "力度 0 的音符必须**完全**不发声");
    for pair in rms.windows(2) {
        assert!(
            pair[1] > pair[0],
            "有效值必须随力度严格单调: {rms:?}（忽略力度会让这一条变红）"
        );
    }

    // 线性比：RMS(127) / RMS(1) 必须 ≈ 127（容差 1e-3 相对）
    let ratio = rms[4] / rms[1];
    let expected = 127.0f64;
    assert!(
        (ratio - expected).abs() / expected < 1e-3,
        "力度到增益必须是线性的: RMS 比 {ratio} 与力度比 {expected} 相差超过 1e-3 相对容差"
    );
    // RMS(64) / RMS(32) 同理（同一轨道参数、同一包络 ⇒ 只有力度不同）
    let ratio_mid = rms[3] / rms[2];
    assert!(
        (ratio_mid - 2.0).abs() < 2.0 * 1e-3,
        "力度 64 与 32 的有效值之比应为 2, 实际 {ratio_mid}"
    );
}

/// J6：空工程 / 无音符工程 / 纯音频片段工程 ⇒ 输出静音且**不 panic**。
#[test]
fn empty_and_noteless_projects_render_silence_without_panic() {
    // (a) 只有主总线、没有任何轨道
    let empty = render(&empty_project(), 32);
    assert_eq!(empty.nonzero(), 0, "空工程必须逐位静音");
    assert_eq!(empty.stats.quanta, 32);
    assert_eq!(empty.stats.scheduled_notes, 0);

    // (b) 有轨道、有 MIDI 片段但**没有音符**
    let fixture = bare_track_project();
    let bare = render(&fixture.project, 32);
    assert_eq!(bare.nonzero(), 0, "无音符工程必须逐位静音");
    assert_eq!(bare.stats.meter_frames, 32 * 2, "轨道 + 母线各一帧");

    // (c) 只有音频片段的工程：采样播放未接入 ⇒ 仍然静音（不是噪声底）
    let audio = audio_clip_project();
    let played = render(&audio.project, 32);
    assert_eq!(played.nonzero(), 0, "音频片段尚未接入 ⇒ 必须静音");

    // 三者都不得 panic、统计自洽
    for rendered in [&empty, &bare, &played] {
        assert!(rendered.peak() == 0.0);
        assert_eq!(rendered.stats.voice_steals, 0);
        assert_eq!(rendered.revision, Some(1));
    }
}

/// J7：**升八度让零交叉数翻倍** —— 整数相位递推的音高正确性。
#[test]
fn octave_doubles_the_zero_crossing_rate() {
    let low_fixture = note_project(&[NoteSpec::at(0, 1920, 69, 127)]); // A4 = 440 Hz
    let high_fixture = note_project(&[NoteSpec::at(0, 1920, 81, 127)]); // A5 = 880 Hz
    let low = render(&low_fixture.project, 376); // 48128 帧 ≈ 1.0 s
    let high = render(&high_fixture.project, 376);

    let low_crossings = zero_crossings(&low.left);
    let high_crossings = zero_crossings(&high.left);
    println!("[engine-sound] J7 crossings A4={low_crossings} A5={high_crossings}");

    // 一个周期 2 次零交叉 ⇒ 期望值 = 2 × f × 窗口秒数。
    // 容差理由：窗口边界处最多差 1 次穿越（首帧相位为 0 时那一次不被计数），
    // 采样率与音高都不是整数比时还会再多 1 次 ⇒ 给 ±3 的绝对容差（约 0.34%）。
    // 判别力不受影响：把相位累加改成"每样本从 0 开始"会让输出恒为 `table[0] == 0`
    // （静音）或落到完全不同的频率上，零交叉数会掉到 0/常数级。
    let expected = 2.0 * 440.0 * (low.frames() as f64 / 48_000.0);
    assert!(
        (low_crossings as f64 - expected).abs() <= 3.0,
        "A4 的零交叉数应 ≈ {expected:.1}, 实际 {low_crossings}"
    );
    assert!(
        (high_crossings as f64 - 2.0 * low_crossings as f64).abs() <= 3.0,
        "升八度必须让零交叉数翻倍: A4={low_crossings} A5={high_crossings}"
    );
}

/// J8：发布一份**等价**快照（只改 `revision`）不得重触发音符、不得改变任何样本位。
///
/// 这是"快照切换不重触发"的端到端形态：如果没有这一条，`align_cursors` 的
/// 单调游标就可能被写成"切换时清零"而所有判据仍然全绿。
#[test]
fn rearming_an_equivalent_snapshot_does_not_retrigger() {
    let fixture = note_project(&[
        NoteSpec::at(0, 960, 60, 100),
        NoteSpec::at(960, 960, 64, 100),
        NoteSpec::at(1920, 960, 67, 100),
    ]);
    let quanta = 300; // 38400 帧 = 1536 tick，覆盖前两个音符
    let reference = render(&fixture.project, quanta);

    let project = fixture.project.clone();
    let switched = render_with(&fixture.project, quanta, 1, move |quantum, rig| {
        if quantum == 150 {
            rig.publish_equivalent(&project, 2);
        }
    });

    assert_eq!(
        switched.stats.snapshot_switches, 1,
        "快照必须**真的**被切换过一次（否则这条判据是空转）"
    );
    assert_eq!(switched.revision, Some(2));
    assert_eq!(reference.revision, Some(1));
    assert_eq!(
        switched.stats.notes_triggered, reference.stats.notes_triggered,
        "等价快照的切换不得让任何音符被重复触发"
    );
    assert_eq!(
        switched.left_bits(),
        reference.left_bits(),
        "等价快照的切换不得改变任何一个样本位"
    );
}

/// J9：模型 → 快照 → 采样位置的**覆盖度**（手算栅格逐条对表）。
#[test]
fn snapshot_projects_model_notes_onto_the_sample_grid() {
    let fixture = note_project(&[
        NoteSpec::at(0, 480, 60, 127),
        NoteSpec::at(960, 480, 64, 127),
        NoteSpec::at(1920, 480, 67, 127),
    ]);
    let snapshot =
        EngineSnapshot::from_project(&fixture.project, 1).expect("夹具工程必须能编译成快照");

    assert_eq!(snapshot.scheduled_notes(), 3);
    assert_eq!(snapshot.note_schedule_drops(), 0);
    let schedule = snapshot.schedule(&fixture.track).expect("该轨必须有调度表");
    assert_eq!(schedule.len(), 3);
    let onsets: Vec<u64> = schedule
        .notes()
        .iter()
        .map(|note| note.start_sample())
        .collect();
    assert_eq!(
        onsets,
        vec![0, 24_000, 48_000],
        "tick → 样本位置必须手算可对"
    );
    let pitches: Vec<u8> = schedule.notes().iter().map(|note| note.pitch()).collect();
    assert_eq!(pitches, vec![60, 64, 67]);
    for note in schedule.notes() {
        assert_eq!(note.end_sample() - note.start_sample(), 12_000);
        assert!(note.phase_inc() > 1, "相位增量必须由音高算出");
        assert!(note.gain() > 0.0);
    }
    // 高音的相位增量必须更大（音高单调 ⇒ 增量单调）
    assert!(schedule.notes()[0].phase_inc() < schedule.notes()[2].phase_inc());
}

/// J10：超过调度表容量上限时**计数丢弃**，不 panic、不无界增长。
#[test]
fn note_capacity_is_bounded_and_counted() {
    let overflow = 7usize;
    let specs: Vec<NoteSpec> = (0..MAX_NOTES_PER_TRACK + overflow)
        .map(|index| NoteSpec::at(index as u64, 1, 60, 100))
        .collect();
    let fixture = note_project(&specs);
    let snapshot =
        EngineSnapshot::from_project(&fixture.project, 1).expect("夹具工程必须能编译成快照");

    assert_eq!(
        snapshot.scheduled_notes(),
        MAX_NOTES_PER_TRACK,
        "调度表必须被钉在容量上限"
    );
    assert_eq!(
        snapshot.note_schedule_drops(),
        overflow as u64,
        "超出容量的音符必须被**计数**而不是静默消失"
    );
    // 渲染一个量子仍然正常（不 panic），并且第一个音符真的响
    let rendered = render(&fixture.project, 1);
    assert!(rendered.nonzero() > 0, "容量上限之内的音符必须照常发声");
}
