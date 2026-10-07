//! **设备腿**的判据：真实时钟驱动音频线程（本票的核心交付）。
//!
//! 被测对象是 [`yeban_app::engine_host::EngineHost::open_device`] —— 它把这一代引擎
//! **交给 `yeban_engine::device`**（`open_output` + `play()`），于是
//! `EngineRuntime::process_quantum` 由 **cpal 的回调线程**按设备时钟驱动，
//! 而不是由控制面推量子。
//!
//! # 这些判据各自证明什么（以及**不**证明什么）
//!
//! | 判据 | 有声卡的机器 | 无声卡的机器（含托管 CI） |
//! | :--- | :--- | :--- |
//! | `a_required_exclusive_mode_fails_gracefully...` | ✅ 真跑（在**碰设备之前**就明确失败） | ✅ 真跑（同一条路径） |
//! | `opening_a_device_before_any_engine_is_an_explicit_error` | ✅ 真跑 | ✅ 真跑 |
//! | `the_control_driven_engine_survives_a_failed_device_open` | ✅ 真跑 | ✅ 真跑（用 `RequireExclusive` 造确定的失败） |
//! | `the_device_leg_opens...a_real_clock_advances...` | ✅ 两条分支都断言（真读数） | ✅ 只跑 `Err` 分支（断言"失败是优雅的"） |
//! | `the_device_leg_carries_the_transport_position...` | ✅ 真读数 | ⚠ 只跑 `Err` 分支并打印 `本机未执行` |
//! | `the_device_wiring_never_names_a_raw_cpal_type` | ✅ 真跑（纯源码判据） | ✅ 真跑 |
//!
//! ⚠ **真设备分支若未执行，本文件会打印一行 `[device-leg] 本机未执行` 并且不冒充通过**
//! —— 那是"未被判定"，不是"通过"（与 macOS 上 Golden 判据的措辞同一条纪律）。
//!
//! # 红线（本票最高约束）
//!
//! 音频线程上的**唯一**新路径是 `device::render_callback` → `process_quantum`。
//! 它的零分配/零锁/零 I-O 判据在 `crates/yeban-engine/tests/rt_zero_alloc.rs` 的**判据 ⑳**
//! （不打开任何设备即可执行）。本文件**不**声称覆盖那条红线：它证明的是"这条路径被接线了"
//! 与"接线失败时产品仍可用"。

use std::time::{Duration, Instant};

use yeban_app::bridge::demo_project;
use yeban_app::engine_host::{EditMark, EngineHost, EngineHostError};
use yeban_engine::device::{DeviceError, EngineConfig, ShareMode};
use yeban_engine::transport::{TransportReading, TransportState};

/// 造一代**控制驱动**的引擎（`reload(.., 0)` ⇒ 不空转），并记下开局标记。
fn control_driven_host() -> EngineHost {
    let mut host = EngineHost::new();
    host.reload(&demo_project(), 0)
        .expect("演示夹具必须能建一代引擎");
    host.mark_applied(EditMark::new(None, 0));
    host
}

/// 判据 1（**本票核心**）：设备腿打开之后，**真实时钟**推进走带位置。
///
/// 量什么（有声卡分支）：`play()` 之后**真实时间** `elapsed` 里，引擎读数
/// `position_frames` 的增量（单位 = 帧）。判据是增量与 `elapsed × sample_rate`
/// 在 **2 倍容差**内一致 —— 容差写出来是因为"打开流到第一个回调"有一段不确定的
/// 启动延迟；2 倍足以排除"根本没被驱动"（那会给 0）与"另一条时钟在推"（那会差一个量级）。
///
/// 无声卡分支（托管 CI 的常态）：只断言**失败是优雅的**（`DeviceError` 的明确一类、
/// 引擎仍在、控制驱动仍能推量子），并打印 `本机未执行`。
#[test]
fn the_device_leg_opens_the_sound_card_and_a_real_clock_advances_the_transport() {
    let mut host = control_driven_host();
    let stopped = host.stop();
    assert_eq!(stopped.state, TransportState::Stopped, "开局先停在 Stopped");

    match host.open_device(EngineConfig::default()) {
        Ok((opening, _collector)) => {
            println!(
                "[device-leg] 设备=`{}` 协商={} Hz / {} ch 请求每回调 {} 帧 (Fixed={}) 共享模式={:?} \
                 交接量子={} @ tick {} ({:?})",
                opening.device_name,
                opening.sample_rate,
                opening.channels,
                opening.block_frames,
                opening.fixed_block_accepted,
                opening.share_mode,
                opening.carryover_quanta,
                opening.carryover_position_ticks,
                opening.carryover_state,
            );
            assert!(!opening.device_name.is_empty(), "设备名不能为空");
            assert!(opening.sample_rate > 0, "协商出的采样率必须为正");
            assert!(opening.channels >= 1, "协商出的通道数必须 ≥ 1");
            assert!(host.device_active(), "设备腿必须活跃");
            assert!(host.has_engine(), "设备腿活跃 ⇒ has_engine 必须为真");
            assert!(host.transport_ready(), "设备腿活跃 ⇒ 走带命令必须发得出去");
            assert_eq!(
                host.device_name(),
                Some(opening.device_name.as_str()),
                "读回的设备名必须就是打开的那一台"
            );
            assert_eq!(
                host.engine_stats(),
                None,
                "已登记的边界：设备腿活跃时 EngineStats 不可读（运行时在音频线程上）"
            );
            assert_eq!(
                host.pump(4),
                0,
                "设备腿活跃时控制面**不许**推量子（推了就是第二条音频路径）"
            );

            // ---- 真实时钟推进 ----
            let before = host.transport();
            let start = Instant::now();
            let playing = host.play();
            assert_eq!(
                playing.state,
                TransportState::Playing,
                "play() 必须等到设备确认（commands_applied 前进）—— 否则界面画的是命令之前的状态"
            );
            std::thread::sleep(Duration::from_millis(120));
            let after = host.transport();
            let elapsed = start.elapsed();
            let advanced = after.position_frames.saturating_sub(before.position_frames);
            let expected = opening.sample_rate as u128 * elapsed.as_millis() / 1_000;
            let ratio = advanced as f64 / expected.max(1) as f64;
            println!(
                "[device-leg] elapsed={elapsed:?} 位置 {} → {} 帧（增量 {advanced} 帧；\
                 elapsed × {} Hz = {expected} 帧 ⇒ 比值 {ratio:.3}）状态 {:?} → {:?}",
                before.position_frames,
                after.position_frames,
                opening.sample_rate,
                before.state,
                after.state,
            );
            assert!(
                after.position_frames > before.position_frames,
                "真实时钟必须让位置前进（实测 {} → {}）",
                before.position_frames,
                after.position_frames
            );
            assert!(
                (advanced as u128) * 2 >= expected,
                "推进量不得低于真实时间的一半：{advanced} 帧 vs 期望 {expected} 帧"
            );
            assert!(
                (advanced as u128) <= expected.saturating_mul(2),
                "推进量不得高于真实时间的两倍：{advanced} 帧 vs 期望 {expected} 帧"
            );

            // ---- 停住之后位置真的冻住 ----
            let stopped = host.stop();
            assert_eq!(stopped.state, TransportState::Stopped);
            let frozen = host.transport();
            std::thread::sleep(Duration::from_millis(60));
            assert_eq!(
                host.transport().position_ticks,
                frozen.position_ticks,
                "Stopped 之后设备回调仍在跑，但位置不许再前进"
            );
            assert_eq!(
                host.device_backend_errors(),
                Some(0),
                "cpal 错误回调不许被触发（它只做原子自增 [红线 7]）"
            );
        }
        Err(error) => {
            println!(
                "[device-leg] 本机未执行真设备分支：没有可用输出设备（字面错误 = {error}）；\
                 仅断言「失败是优雅的」"
            );
            assert!(
                matches!(error, EngineHostError::Device(_)),
                "失败必须是 DeviceError 的明确一类，实际 {error:?}"
            );
            assert!(host.has_engine(), "开设备失败不得清空引擎");
            assert!(!host.device_active(), "失败不得留下设备腿");
            assert!(
                host.engine_stats().is_some(),
                "失败后仍是控制驱动 ⇒ 统计必须仍可读"
            );
            assert_eq!(host.pump(1), 1, "失败后控制驱动仍能推量子（产品可用）");
        }
    }
}

/// 判据 2（**无声卡也能跑**）：要求独占模式 ⇒ 在**碰设备之前**就明确失败，
/// 而手里那一代**一位不动**。
///
/// 为什么这条判据有价值：它是**确定性**的失败注入（`negotiate` 的第一行就返回
/// `ExclusiveModeUnsupported`，见 `crates/yeban-engine/src/device.rs`），因此
/// **不需要**任何设备、在任何机器上都真跑。它同时钉住"优雅失败"的结构：
/// `open_device` 若先把运行时交出去再判独占，`engine_stats()` 会变 `None`、
/// `pump` 会返回 0 ⇒ 这条判据当场变红。
#[test]
fn a_required_exclusive_mode_fails_gracefully_and_keeps_the_control_driven_engine() {
    let mut host = control_driven_host();
    let playing = host.play();
    assert_eq!(playing.state, TransportState::Playing);
    let before = host.transport();

    let config = EngineConfig {
        share_mode: ShareMode::RequireExclusive,
        ..EngineConfig::default()
    };
    let error = host
        .open_device(config)
        .expect_err("RequireExclusive 必须明确失败（cpal 0.18 没有独占 API）");
    println!("[device-leg] RequireExclusive 的字面错误 = {error}");
    assert!(
        matches!(
            error,
            EngineHostError::Device(DeviceError::ExclusiveModeUnsupported)
        ),
        "必须是 ExclusiveModeUnsupported，实际 {error:?}"
    );

    // 手里那一代**一位不动**：还是控制驱动、统计可读、还能推量子、走带照常前进。
    assert!(!host.device_active());
    assert!(host.has_engine());
    assert!(
        host.engine_stats().is_some(),
        "开设备失败之后运行时必须仍在控制线程手里"
    );
    assert_eq!(host.pump(1), 1, "控制驱动仍能推量子");
    let after = host.transport();
    assert_eq!(
        after.state,
        TransportState::Playing,
        "状态不许被开设备动作改掉"
    );
    assert!(
        after.position_frames > before.position_frames,
        "走带位置必须继续前进：{} → {}",
        before.position_frames,
        after.position_frames
    );
    assert_eq!(
        host.snapshot_counts().expect("有引擎必有计数").published,
        1,
        "开设备失败不得发布 / 换快照"
    );
}

/// 判据 3：还没有一代引擎时开设备 ⇒ `NoEngine`（明确的返回值，不是 panic）。
#[test]
fn opening_a_device_before_any_engine_is_an_explicit_error() {
    let mut host = EngineHost::new();
    let error = host
        .open_device(EngineConfig::default())
        .expect_err("没有引擎时开设备必须报错");
    assert!(matches!(error, EngineHostError::NoEngine), "实际 {error:?}");
    assert!(!host.has_engine());
    assert!(!host.device_active());
    assert_eq!(host.pump(3), 0, "没有引擎时推量子恒为 0");
    assert_eq!(host.transport(), TransportReading::cold());
}

/// 判据 4：设备腿**继承**控制驱动的走带位置（不是从 tick 0 重新开始）。
///
/// 这条判据钉住"不许另造一条未初始化的音频路径"：`open_device` 必须先往**新**运行时
/// 交一条 `Stop` + `SeekTicks(当前位置)` 并推 1 个量子让它生效。把这一步删掉 ⇒ 新运行时
/// 是 `Transport::free_running(48_000, ..)` 的默认值（Playing @ 0）⇒ 这里的断言全红。
///
/// **位置的口径是 tick**（960 PPQ 是模型层的权威 [MODEL-ISO-001]），因此：
/// - **tick 必须逐位相等**（这一条是判据的牙）；
/// - **帧是 tick 的派生量**：`TransportCommand::SeekTicks` 的语义是"位置恰好是 tick、
///   相位余数清零"（`Transport::seek` 的文档），因此帧位置由 tick 重新导出，可能比
///   控制驱动累计出来的帧数**小不到一个 tick**（实测 1408 → 1400 帧 = 0.32 tick）。
///   这里断言的是由算术可证的**不变式** `frames_for(floor(t)) <= frames`
///   （tick 是向下取整 ⇒ 重新导出的帧数不可能超过累计帧数），而不是相等。
///
/// 无声卡分支：打印 `本机未执行` 并改为断言失败是优雅的。
#[test]
fn the_device_leg_carries_the_transport_position_of_the_control_driven_engine() {
    let mut host = control_driven_host();
    // 先在**控制驱动**形态下把位置推到非零（`play()` 本身推 1 个量子，再推 10 个）。
    host.play();
    assert_eq!(host.pump(10), 10, "控制驱动必须真推 10 个量子");
    host.stop();
    let before_open = host.transport();
    assert!(
        before_open.position_frames > 0,
        "夹具必须给出非零位置（实测 {}）",
        before_open.position_frames
    );
    assert_eq!(before_open.state, TransportState::Stopped);

    match host.open_device(EngineConfig::default()) {
        Ok((opening, _collector)) => {
            println!(
                "[device-leg] 交接读数：carryover_quanta={} @ tick {} ({:?})；\
                 打开之后引擎读数 = {:?}；控制驱动的帧位置 {} 帧",
                opening.carryover_quanta,
                opening.carryover_position_ticks,
                opening.carryover_state,
                host.transport(),
                before_open.position_frames,
            );
            assert_eq!(
                opening.carryover_quanta, 1,
                "交接只允许推 **1** 个量子（把新运行时带到当前状态）"
            );
            assert_eq!(
                opening.carryover_position_ticks, before_open.position_ticks,
                "交接位置必须等于控制驱动的当前位置"
            );
            assert_eq!(opening.carryover_state, TransportState::Stopped);
            let after = host.transport();
            assert!(
                before_open.position_ticks > 0,
                "夹具必须给出非零 tick 位置（否则这条判据测的是 0 == 0）"
            );
            assert_eq!(
                after.position_ticks, before_open.position_ticks,
                "设备腿的 tick 位置必须**恰好**是交接位置（0 就是「另造了一条未初始化的路径」）"
            );
            assert!(
                after.position_frames <= before_open.position_frames,
                "tick 是向下取整 ⇒ 重新导出的帧位置不可能超过累计帧位置：{} 帧 vs {} 帧",
                after.position_frames,
                before_open.position_frames
            );
            assert_eq!(
                after.state,
                TransportState::Stopped,
                "交接时是 Stopped ⇒ 设备腿不许自己跑起来"
            );
        }
        Err(error) => {
            println!("[device-leg] 本机未执行设备腿分支：没有可用输出设备（字面错误 = {error}）");
            assert!(
                matches!(error, EngineHostError::Device(_)),
                "实际 {error:?}"
            );
            assert!(host.has_engine());
            assert_eq!(
                host.pump(1),
                1,
                "失败之后仍必须能按控制驱动推进（产品可用）"
            );
        }
    }
}

/// 判据 5（纯源码判据，零编译）：接线路径**不许**出现裸 `cpal` 类型（ADR-0001 D19）。
///
/// 口径：把每一行的 `//` 之后（含 `//!` 文档）切掉，再找 `cpal::`。
/// 因此这一条**不是**"文件里没有 cpal 这个词"（文档里必须能谈论它），
/// 而是"**可执行代码**里没有裸 cpal 类型"。
///
/// 第二条断言是**防删除**：接线点必须真的在（`open_output` 与 `.open_device(`）。
#[test]
fn the_device_wiring_never_names_a_raw_cpal_type() {
    fn code_without_comments(source: &str) -> String {
        source
            .lines()
            .map(|line| match line.find("//") {
                Some(at) => &line[..at],
                None => line,
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
    for (name, source) in [
        ("engine_host.rs", include_str!("../src/engine_host.rs")),
        ("main.rs", include_str!("../src/main.rs")),
        ("host.rs", include_str!("../src/host.rs")),
    ] {
        let code = code_without_comments(source);
        assert!(
            !code.contains("cpal::"),
            "{name} 的可执行代码里出现了裸 cpal 类型（D19 要求设备宿主只属于引擎侧）"
        );
    }
    assert!(
        include_str!("../src/engine_host.rs").contains("yeban_engine::device::open_output"),
        "接线点必须是引擎的设备宿主（删掉它 = 设备腿不再是引擎侧的事）"
    );
    assert!(
        include_str!("../src/main.rs").contains(".open_device("),
        "生产路径必须真的调用 open_device（不调用 = 这一票没有接线）"
    );
}
