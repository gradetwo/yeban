//! `line/engine-mix` 的端到端判据（一）：**声相定律与母线限制器**。
//! [ARCH-DSP-001, MODEL-AST-002, ARCH-RT-001, ARCH-DET-001]
//!
//! 全部判据在 **`--no-default-features`（不编译 cpal）** 下运行 —— 与
//! `synth_render.rs` 一样，这是本机与 CI 的主路径，不需要声卡。
//!
//! | 编号 | 判据 | 怎么变红（注入） |
//! | :--- | :--- | :--- |
//! | M1 | 居中等功率：左/右有效值相等，且**恰好**是"全左"的 √2/2 | 把声相改回等增益复制 |
//! | M2 | 左/右声相增益落在 `cos θ`/`sin θ` 上（含全左时右声道**逐位**静音） | 声相取反 / 忽略 `pan` |
//! | M3 | 声相是全链路的**标量**变换（中心 = 硬左 × √2/2，逐样本同容差） | 逐样本加噪/加抖动 |
//! | M4 | 过阈值夹具经母线限制器后峰值 **≤ 天花板**，且限制器**确实压过**样本 | 把阈值乘 10 / 摘掉限制器 |
//! | M5 | 未过阈值的工程**逐位**不被篡改（`reductions == 0` + 两次渲染逐位一致） | 在母线里加抖动/旁路重采样 |
//! | M6 | 同输入两次渲染**逐位相同**（含混音链全部器件） | 引入真熵源 |
//! | M7 | 静音工程（含音色/声相配置）仍然**逐位**静音 | 限制器直流泄漏 / 滤波器自激 |
//! | M8 | **主总线推子**改变输出：0 dB 与 −6 dB 的样本满足 `out₋₆ = f32(out₀ × g)`（逐位） | 主总线增益不参与混音（快照字段接线断在实时侧） |
//! | M9 | 主总线推子在**母线限制器之前**：−6 dB 把过阈值夹具拉回透明区 | 把推子移到限制器之后 |
//! | M10 | 母线限制器的**当前压限**是一条**量规**：逐量子采样取最大值 == 全程最大量（**逐位**）；音符结束后回落；未过阈值时**逐位**为 `0.0` | 把该读数写成常数 / 从不更新 |
//!
//! 判据的**实测数字**与口径表见 `docs/ledger/engine-mix-notes.md`。

mod support;

use support::{MixSpec, NoteSpec, empty_project, render, render_with, rms_peak, tuned_project};

/// 声相定律的声明值：等功率 −3 dB 居中 ⇒ `cos(π/4) = sin(π/4) = √2/2`。
const CENTRE_GAIN: f64 = core::f64::consts::FRAC_1_SQRT_2;

/// M1 + M3：**居中等功率**，且中心是"全左"的 `√2/2` 标量缩放。
///
/// ⚠ 夹具必须整体**低于限制器阈值**（这里用 −6 dB）：一个力度 127 的音符在全左时
/// 峰值实测 **0.90005** —— 恰好越过阈值，于是"全左"那一版被限制器压了、
/// "居中"那一版没有，两者的比值就不再是 `√2/2`（第一版实测 0.7126 > 0.7071）。
/// 这条不是噪声，是**限制器的非线性**：判据若不断言"整段在透明区"，它测的是
/// 两个不同增益下的比值，而不是声相定律。
#[test]
fn centre_pan_is_equal_power_and_exactly_half_sqrt_two_of_hard_left() {
    // 同一个音符（力度 127、−6 dB）在三个声相位置各渲染一次。
    let notes = [NoteSpec::at(0, 960, 69, 127)];
    let quiet = |pan: f32| MixSpec {
        volume_db: -6.0,
        ..MixSpec::pan(pan)
    };
    let centre = render(&tuned_project(&notes, quiet(0.0)).project, 120);
    let left = render(&tuned_project(&notes, quiet(-1.0)).project, 120);
    let right = render(&tuned_project(&notes, quiet(1.0)).project, 120);

    let (centre_l, _) = rms_peak(&centre.left);
    let (centre_r, _) = rms_peak(&centre.right);
    let (hard_l, _) = rms_peak(&left.left);
    let (hard_r, _) = rms_peak(&right.right);

    println!(
        "[engine-mix] M1 rms centre_l={centre_l:.6} centre_r={centre_r:.6} \
         hard_l={hard_l:.6} hard_r={hard_r:.6} ratio={:.6}",
        centre_l / hard_l
    );

    assert!(hard_l > 0.01, "夹具必须真的出声");
    assert!(
        centre.peak() < yeban_engine::mixer::LIMITER_THRESHOLD
            && left.peak() < yeban_engine::mixer::LIMITER_THRESHOLD,
        "夹具必须整体低于阈值（否则比值测的是限制器的非线性，而不是声相定律）: \
         centre={} hard_left={}",
        centre.peak(),
        left.peak()
    );
    assert_eq!(centre.stats.limiter_gain_reductions, 0);
    assert_eq!(left.stats.limiter_gain_reductions, 0);
    // ⭐ **R93/R91 显式下界（可诊断性守卫，⛔ 不是覆盖度守卫）**：
    // `(centre_l - centre_r).abs() <= 1e-12` 在**两侧都是 0** 时**恒真**（静音也是"完全对称"）。
    //
    // **R91 受控实验（两臂都实测过，夹具侧注入：只把 `centre` 那次渲染压到 −120 dB）**：
    // | 形态 | 结果 |
    // | :--- | :--- |
    // | 有本行下界 | **RED 在 73 行**：「夹具前提：居中渲染的左右峰值必须显著非零… centre_l=0 centre_r=0」 |
    // | 旧形态（无本行） | **RED 在 76 行**：「居中的有效值必须是全左的 √2/2… 实际比值 0」 |
    // ⇒ ⭐ **两臂都红 ⇒ 不存在"旧形态按构造必绿"的臂** ⇒ 本行**不是**覆盖度守卫：
    // 它把"真空"从**下游间接失败**（比值 0，读起来像声相定律错了）变成**在原地直接指名**
    // （"居中那次渲染根本没出声"）。⛔ 因此我**不声称**它被"喂出牙"，只声称它**改善了可诊断性**。
    // （对照：第 59 行既有的 `hard_l > 0.01` 守的是**全左**那一版的可听性，与本行**不同侧**。）
    assert!(
        centre_l > 1e-3 && centre_r > 1e-3,
        "夹具前提：居中渲染的左右峰值必须显著非零（否则对称性断言是真空的）: \
         centre_l={centre_l} centre_r={centre_r}"
    );
    assert!(
        (centre_l - centre_r).abs() <= 1e-12,
        "居中必须左右**完全**对称: {centre_l} vs {centre_r}"
    );
    // 声相是全链路的标量：居中 = 全左 × √2/2（容差只吸收 f32 乘法与 f64 求和）。
    let ratio = centre_l / hard_l;
    assert!(
        (ratio - CENTRE_GAIN).abs() < 1e-6,
        "居中的有效值必须是全左的 √2/2 = {CENTRE_GAIN}, 实际比值 {ratio}"
    );
    let ratio_right = hard_r / hard_l;
    assert!(
        (ratio_right - 1.0).abs() < 1e-6,
        "全左与全右必须对称（比值 1.0）, 实际 {ratio_right}"
    );
}

/// M2：左/右声相增益落在声明的 `cos θ` / `sin θ` 曲线上；全左时右声道**逐位**静音。
#[test]
fn pan_gains_follow_the_declared_cosine_sine_curve() {
    let notes = [NoteSpec::at(0, 960, 69, 127)];
    let quiet = |pan: f32| MixSpec {
        volume_db: -6.0,
        ..MixSpec::pan(pan)
    };
    let hard_left = render(&tuned_project(&notes, quiet(-1.0)).project, 60);

    // 1) 全左 ⇒ 右声道**逐位**为 0（`x * 0.0f32` 对有限 x 恒为 +0.0）。
    assert!(
        hard_left.right.iter().all(|sample| *sample == 0.0),
        "全左时右声道必须逐位静音"
    );
    assert!(
        hard_left.left.iter().any(|sample| *sample != 0.0),
        "全左时左声道必须出声"
    );

    // 2) 全右 ⇒ 左声道只剩 `cos(π/2) ≈ -4.37e-8` 的**浮点残差**（不是逐位 0）：
    //    等功率曲线的两个端点是 `cos(0)=1 / sin(0)=0` 与 `cos(π/2)` / `sin(π/2)=1`，
    //    而 `cos(π/2)` 在 f32 里是 −4.371139e-8 而不是 0。判据因此断言**残差量级**
    //    （≤ 1e-7 × 全幅）而不是逐位相等 —— 那是数学事实，不是实现缺陷。
    let hard_right = render(&tuned_project(&notes, quiet(1.0)).project, 60);
    let (hard_l, _) = rms_peak(&hard_left.left);
    let (residual, _) = rms_peak(&hard_right.left);
    let nz = hard_right.left.iter().filter(|s| **s != 0.0).count();
    println!("[engine-mix] M2 hard-right left residual rms={residual:e} (nonzero={nz})");
    assert!(
        residual / hard_l < 1e-7,
        "全右时左声道必须只剩浮点残差（实测比值 {}）",
        residual / hard_l
    );
    // 对称性：抓"左右写反"的注入 —— 全右的**右**声道必须与全左的**左**声道同量级。
    let (hard_r, _) = rms_peak(&hard_right.right);
    assert!(
        (hard_r / hard_l - 1.0).abs() < 1e-6,
        "全左/全右必须对称: {hard_l} vs {hard_r}"
    );

    // 3) 中心：`cos(π/4) = sin(π/4) = √2/2` ⇒ 左样本 = 全左样本 × √2/2。
    let centre = render(&tuned_project(&notes, quiet(0.0)).project, 60);
    let mut worst = 0.0f64;
    for (centre_sample, hard_sample) in centre.left.iter().zip(hard_left.left.iter()) {
        let expected = f64::from(*hard_sample) * CENTRE_GAIN;
        worst = worst.max((f64::from(*centre_sample) - expected).abs());
    }
    println!("[engine-mix] M2 worst |centre - hard_left*√2/2| = {worst:e}");
    assert!(
        worst < 1e-6,
        "居中样本必须等于全左样本 × √2/2（最大偏差 {worst:e}）"
    );

    // 4) 中右（pan = 0.5）：θ = 3π/8 ⇒ (cos, sin) 比值 = tan(3π/8)。
    let half_right = render(&tuned_project(&notes, quiet(0.5)).project, 60);
    let (l, _) = rms_peak(&half_right.left);
    let (r, _) = rms_peak(&half_right.right);
    let expected_ratio = (3.0 * core::f64::consts::FRAC_PI_8).tan();
    let ratio = r / l;
    assert!(
        (ratio - expected_ratio).abs() / expected_ratio < 1e-5,
        "pan=0.5 的左右比应为 tan(3π/8) = {expected_ratio:.6}, 实际 {ratio:.6}"
    );
}

/// M4：过阈值的夹具经**母线限制器**后峰值不超过天花板，且限制器确实压过样本。
///
/// 夹具：一个四分音符（力度 127）**不加增益**时峰值约 0.711（声相居中 = −3 dB），
/// 因此这里给 +6 dB 音量把它推到阈值之上。
#[test]
fn bus_limiter_caps_an_over_threshold_fixture() {
    let notes = [NoteSpec::at(0, 960, 69, 127)];
    let rendered = render(&tuned_project(&notes, MixSpec::volume(6.0)).project, 120);
    let peak = rendered.peak();

    println!(
        "[engine-mix] M4 peak={peak:.6} reductions={} ceiling={:.3}",
        rendered.stats.limiter_gain_reductions,
        yeban_engine::mixer::LIMITER_CEILING,
    );

    assert!(
        rendered.stats.limiter_gain_reductions > 0,
        "夹具没有驱动限制器（reductions = 0）—— 这条判据会变成永真"
    );
    assert!(
        rendered.stats.limiter_max_reduction > 0.0,
        "最大压限量必须为正（否则'压过'只是计数错觉）"
    );
    assert!(
        peak <= yeban_engine::mixer::LIMITER_CEILING,
        "限制后的峰值 {peak} 超过天花板 {}",
        yeban_engine::mixer::LIMITER_CEILING
    );
    assert!(peak > 0.8, "峰值被压得太狠（{peak}）—— 弹道或阈值写错了");
}

/// M5：**未过阈值**的工程逐位不被篡改（限制器在 1.0 增益下是恒等映射）。
///
/// 判别力来源：`reductions == 0` 是**结构性**证据（限制器一次也没压），
/// 加上"两次渲染逐位一致"（排除抖动/重采样之类的隐性改写）。
#[test]
fn sub_threshold_projects_are_not_touched_by_the_limiter() {
    let notes = [NoteSpec::at(0, 960, 69, 100)];
    // −12 dB ⇒ 峰值约 0.18，远低于 0.9 阈值。
    let quiet = tuned_project(&notes, MixSpec::volume(-12.0));
    let first = render(&quiet.project, 120);
    let second = render(&quiet.project, 120);

    println!(
        "[engine-mix] M5 peak={:.6} reductions={} ceiling={:.3}",
        first.peak(),
        first.stats.limiter_gain_reductions,
        yeban_engine::mixer::LIMITER_CEILING,
    );

    assert!(first.peak() > 0.05, "夹具必须真的出声");
    assert!(
        first.peak() < yeban_engine::mixer::LIMITER_THRESHOLD,
        "夹具必须整体低于阈值（否则这条判据测的是另一件事）"
    );
    assert_eq!(
        first.stats.limiter_gain_reductions, 0,
        "未过阈值的工程不得被限制器改写任何一个样本"
    );
    assert_eq!(first.stats.limiter_max_reduction, 0.0);
    assert_eq!(
        first.left_bits(),
        second.left_bits(),
        "未过阈值的工程两次渲染必须逐位相同"
    );
}

/// M6：**确定性** —— 同输入两次独立装配 + 渲染必须逐位相同（含声相/限制器/滤波器）。
#[test]
fn mix_chain_is_byte_deterministic() {
    let notes = [
        NoteSpec::at(0, 480, 60, 127),
        NoteSpec::at(960, 960, 67, 64),
        NoteSpec::at(2400, 480, 72, 32),
    ];
    let fixture = tuned_project(&notes, MixSpec::tone(1_200.0, 0.3));
    let first = render(&fixture.project, 200);
    let second = render(&fixture.project, 200);

    assert!(first.nonzero() > 0, "对照渲染必须真的出声");
    assert_eq!(first.left_bits(), second.left_bits(), "左声道必须逐位相同");
    assert_eq!(first.right, second.right, "右声道必须逐位相同");
    assert_eq!(
        first.stats.voice_steals, second.stats.voice_steals,
        "窃取次数必须确定"
    );
}

/// M7：静音工程（**带音色与声相配置**）仍然逐位静音。
///
/// 这条比 `synth_render.rs` 的 J6 更强：它同时压住"滤波器自激/直流泄漏"
/// 与"限制器释放尾巴"两种可能的非零来源。
#[test]
fn silent_projects_stay_bit_silent_through_the_mix_chain() {
    let silent = render(&empty_project(), 32);
    assert_eq!(silent.nonzero(), 0, "空工程必须逐位静音");
    assert_eq!(silent.stats.limiter_gain_reductions, 0);

    // 有轨道、有音色配置、但**没有音符**：滤波器与限制器都接在链路上。
    let bare = tuned_project(&[], MixSpec::tone(200.0, 0.9));
    let rendered = render(&bare.project, 64);
    assert_eq!(rendered.nonzero(), 0, "无音符工程必须逐位静音");
    assert_eq!(
        rendered.stats.limiter_gain_reductions, 0,
        "静音不得驱动限制器"
    );
    assert_eq!(rendered.peak(), 0.0);
}

/// M8：**主总线推子端到端** —— 同一工程、同一输入，主总线 `volume_db = 0` 与 `= −6`
/// 各渲染一次；两次输出必须**逐样本逐位**满足 `out₋₆[n] == f32(out₀[n] × g)`，
/// 其中 `g` 是实时侧**真的武装**的那个增益（[`EngineRuntime::armed_master_gain`]）。
///
/// 为什么这是"声音"的证据（而不是"数据"的证据）：断言作用在**真实渲染路径**产出的
/// 样本位模式上 —— `EngineSnapshot::from_project` → `SnapshotSlot` →
/// `EngineRuntime::process_quantum`（`support` 模块刻意只走产品路径）。
///
/// 夹具刻意整体低于限制器阈值（音轨 −6 dB）：两次渲染都断言
/// `limiter_gain_reductions == 0`，因此限制器是**逐位恒等**映射
/// （`x * 1.0f32 == x`），比值不会掺入限制器的非线性。过阈值下的行为由 M9 钉住。
#[test]
fn master_fader_scales_the_rendered_output_sample_by_sample() {
    let notes = [NoteSpec::at(0, 960, 69, 127)];
    let fixture = tuned_project(
        &notes,
        MixSpec {
            volume_db: -6.0,
            ..MixSpec::pan(0.0)
        },
    );
    let master = fixture.master;

    let mut unity_project = fixture.project.clone();
    unity_project
        .tracks
        .get_mut(&master)
        .expect("`tuned_project` 的母线在 tracks 里")
        .volume_db = 0.0;
    let mut quiet_project = fixture.project.clone();
    quiet_project
        .tracks
        .get_mut(&master)
        .expect("`tuned_project` 的母线在 tracks 里")
        .volume_db = -6.0;

    // 抓**实时侧真的武装**的那个数（而不是自己算一个期望值）。
    let mut armed_unity = f32::NAN;
    let mut armed_quiet = f32::NAN;
    let unity = render_with(&unity_project, 120, 1, |quantum, rig| {
        if quantum == 2 {
            armed_unity = rig.runtime.armed_master_gain();
        }
    });
    let quiet = render_with(&quiet_project, 120, 1, |quantum, rig| {
        if quantum == 2 {
            armed_quiet = rig.runtime.armed_master_gain();
        }
    });

    let (unity_rms, unity_peak) = rms_peak(&unity.left);
    let (quiet_rms, quiet_peak) = rms_peak(&quiet.left);
    println!(
        "[engine-master] M8 armed: 0 dB = {armed_unity:.9} ({:#010x}) (−6 dB = {armed_quiet:.9} \
         ({:#010x})) peak: {unity_peak:.9} → {quiet_peak:.9} (ratio {:.9}) \
         rms: {unity_rms:.9} → {quiet_rms:.9} (ratio {:.9}) \
         fingerprint: {:#018x} → {:#018x}",
        armed_unity.to_bits(),
        armed_quiet.to_bits(),
        quiet_peak / unity_peak,
        quiet_rms / unity_rms,
        unity.fingerprint(),
        quiet.fingerprint(),
    );

    // 覆盖度（防"什么都没跑"的假绿）：
    assert!(unity_rms > 0.01, "对照渲染必须真的出声");
    assert_eq!(
        armed_unity.to_bits(),
        1.0f32.to_bits(),
        "0 dB 必须武装单位增益"
    );
    assert_ne!(
        armed_quiet.to_bits(),
        1.0f32.to_bits(),
        "−6 dB 必须武装**非**单位增益（否则下面的逐位断言会永真）"
    );
    assert_eq!(
        unity.stats.limiter_gain_reductions, 0,
        "夹具必须整体低于阈值（否则测的是限制器的非线性）"
    );
    assert_eq!(
        quiet.stats.limiter_gain_reductions, 0,
        "−6 dB 之后必须仍在阈值以下"
    );
    assert!(
        unity.peak() < yeban_engine::mixer::LIMITER_THRESHOLD
            && quiet.peak() < yeban_engine::mixer::LIMITER_THRESHOLD,
        "两次渲染都必须整体低于阈值: unity={} quiet={}",
        unity.peak(),
        quiet.peak()
    );

    // 差异必须是 −6 dB：峰值与有效值都按同一个线性标量缩放。
    let peak_ratio = f64::from(quiet_peak) / f64::from(unity_peak);
    let rms_ratio = quiet_rms / unity_rms;
    let expected = f64::from(armed_quiet);
    assert!(
        (peak_ratio - expected).abs() < 1e-6,
        "峰值比必须等于武装增益 {expected}（实测 {peak_ratio}）"
    );
    assert!(
        (rms_ratio - expected).abs() < 1e-6,
        "有效值比必须等于武装增益 {expected}（实测 {rms_ratio}）"
    );
    // 差 −6 dB 的算术（`10^(-6/20) = 0.5011872…`）。
    assert!(
        (peak_ratio - 0.501_187_2).abs() < 1e-6,
        "峰值比必须 ≈ 0.5011872（−6 dB）, 实测 {peak_ratio}"
    );
    assert_ne!(
        unity.fingerprint(),
        quiet.fingerprint(),
        "两次渲染的位模式必须不同（否则'没接线'也会绿）"
    );

    // **最强形式**：逐样本逐位恒等 `out₋₆[n] == f32(out₀[n] × g)`。
    assert_eq!(quiet.left.len(), unity.left.len());
    let mut worst = 0.0f64;
    for (quiet_sample, unity_sample) in quiet.left.iter().zip(unity.left.iter()) {
        let expected_sample = *unity_sample * armed_quiet;
        worst = worst.max((f64::from(*quiet_sample) - f64::from(expected_sample)).abs());
        assert_eq!(
            quiet_sample.to_bits(),
            expected_sample.to_bits(),
            "主总线推子必须是逐样本的 f32 标量乘: {unity_sample} × {armed_quiet}"
        );
    }
    for (quiet_sample, unity_sample) in quiet.right.iter().zip(unity.right.iter()) {
        let expected_sample = *unity_sample * armed_quiet;
        assert_eq!(
            quiet_sample.to_bits(),
            expected_sample.to_bits(),
            "右声道同理: {unity_sample} × {armed_quiet}"
        );
    }
    println!("[engine-master] M8 worst |out₋₆ − out₀×g| = {worst:e}（要求逐位 0）");
    assert_eq!(worst, 0.0, "逐样本偏差必须为 0（浮点比较，不是容差）");
}

/// M9：主总线推子在**母线限制器之前**（[`yeban_engine`] 的接线位置决定，见 `rt::scale_bus`）。
///
/// 判别力来源：音轨 +6 dB 时，0 dB 主总线的信号**越过**阈值 ⇒ 限制器真的压；
/// 主总线 −6 dB 先把它拉回阈值以下 ⇒ 限制器**不再**介入（`reductions == 0`）。
/// 若把推子移到限制器**之后**，两次渲染的限制器都会介入，`reductions == 0`
/// 这一半立即变红。这条判据把"位置"这个设计选择钉成可执行的事实。
#[test]
fn master_fader_sits_upstream_of_the_bus_limiter() {
    let notes = [NoteSpec::at(0, 960, 69, 127)];
    let fixture = tuned_project(&notes, MixSpec::volume(6.0));
    let master = fixture.master;

    let mut unity_project = fixture.project.clone();
    unity_project
        .tracks
        .get_mut(&master)
        .expect("`tuned_project` 的母线在 tracks 里")
        .volume_db = 0.0;
    let mut quiet_project = fixture.project.clone();
    quiet_project
        .tracks
        .get_mut(&master)
        .expect("`tuned_project` 的母线在 tracks 里")
        .volume_db = -6.0;

    let unity = render(&unity_project, 120);
    let quiet = render(&quiet_project, 120);
    println!(
        "[engine-master] M9 0 dB: peak={:.6} reductions={} | −6 dB: peak={:.6} reductions={}",
        unity.peak(),
        unity.stats.limiter_gain_reductions,
        quiet.peak(),
        quiet.stats.limiter_gain_reductions,
    );

    assert!(
        unity.stats.limiter_gain_reductions > 0,
        "夹具必须先越过阈值（reductions = 0 ⇒ 这条判据会变成永真）"
    );
    assert!(
        unity.peak() <= yeban_engine::mixer::LIMITER_CEILING,
        "限制后的峰值 {} 超过天花板",
        unity.peak()
    );
    assert_eq!(
        quiet.stats.limiter_gain_reductions, 0,
        "推子在限制器**之前** ⇒ −6 dB 必须先离开限制区（reductions 必须为 0）"
    );
    assert!(
        quiet.peak() < yeban_engine::mixer::LIMITER_THRESHOLD,
        "−6 dB 之后的峰值必须回到阈值以下: {}",
        quiet.peak()
    );
    assert!(
        quiet.peak() < unity.peak(),
        "拉低推子必须让输出变小: {} vs {}",
        quiet.peak(),
        unity.peak()
    );
}

/// M10：母线限制器的**当前压限**（`EngineStats::limiter_current_reduction`）是一条
/// **量规**，且与全程最大量自洽。
///
/// 为什么需要这条判据：`line/engine-mix` 的台账把"限制器的增益衰减表（GR）上报"
/// 登记为缺口 —— 当时只有"累计压过多少样本"与"全程最大压限"两个累计量，
/// **没有**按量子可读的"当前压了多少"（原文逐字见
/// [`yeban_engine::rt::EngineStats::limiter_current_reduction`] 的文档）。
///
/// 判别力来源（每一条各自打掉一种假绿）：
///
/// 1. **有牙**：逐量子采样该读数，其**最大值**必须与同一段渲染的
///    `limiter_max_reduction` **逐位相等**。后者由**另一条**代码路径维护
///    （`if reduction > max`）⇒ "只写常数" / "写错字段" / "少写一个量子"都会破这条等号；
/// 2. **覆盖度**：采样条数必须等于渲染的量子数（少采一个就可能漏掉最大值）；
/// 3. **是量规不是常数**：整段渲染里至少出现**两个**不同的位模式（音符结束之后
///    限制器释放 ⇒ 读数必然移动）；
/// 4. **可回落且释放走到底**：末次读数严格小于峰值，并在释放完成后**逐位**回到 `+0.0`
///    —— 这一条打掉"把全程最大量直接写进去"那种偷懒实现（它会让末次值等于峰值）；
/// 5. **透明时贴地**：未过阈值的夹具里它**逐位**为 `+0.0`（`reductions == 0` 是
///    结构性前提，两者一起读才排除"限制器没被驱动"的假绿）。
#[test]
fn limiter_current_reduction_is_a_gauge_that_agrees_with_the_running_maximum() {
    // 96 tick = 2 400 样本 ≈ 18.75 个量子；渲染 60 个量子 ⇒ 音符结束后还有
    // ~41 个量子的**释放尾巴**（`LIMITER_RELEASE_PER_SAMPLE` = 5.0e-5/样本）。
    let notes = [NoteSpec::at(0, 96, 69, 127)];
    let loud = tuned_project(&notes, MixSpec::volume(6.0));

    // `render_with` 的 `at` 在**每个量子的渲染之前**被调用 ⇒ `at(k)` 观察到的是
    // 第 k−1 个量子结束时的读数；渲染结束后再补上最后一个量子（`Render::stats`）
    // ⇒ 采样集合覆盖**每一个**有快照的量子的量子边界值。
    let mut sampled: Vec<f32> = Vec::new();
    let rendered = render_with(&loud.project, 60, 1, |quantum, rig| {
        if quantum > 0 {
            sampled.push(rig.runtime.stats().limiter_current_reduction);
        }
    });
    let last = rendered.stats.limiter_current_reduction;
    sampled.push(last);

    let peak = sampled.iter().copied().fold(0.0f32, f32::max);
    let distinct = {
        let mut bits: Vec<u32> = sampled.iter().map(|value| value.to_bits()).collect();
        bits.sort_unstable();
        bits.dedup();
        bits.len()
    };
    println!(
        "[engine-mix] M10 采样={} 峰值当前压限={peak:.6} 末次={last:.6}（位模式 {:#010x}）\
         全程最大={:.6} 不同位模式={distinct} reductions={}",
        sampled.len(),
        last.to_bits(),
        rendered.stats.limiter_max_reduction,
        rendered.stats.limiter_gain_reductions,
    );

    // (1) 夹具真的驱动了限制器 —— 否则下面每一条都可能是永真。
    assert!(
        rendered.stats.limiter_gain_reductions > 0,
        "夹具没有驱动限制器（reductions = 0）—— 这条判据会变成永真"
    );
    // (1b) 覆盖度：采样必须覆盖**每一个**量子（少一个就可能漏掉最大值）。
    assert_eq!(
        sampled.len(),
        60,
        "采样必须覆盖每一个量子（`at` 在每个量子渲染之前被调用一次）"
    );
    // (2) 量规是**动**的：至少两个不同的位模式。
    assert!(
        distinct > 1,
        "当前压限在整段渲染里恒为一个值（不同位模式 {distinct} 个）⇒ 它不是量规"
    );
    // (3) 逐量子采样取最大值 == 全程最大量（两条代码路径之间的**逐位**等号）。
    assert_eq!(
        peak.to_bits(),
        rendered.stats.limiter_max_reduction.to_bits(),
        "逐量子采样的最大值必须与 limiter_max_reduction 逐位相等: {peak} vs {}",
        rendered.stats.limiter_max_reduction
    );
    // (4) 可回落：释放发生之后末次值严格小于峰值。
    assert!(peak > 0.0, "峰值当前压限必须为正（否则'压过'只是计数错觉）");
    assert!(
        last < peak,
        "音符结束之后量规必须回落: 末次={last} 峰值={peak}"
    );
    assert!(last >= 0.0, "压限量不得为负: {last}");
    // (4b) 释放**走到底**：从峰值衰减 60 个量子之后回到逐位 `+0.0`（增益弹道只有
    // f32 加/比较与一次除法，属 [ADR-0001 D32] 的 IEEE 精确类 ⇒ 这条等号跨架构成立；
    // 它同时钉住"量规跟的是限制器的**释放**，而不是某个只减不归零的中间量"）。
    assert_eq!(
        last.to_bits(),
        0.0f32.to_bits(),
        "释放完成后当前压限必须逐位回到 +0.0: {last}"
    );

    // (5) 未过阈值：量规**逐位**贴地（限制器透明 ⇒ `1.0 − 1.0 = +0.0`）。
    let quiet = tuned_project(&notes, MixSpec::volume(-12.0));
    let mut quiet_samples: Vec<f32> = Vec::new();
    let quiet_rendered = render_with(&quiet.project, 30, 1, |quantum, rig| {
        if quantum > 0 {
            quiet_samples.push(rig.runtime.stats().limiter_current_reduction);
        }
    });
    println!(
        "[engine-mix] M10 未过阈值: 采样={} reductions={} 末次={:.6}",
        quiet_samples.len(),
        quiet_rendered.stats.limiter_gain_reductions,
        quiet_rendered.stats.limiter_current_reduction,
    );
    assert_eq!(
        quiet_rendered.stats.limiter_gain_reductions, 0,
        "未过阈值的夹具不得被限制器改写任何一个样本"
    );
    for (index, value) in quiet_samples.iter().enumerate() {
        assert_eq!(
            value.to_bits(),
            0.0f32.to_bits(),
            "未过阈值时当前压限必须逐位为 +0.0（第 {index} 个采样是 {value}）"
        );
    }
    assert_eq!(
        quiet_rendered.stats.limiter_current_reduction.to_bits(),
        0.0f32.to_bits(),
        "未过阈值时末次读数必须逐位为 +0.0"
    );
}
