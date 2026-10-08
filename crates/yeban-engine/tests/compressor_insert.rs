//! `line/engine-wiring` 的端到端判据：**每轨插入压缩器**（`crate::insert` → `rt`）。
//! [ARCH-RT-001, ARCH-DET-001, ROAD-M2-006]
//!
//! 全部判据断言在**真实渲染路径**产出的样本上：
//! `YebanProjectV1` → `EngineSnapshot::from_project` → `SnapshotSlot` →
//! `EngineRuntime::process_quantum`。`support` 模块刻意不给测试专用捷径。
//!
//! | 编号 | 判据 | 怎么变红（注入） |
//! | :--- | :--- | :--- |
//! | C0 | **没有已识别压缩器参数**的工程，输出与**接线前**（`5bd411f`）逐字节相同（落盘原始样本，跨提交比 `sha256`） | 让插入链对"无参数的效果器"也生效 / 悄悄改了别的接线 |
//! | C1 | `InternalEffect` 设备**参数为空**（既有夹具的形状）⇒ 输出与"根本没有设备链"逐位相同 | 按"设备存在"而不是"参数命中"激活 |
//! | C2 | `bypassed = true` 的压缩器设备 ⇒ 不武装、输出逐位不变 | 旁通的设备被接回来 |
//! | C3 | 命中的设备**真的武装进实时侧**：`armed_compressor(轨)` 逐位等于投影结果，且 `EngineStats` 报出压过的样本数与最大衰减（dB） | 快照里有、实时侧没读（`armed_*` 为 `None` / 计数恒 0） |
//! | C4 | 武装后**真的改变输出**：母线峰值下降、且与未武装的指纹不同 | 压缩器被接到不参与混音的地方 |
//! | C5 | 同输入两次独立装配 + 渲染**逐位相同**（含插入压缩器） | 引入真熵源 / 弹道状态跨装配泄漏 |
//! | C6 | 引擎的 `Compressor` **就是** `yeban_dsp::compressor::Compressor`（类型与函数地址同一性，不是同构复制品） | 在引擎侧留第二份压缩器实现 |
//! | C7 | 设备链顺序：第一个**含已识别参数名**的效果器是唯一来源；更早的无关效果器不参与 | 取"最后一个"或"任意一个"设备 |
//!
//! ## C0 的证据落盘（跨提交比对）
//!
//! C0 把交错样本按**小端 `f32`** 写成 `$TMPDIR/yeban-compressor-unarmed.raw`
//! （武装的那份写 `…-armed.raw`）⇒ 提交前后各跑一次、`shasum -a 256` 两张原始样本
//! 即可判"未接线时逐字节不变"。判据刻意**不**把哈希硬编码进源码：
//! 那会把"本机 libm 的某一位"当成规范常数（跨平台金标准不是本票的事）。
//!
//! ## 判据的**实测数字**与口径表
//!
//! 见 `docs/ledger/engine-mix-notes.md` 的同族记录（本线**未改** `docs/**`：它由集成者独占）。

mod support;

use std::path::Path;

use support::{MixSpec, NoteSpec, note_project, render, render_with, tuned_project};
use yeban_model::{DeviceDefinition, DeviceKind, EntityId, ParameterValue, YebanProjectV1};

/// 判据用的栅格：120 BPM / 48 kHz / 960 PPQ ⇒ 1 tick = 25 样本（见 `support` 模块文档）。
const NOTES: [NoteSpec; 4] = [
    NoteSpec::at(0, 480, 60, 127),
    NoteSpec::at(480, 480, 64, 110),
    NoteSpec::at(960, 960, 67, 100),
    NoteSpec::at(1920, 480, 72, 90),
];

/// 渲染量子数：400 × 128 = 51,200 帧 ≈ 1.067 s @48 kHz ⇒ 盖住最后一个音符的尾音。
const QUANTA: usize = 400;

/// 判据用的**母线峰值**上限口径：见 `yeban_engine::mixer::LIMITER_THRESHOLD`。
///
/// 这一组夹具（0 dB 音量、无压缩器、力度 127）实测峰值低于阈值 ⇒ 母线限制器
/// **一次都没压**（`limiter_gain_reductions == 0`）。C4 因此可以把
/// "峰值下降"**单独**归因给插入压缩器，而不是被限制器掩盖。
fn compressor_device(params: &[(&str, f32)]) -> DeviceDefinition {
    DeviceDefinition {
        id: EntityId::new(),
        name: "Comp".to_owned(),
        kind: DeviceKind::InternalEffect,
        bypassed: false,
        params: params
            .iter()
            .map(|(name, value)| ParameterValue {
                name: (*name).to_owned(),
                value: *value,
                unit: None,
            })
            .collect(),
        latency_samples: 0,
    }
}

/// 把设备链接到夹具的那条 MIDI 轨上（**只改夹具**，不改模型层）。
fn mount(project: &mut YebanProjectV1, track: EntityId, devices: Vec<DeviceDefinition>) {
    let entry = project
        .tracks
        .get_mut(&track)
        .expect("夹具里必须有那条 MIDI 轨");
    entry.devices = devices;
}

/// 把交错样本按小端 `f32` 落盘，返回 `(路径, 字节数)`。
///
/// 与 `tests/metronome_render.rs` 的 M0 同一个形状：判据不硬编码哈希。
fn dump(name: &str, left: &[f32], right: &[f32]) -> (std::path::PathBuf, usize) {
    let mut bytes = Vec::with_capacity(left.len() * 8);
    for index in 0..left.len() {
        bytes.extend_from_slice(&left[index].to_le_bytes());
        bytes.extend_from_slice(&right[index].to_le_bytes());
    }
    let path = Path::new(&std::env::temp_dir()).join(name);
    std::fs::write(&path, &bytes).expect("原始样本必须能落盘");
    (path, bytes.len())
}

/// C0 + C1 + C2：**未武装**的三条口径必须逐位相同。
///
/// 三份输入的唯一差别是设备链：
///
/// 1. `bare`：**没有**设备链（接线前的既有夹具形状）；
/// 2. `unknown`：一台 `InternalEffect`，`params` 为空（只为上报延迟而存在的那一类）；
/// 3. `unrelated`：一台 `InternalEffect`，参数名一个都不认识；
/// 4. `bypassed`：参数**命中**但 `bypassed = true`。
///
/// 四份都必须给出同一份样本（`Render::fingerprint` + 逐位 `left_bits`）。
/// 任何一份不同 ⇒ 插入链对"没有已识别压缩器参数的工程"也生效了。
#[test]
fn projects_without_recognised_compressor_params_are_bit_identical() {
    let fixture = note_project(&NOTES);
    let track = fixture.track;

    let bare = render(&fixture.project, QUANTA);

    let mut unknown_project = fixture.project.clone();
    mount(&mut unknown_project, track, vec![compressor_device(&[])]);
    let unknown = render(&unknown_project, QUANTA);

    let mut unrelated_project = fixture.project.clone();
    mount(
        &mut unrelated_project,
        track,
        vec![compressor_device(&[
            ("reverb_mix", 0.3),
            ("filter_cutoff_hz", 800.0),
        ])],
    );
    let unrelated = render(&unrelated_project, QUANTA);

    let mut bypassed_project = fixture.project.clone();
    let mut bypassed_device = compressor_device(&[("threshold_db", -30.0), ("ratio", 8.0)]);
    bypassed_device.bypassed = true;
    mount(&mut bypassed_project, track, vec![bypassed_device]);
    let bypassed = render(&bypassed_project, QUANTA);

    let (path, bytes) = dump("yeban-compressor-unarmed.raw", &bare.left, &bare.right);
    println!(
        "[engine-wiring/C0] 未武装: 落盘={} 字节={bytes} 帧={} 指纹={:#018x}",
        path.display(),
        bare.frames(),
        bare.fingerprint()
    );

    assert!(bare.frames() > 0, "夹具必须真的渲染出帧");
    assert!(bare.peak() > 0.0, "夹具必须真的出声（否则逐位相同是空转）");
    assert_eq!(
        bare.stats.limiter_gain_reductions, 0,
        "本夹具必须整体低于母线限制器阈值 —— 否则 C4 的峰值差归因不唯一"
    );
    for (label, other) in [
        ("参数为空的效果器", &unknown),
        ("参数不认识的的效果器", &unrelated),
        ("bypassed 的压缩器", &bypassed),
    ] {
        assert_eq!(
            other.fingerprint(),
            bare.fingerprint(),
            "{label}: 输出必须与没有设备链时逐位相同"
        );
        assert_eq!(
            other.left_bits(),
            bare.left_bits(),
            "{label}: 左声道必须逐位相同"
        );
    }

    // C1 的独立一条：**没有任何设备链**的工程快照里，插入表必须是空的。
    let snapshot =
        yeban_engine::snapshot::EngineSnapshot::from_project(&fixture.project, 1).expect("快照");
    assert!(
        snapshot.inserts().is_empty(),
        "没有效果器设备的工程不得产生任何插入链"
    );
    assert!(snapshot.insert_params(&track).is_none());
}

/// 判据用的**武装**参数：深阈值 + 高比率 ⇒ 夹具的每一个音符都被压到。
///
/// 取值刻意不在 `CompressorParams::DEFAULT` 上：C3 要断言"投影出来的**这一份**数"
/// 真的走到了实时侧，用默认值会让"没接线"与"接线了"读数相同（假绿）。
const ARMED_PARAMS: [(&str, f32); 4] = [
    ("threshold_db", -30.0),
    ("ratio", 8.0),
    ("attack", 0.001),
    ("release", 0.05),
];

/// 一条挂**已识别压缩器**的 MIDI 轨（音量 0 dB，声相居中）。
fn armed_project() -> (YebanProjectV1, EntityId) {
    let mut fixture = tuned_project(&NOTES, MixSpec::volume(0.0));
    let track = fixture.track;
    mount(
        &mut fixture.project,
        track,
        vec![compressor_device(&ARMED_PARAMS)],
    );
    (fixture.project, track)
}

/// C3：命中的设备**真的武装进了实时侧**。
///
/// 三层证据，缺一不可：
///
/// 1. **快照层**：`EngineSnapshot::insert_params(轨)` 给出投影参数（模型 → 快照）；
/// 2. **实时层**：`EngineRuntime::armed_compressor(轨)` 逐位等于第 1 层
///    （快照 → 逐样本路径的武装表）。这是"快照里有、实时侧没读"这类缺陷的**唯一**
///    机械判据 —— 那种缺陷不会 panic，也不会让"峰值 ≤ 天花板"之类的静态断言变红；
/// 3. **行为层**：`EngineStats` 报出压过的样本数与最大衰减（dB）> 0。
#[test]
fn recognised_compressor_is_armed_into_the_realtime_path() {
    let (project, track) = armed_project();

    let snapshot =
        yeban_engine::snapshot::EngineSnapshot::from_project(&project, 1).expect("夹具快照");
    let projected = snapshot
        .insert_params(&track)
        .and_then(yeban_engine::insert::InsertParams::compressor)
        .expect("命中的效果器设备必须投影出压缩器参数");

    let mut captured: Option<yeban_engine::insert::CompressorParams> = None;
    let mut slots = usize::MAX;
    let rendered = render_with(&project, QUANTA, 1, |quantum, rig| {
        if quantum == 1 {
            captured = rig.runtime.armed_compressor(&track);
            slots = rig.runtime.armed_insert_slot_count();
        }
    });

    let armed = captured.expect("命中的压缩器必须被武装进实时侧");
    for (name, actual, expected) in [
        ("threshold_db", armed.threshold_db, projected.threshold_db),
        ("ratio", armed.ratio, projected.ratio),
        ("knee_db", armed.knee_db, projected.knee_db),
        ("detector_s", armed.detector_s, projected.detector_s),
        ("attack_s", armed.attack_s, projected.attack_s),
        ("release_s", armed.release_s, projected.release_s),
        ("makeup_db", armed.makeup_db, projected.makeup_db),
    ] {
        assert_eq!(
            actual.to_bits(),
            expected.to_bits(),
            "武装表的 {name} 必须逐位等于快照投影: 武装={actual} 投影={expected}"
        );
    }
    assert_eq!(slots, 1, "夹具只有一台压缩器 ⇒ 只应占一个槽位");
    // 投影的值确实不是默认值（否则"接线了"与"没接线"读数相同）。
    assert_ne!(
        armed.threshold_db.to_bits(),
        yeban_engine::insert::CompressorParams::DEFAULT
            .threshold_db
            .to_bits()
    );

    let stats = rendered.stats;
    println!(
        "[engine-wiring/C3] 武装: 槽位={slots} 压过样本={} 最大衰减={:.3} dB 峰值={:.6}",
        stats.insert_gain_reductions,
        stats.insert_max_reduction_db,
        rendered.peak()
    );
    assert!(
        stats.insert_gain_reductions > 0,
        "夹具的音符必须真的被压缩器压到（压过样本数 = 0 ⇒ 这条判据没覆盖插入链）"
    );
    assert!(
        stats.insert_max_reduction_db > 0.0,
        "最大增益衰减必须 > 0 dB（0 ⇒ 只有检波器在跑、弹道没压）"
    );
}

/// C4：武装之后输出**真的变了**，且变化方向正确（压缩只能衰减）。
///
/// "没有变化"是插入链最容易出现的假绿：快照里有参数、`armed_*` 也有读数，
/// 但器件被接到了一条不参与混音的分支上。
#[test]
fn an_armed_compressor_changes_the_output_and_only_attenuates() {
    let (project, track) = armed_project();
    let mut unarmed_project = project.clone();
    mount(&mut unarmed_project, track, Vec::new());

    let armed = render(&project, QUANTA);
    let unarmed = render(&unarmed_project, QUANTA);

    println!(
        "[engine-wiring/C4] 峰值: 武装={:.6} 未武装={:.6} 指纹: 武装={:#018x} 未武装={:#018x}",
        armed.peak(),
        unarmed.peak(),
        armed.fingerprint(),
        unarmed.fingerprint(),
    );

    assert!(unarmed.peak() > 0.0, "未武装的夹具必须真的出声");
    assert_eq!(
        unarmed.stats.limiter_gain_reductions, 0,
        "未武装的夹具必须整体低于母线限制器阈值 ⇒ 峰值差归因唯一"
    );
    assert!(
        armed.peak() < unarmed.peak(),
        "压缩器只能衰减: 武装峰值 {} 必须小于未武装峰值 {}",
        armed.peak(),
        unarmed.peak()
    );
    assert_ne!(
        armed.fingerprint(),
        unarmed.fingerprint(),
        "武装压缩器后输出必须改变（指纹相同 ⇒ 器件被接在了不参与混音的地方）"
    );
}

/// C5：**确定性** —— 同输入两次独立装配 + 渲染必须逐位相同（含插入压缩器的弹道状态）。
///
/// 弹道是跨量子的时间状态，因此"两次装配一致"同时排除了"状态跨装配泄漏"
/// （例如 `EngineRuntime` 复用了别的实例的器件槽）。
///
/// ⚠ 两份夹具的 `EntityId` **不同**（`EntityId::new()` 每次给一个新 ULID）：
/// 判据刻意**不**断言 id 相同，而断言**输出**逐位相同 —— 与
/// `tests/mix_render.rs` 的 M6 同一条口径：身份不同但结构相同的装配必须给出同一份样本。
#[test]
fn two_independent_runs_with_an_armed_compressor_are_bit_identical() {
    let (first_project, _) = armed_project();
    let (second_project, _) = armed_project();

    let first = render(&first_project, QUANTA);
    let second = render(&second_project, QUANTA);

    assert_eq!(first.left_bits(), second.left_bits(), "左声道必须逐位相同");
    assert_eq!(first.right, second.right, "右声道必须逐位相同");
    assert_eq!(first.fingerprint(), second.fingerprint());
    assert_eq!(
        first.stats.insert_gain_reductions,
        second.stats.insert_gain_reductions
    );
    assert_eq!(
        first.stats.insert_max_reduction_db.to_bits(),
        second.stats.insert_max_reduction_db.to_bits()
    );
}

/// C6：引擎的 `Compressor` **就是** `yeban_dsp::compressor::Compressor`。
///
/// 这是编译期判据（类型赋值）+ 运行期判据（函数地址相等 + 零延迟契约）。
/// 注入：在引擎侧加一份自己的压缩器类型或 `process_mono` ⇒ 两者之一立即变红。
/// 与 `mixer.rs` 的 `engine_bus_limiter_is_literally_the_dsp_limiter` 同款。
#[test]
fn engine_compressor_is_literally_the_dsp_compressor() {
    // 类型同一性（编译期）：dsp 类型的绑定可以直接由 engine 路径构造。
    let via_engine: yeban_dsp::compressor::Compressor = yeban_engine::insert::Compressor::new(
        yeban_engine::insert::CompressorParams::DEFAULT,
        48_000.0,
    );
    // 零延迟契约：压缩器**不进** PDC 表（接线因此不改任何补偿计划）。
    assert_eq!(
        via_engine.latency_samples(),
        0,
        "压缩器必须零延迟，否则接线会改变 PDC 计划"
    );

    // 方法同一性（地址相等）：这些**必须**是同一个函数项，而不是两份同构实现。
    let engine_process: fn(&mut yeban_engine::insert::Compressor, &mut [f32]) -> usize =
        yeban_engine::insert::Compressor::process_mono;
    let dsp_process: fn(&mut yeban_dsp::compressor::Compressor, &mut [f32]) -> usize =
        yeban_dsp::compressor::Compressor::process_mono;
    assert!(core::ptr::fn_addr_eq(engine_process, dsp_process));

    let engine_new: fn(
        yeban_engine::insert::CompressorParams,
        f32,
    ) -> yeban_engine::insert::Compressor = yeban_engine::insert::Compressor::new;
    let dsp_new: fn(
        yeban_dsp::compressor::CompressorParams,
        f32,
    ) -> yeban_dsp::compressor::Compressor = yeban_dsp::compressor::Compressor::new;
    assert!(core::ptr::fn_addr_eq(engine_new, dsp_new));

    let engine_set: fn(
        &mut yeban_engine::insert::Compressor,
        yeban_engine::insert::CompressorParams,
    ) = yeban_engine::insert::Compressor::set_params;
    let dsp_set: fn(
        &mut yeban_dsp::compressor::Compressor,
        yeban_dsp::compressor::CompressorParams,
    ) = yeban_dsp::compressor::Compressor::set_params;
    assert!(core::ptr::fn_addr_eq(engine_set, dsp_set));

    // 参数类型同一性（逐位）：engine 路径的 `DEFAULT` 就是 dsp 的 `DEFAULT`。
    assert_eq!(
        yeban_engine::insert::CompressorParams::DEFAULT
            .threshold_db
            .to_bits(),
        yeban_dsp::compressor::CompressorParams::DEFAULT
            .threshold_db
            .to_bits()
    );
}

/// C7：设备链顺序 —— 第一个**含已识别参数名**的效果器是唯一来源。
///
/// 更早的无关效果器不参与；更晚的效果器不覆盖。
#[test]
fn the_first_recognised_effect_in_the_chain_is_the_only_source() {
    let fixture = note_project(&NOTES);
    let track = fixture.track;
    let mut project = fixture.project.clone();
    mount(
        &mut project,
        track,
        vec![
            // 第一条：一个已识别名字都没有 ⇒ 不是来源。
            compressor_device(&[("mix", 0.5)]),
            // 第二条：命中 ⇒ 唯一来源。
            compressor_device(&[("ratio", 3.0), ("threshold", -20.0)]),
            // 第三条：不得覆盖第二条。
            compressor_device(&[("ratio", 99.0), ("threshold_db", -60.0)]),
        ],
    );

    let snapshot = yeban_engine::snapshot::EngineSnapshot::from_project(&project, 1).expect("快照");
    let params = snapshot
        .insert_params(&track)
        .and_then(yeban_engine::insert::InsertParams::compressor)
        .expect("第二条设备必须命中");
    assert_eq!(params.ratio.to_bits(), 3.0f32.to_bits(), "第二条是唯一来源");
    assert_eq!(
        params.threshold_db.to_bits(),
        (-20.0f32).to_bits(),
        "别名 `threshold` 必须与 `threshold_db` 同解"
    );

    // 行为层：同一条链只武装一台器件（`armed_insert_slot_count` 是权威读数）。
    let mut slots = usize::MAX;
    let _ = render_with(&project, 2, 1, |quantum, rig| {
        if quantum == 1 {
            slots = rig.runtime.armed_insert_slot_count();
        }
    });
    assert_eq!(slots, 1, "一条链只允许武装一台压缩器");
}
