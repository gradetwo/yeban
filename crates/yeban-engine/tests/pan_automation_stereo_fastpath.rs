//! ⭐ **T-FIX-14（审计 #14）**：**声相自动化已武装 ＋ 分声道轨 ＋ 直连 master 快路**
//! 时，右声道不得被丢掉。
//!
//! # 缺陷（修前）
//!
//! `render_branch` 的声相段按四格分流（静态/自动化 × 快路/慢路）。其中
//! 「自动化 ＋ 快路」（`single_unit_to_master`）一格调用的是**单声道**求和
//! `sum_into_bus_smoothed`，只读左缓冲 `track_scratch`：分声道轨的右缓冲
//! `track_scratch_r` 从未被读 ⇒ 右声道内容被左声道顶替（等价于丢右声道）。
//! 同格的慢路（`render_panned_pair_into_smoothed`）与静态格
//! （`sum_pair_into_bus(_smoothed)`）都是成对求和 —— 只有这一格漏了成对版。
//!
//! # 夹具
//!
//! * SFZ 区域 `pan=100`（硬右）＋ 直流样本 ⇒ 源侧左缓冲只剩浮点残留
//!   （`cos(π/2) ≈ 6.1e-8`）、右缓冲满幅 —— 与 `sfz_pan_insert_joint` 同款；
//! * 工程无路由边 ⇒ 该轨走「直连 master、单位增益」快路；
//! * 经事件环发布声相目标 `(1.0, 1.0)` 武装自动化：平滑器在快照边界已
//!   `snap_to` 到静态增益，居中轨静态增益恰为 `(1, 1)`（`centered_pan_gains`）
//!   ⇒ 目标与初值相同 ⇒ 逐样本增益恒为精确的 `1.0`。
//!
//! # 三条腿
//!
//! 1. **自动化（快路）**：右声道必须满幅出声、左声道只剩残留，且与第 2 腿
//!    **逐位相同**（指纹相等）—— 快路与静态成对求和是同一把尺子；
//! 2. **静态（参考）**：不发事件，走静态成对求和；
//! 3. **镜像对照**：区域改 `pan=-100`（硬左）＋ 同样武装 ⇒ 左满幅、右残留
//!    （证明第 1 腿的判别力来自声道，而不是「哪条腿本来就哑」）。
//!
//! 修前第 1 腿必红：右声道只剩左缓冲的残留（峰值 ≈ 1e-8 量级），且指纹与
//! 第 2 腿不同。修后三腿全绿。

mod support;

use std::sync::Arc;

use support::{NoteSpec, note_project};
use yeban_engine::meter::meter_channel;
use yeban_engine::param::{TRACK_PAN_LEFT_SLOT, TRACK_PAN_RIGHT_SLOT};
use yeban_engine::ring::{EngineEvent, ParamAddress, event_channel};
use yeban_engine::rt::EngineRuntime;
use yeban_engine::sfz_instrument::{InMemorySample, SfzInstrument, SfzPlan};
use yeban_engine::snapshot::{EngineSnapshot, SnapshotSlot, retire_channel};
use yeban_sfz::parser::ParseLimits;

/// 一个音高、480 tick ＝ 12 000 帧（120 BPM / 960 PPQ）。
const NOTES: [NoteSpec; 1] = [NoteSpec::at(0, 480, 60, 127)];

/// 400 × 128 = 51 200 帧（覆盖整个音符与释放）。
const QUANTA: usize = 400;

/// 直流样本（0.4）：逐帧同一电平 ⇒ 声道断言不受插值影响。
fn dc_sample(amplitude: f32, frames: usize) -> InMemorySample {
    InMemorySample {
        channels: 1,
        pcm: Arc::new(vec![amplitude; frames]),
        sample_hz: 48_000.0,
        loop_points: None,
    }
}

/// 区域声相由参数决定（`100` ＝ 硬右、`-100` ＝ 硬左）；其余逐字相同。
fn plan_with_pan(pan: i32, sample: &InMemorySample) -> SfzPlan {
    let text = format!(
        "<region> sample=a.wav key=60 pitch_keycenter=60 pan={pan} \
         loop_mode=loop_continuous loop_start=0 loop_end=1000 \
         ampeg_attack=0.001 ampeg_sustain=100 ampeg_release=0.001 amp_veltrack=0"
    );
    let instrument = SfzInstrument::load(text, &ParseLimits::default()).expect("有效 SFZ");
    SfzPlan::prepare(&instrument, sample, 48_000.0).expect("SFZ 摊平")
}

/// 渲染一条腿：`armed` 为真时在首个量子之前发布声相目标 `(1.0, 1.0)`。
/// 返回渲染结果与运行时的声相自动化帧计数（该读数在 `EngineRuntime` 上，
/// 不在 `EngineStats` 里）。
///
/// 驱动方式与 `support::render_snapshot_with` 逐字同款（产品路径：快照槽 →
/// `EngineRuntime::process_quantum`），只是把事件发送端留下来发声相目标。
fn render_leg(pan: i32, armed: bool) -> (support::Render, u64) {
    let fixture = note_project(&NOTES);
    let track = fixture.track;
    let sample = dc_sample(0.4, 48_000);
    let plan = plan_with_pan(pan, &sample);
    let snapshot = EngineSnapshot::from_project(&fixture.project, 1)
        .expect("工程快照")
        .with_sfz(track, plan);

    let slot = SnapshotSlot::new(snapshot);
    let (retire, _queue) = retire_channel(64);
    let (mut sender, receiver) = event_channel(64);
    let (publisher, _collector) = meter_channel(4096);
    let mut runtime = EngineRuntime::new(&slot, retire, receiver, publisher);

    // 预热一个量子（输出丢弃）：快照边界先把声相表武装好，之后发布的声相
    // 目标才会被事件循环截走；顺序反了事件会落进未武装的表被判 Unmapped。
    // 三条腿同款预热 ⇒ 捕获的音频起点一致（音符第 128 帧起）。
    let mut warm = vec![0.0f32; 128 * 2];
    runtime.process_quantum(&mut warm, 2);

    if armed {
        let events = [
            EngineEvent::SetParam {
                target: ParamAddress::new(track, TRACK_PAN_LEFT_SLOT),
                value: 1.0,
            },
            EngineEvent::SetParam {
                target: ParamAddress::new(track, TRACK_PAN_RIGHT_SLOT),
                value: 1.0,
            },
        ];
        assert_eq!(sender.publish(&events), 2, "声相目标事件必须全部入环");
    }

    let mut left = Vec::with_capacity(QUANTA * 128);
    let mut right = Vec::with_capacity(QUANTA * 128);
    let mut out = vec![0.0f32; 128 * 2];
    for _ in 0..QUANTA {
        out.fill(0.0);
        runtime.process_quantum(&mut out, 2);
        for frame in 0..128 {
            left.push(out[frame * 2]);
            right.push(out[frame * 2 + 1]);
        }
    }
    let pan_automation_frames = runtime.pan_automation_frames();
    (
        support::Render {
            left,
            right,
            stats: runtime.stats(),
            revision: runtime.revision(),
        },
        pan_automation_frames,
    )
}

fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0f32, |m, s| m.max(s.abs()))
}

#[test]
fn armed_pan_automation_fast_path_keeps_the_right_channel() {
    let (armed, armed_pan_frames) = render_leg(100, true);
    let (reference, reference_pan_frames) = render_leg(100, false);
    let (mirror, _mirror_pan_frames) = render_leg(-100, true);

    let total_frames = (QUANTA * 128) as u64;
    // 前提钉：三条腿都真的走了分声道渲染（含预热量子 ⇒ +128 帧）；
    // 只有自动化腿计声相自动化帧（预热在发布之前 ⇒ 恰好 QUANTA 个量子）。
    assert_eq!(
        armed.stats.stereo_track_frames,
        total_frames + 128,
        "自动化腿必须走分声道渲染（每量子恰好 ＋frames）"
    );
    assert_eq!(
        reference.stats.stereo_track_frames,
        total_frames + 128,
        "静态腿必须走分声道渲染"
    );
    assert_eq!(
        armed_pan_frames, total_frames,
        "声相自动化必须真的被武装并逐帧推进"
    );
    assert_eq!(
        reference_pan_frames, 0,
        "静态腿不得计声相自动化帧（反向钉）"
    );

    // 硬右源：左只剩浮点残留（口径与 `sfz_pan_insert_joint` 相同）。
    const RESIDUE: f32 = 1e-6;
    let armed_right_peak = peak(&armed.right);
    let armed_left_peak = peak(&armed.left);
    println!(
        "[fix14] 自动化快路：右峰={armed_right_peak:.9}、左峰={armed_left_peak:.9}；\
         静态参考：右峰={:.9}；指纹 自动化={:#018x} / 静态={:#018x}",
        peak(&reference.right),
        armed.fingerprint(),
        reference.fingerprint(),
    );

    // ⭐ 本判据的牙：右声道必须真的出声（修前只剩左缓冲残留 ⇒ 必红）。
    assert!(
        armed_right_peak > 0.01,
        "声相自动化快路下右声道必须出声（修前右声道被左缓冲顶替、只剩残留）：\
         右峰 = {armed_right_peak}"
    );
    assert!(
        armed_left_peak <= RESIDUE,
        "硬右源的左声道必须只剩浮点残留：左峰 = {armed_left_peak}"
    );
    // ⭐ 快路与静态成对求和逐位一致（目标 (1,1) ＝ 静态居中增益 ⇒ 同一把尺子）。
    assert_eq!(
        armed.fingerprint(),
        reference.fingerprint(),
        "声相自动化快路输出必须与静态成对求和逐位相同"
    );

    // 镜像对照：硬左 ＋ 武装 ⇒ 左满幅、右残留（判别力来自声道，不是哑腿）。
    assert!(
        peak(&mirror.left) > 0.01,
        "镜像腿左声道必须出声：左峰 = {}",
        peak(&mirror.left)
    );
    assert!(
        peak(&mirror.right) <= RESIDUE,
        "镜像腿右声道必须只剩浮点残留：右峰 = {}",
        peak(&mirror.right)
    );
}
