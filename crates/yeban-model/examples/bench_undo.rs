//! `[BASELINE-004]` 的**单步撤销时延**测量（目标 p99 ≤ 0.2 ms）。
//!
//! 规范（`docs/YEBAN_INDUSTRY_BENCHMARK_AND_OPEN_SOURCE_STUDY.md`）：
//! 「单步撤销时延 | 模型层逆操作应用耗时 | 测量 `Op` 逆向应用至状态树的 p99 耗时 | **≤ 0.2 毫秒** | Phase 1」。
//!
//! 在此之前本仓库**没有任何打点**：`yeban-model` 的逆操作判据只证明"正确"，不证明"够快"。
//!
//! # 为什么这条能在本机跑、而 `BASELINE-001` 不能
//!
//! 逆操作是**纯计算**：`yeban-model` 零重依赖（serde/ulid/sha2/libm/rand_xoshiro），
//! 所以这条读数**本机就有意义**（不像渲染吞吐那样必须用 `--release` 独占机器）。
//! 但仍有两条纪律：① 必须 `--release`；② **托管 runner 的绝对值只能给数量级** ——
//! 规范点名的"p99 ≤ 0.2 ms"要在指定参考硬件上复跑同一命令才算达标。
//!
//! 用法：
//! ```text
//! cargo run --release -p yeban-model --example bench_undo            # 默认 20,000 次
//! cargo run --release -p yeban-model --example bench_undo -- 100000  # 自定义次数
//! ```

use std::time::Instant;

use yeban_model::ops::Op;
use yeban_model::project::AutomationTarget;
use yeban_model::samples::filled_project;

/// 微秒表示（保留三位小数，方便人读）。
fn micros(duration: std::time::Duration) -> f64 {
    duration.as_secs_f64() * 1_000_000.0
}

/// 取一个**真实存在**的参数寻址目标**以及它当前的数值**。
///
/// 为什么要连当前值一起取：`SetParam` 的前置条件要求 `old_val` 与文档此刻的值一致
/// （否则返回 `OpStateMismatch`）—— 我第一次就是硬编码 0.25 而夹具里其实是 1200.0，
/// 于是判据在"apply"那一步就炸了。**逆操作测量必须建立在一个真的能施加的操作上。**
fn first_param_target(project: &yeban_model::project::YebanProjectV1) -> (AutomationTarget, f32) {
    for (track_id, track) in &project.tracks {
        // let-chain（edition 2024）：`clippy::collapsible_if` 要求把嵌套 if-let 合并。
        // 这个 lint 是**在 Windows 腿上第一次暴露的** —— 不是因为 Windows 特殊，
        // 而是因为这个文件的那一轮 run 被下一次推送取消了（见账本 L23）：**没跑过的判据不算判据**。
        if let Some(device) = track.devices.first()
            && let Some(param) = device.params.first()
        {
            return (
                AutomationTarget::DeviceParam {
                    track_id: *track_id,
                    slot_index: 0,
                    param_index: 0,
                },
                param.value,
            );
        }
    }
    panic!("夹具工程里应当至少有一条带参数设备的音轨");
}

/// 对给定的操作做 `iterations` 次"施加 → 计时逆操作"的采样，返回已排序的样本。
///
/// 注意：**只计逆操作本身**（`apply_inverse` = `invert` + `apply`），这正是"单步撤销"的语义。
fn sample_undo(
    doc: &mut yeban_model::project::YebanProjectV1,
    op: &Op,
    iterations: usize,
) -> Vec<std::time::Duration> {
    // 预热：让分配器/分支预测稳定（否则前几次噪声污染 p50）。
    for _ in 0..1_000.min(iterations) {
        op.apply(doc).expect("apply");
        op.apply_inverse(doc).expect("undo");
    }
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        op.apply(doc).expect("apply");
        let started = Instant::now();
        op.apply_inverse(doc).expect("undo");
        samples.push(started.elapsed());
    }
    samples.sort_unstable();
    samples
}

fn report(label: &str, samples: &[std::time::Duration]) {
    let pick = |numerator: usize| samples[(samples.len() - 1) * numerator / 100];
    let p50 = micros(pick(50));
    let p99 = micros(pick(99));
    let max = micros(*samples.last().expect("非空"));
    let mean = micros(
        samples
            .iter()
            .copied()
            .sum::<std::time::Duration>()
            .checked_div(u32::try_from(samples.len()).expect("样本数不大"))
            .expect("非空"),
    );
    println!(
        "BENCH baseline=004 name={label} iterations={} p50_us={p50:.3} p99_us={p99:.3} \
         max_us={max:.3} mean_us={mean:.3} target_p99_us=200.0 verdict={}",
        samples.len(),
        if p99 <= 200.0 {
            "within-target"
        } else {
            "over-target"
        }
    );
}

fn main() {
    let iterations: usize = std::env::args()
        .nth(1)
        .map_or(20_000, |raw| raw.parse().expect("次数必须是整数"));

    let mut doc = filled_project();
    let (target, current) = first_param_target(&doc);
    let next = if (current - 0.75).abs() < f32::EPSILON {
        0.25
    } else {
        0.75
    };

    // ① 最轻的标量操作（参数写入）。
    let set_param = Op::SetParam {
        target,
        old_val: current,
        new_val: next,
    };
    report(
        "undo_set_param",
        &sample_undo(&mut doc, &set_param, iterations),
    );

    // ② 宏写入（会级联到映射的参数 —— 比 ① 重）。
    let macro_target = doc
        .tracks
        .iter()
        .find(|(_, track)| !track.macros.is_empty())
        .map(|(track_id, track)| (*track_id, track.macros[0].value));
    if let Some((track_id, macro_value)) = macro_target {
        let macro_next = if (macro_value - 0.5).abs() < f32::EPSILON {
            0.25
        } else {
            0.5
        };
        let set_macro = Op::SetMacro {
            track_id,
            macro_index: 0,
            old_val: macro_value,
            new_val: macro_next,
        };
        report(
            "undo_set_macro",
            &sample_undo(&mut doc, &set_macro, iterations),
        );
    }

    // ③ 摆放平移（会改 BTreeMap 里的条目 —— 比标量重）。
    let placement = doc
        .tracks
        .iter()
        .flat_map(|(track_id, track)| {
            track
                .clips
                .values()
                .map(move |clip| (*track_id, clip.id, clip.start_tick))
        })
        .next();
    if let Some((track_id, placement_id, start_tick)) = placement {
        let move_clip = Op::MoveClipPlacement {
            track_id,
            placement_id,
            old_start_tick: start_tick,
            new_start_tick: start_tick + 960,
        };
        report(
            "undo_move_clip",
            &sample_undo(&mut doc, &move_clip, iterations),
        );
    }

    // ④ `Batch`：规范里"AI 提案一键撤销"的形态（多步合成一次撤销）。
    let batch = Op::Batch {
        ops: vec![
            set_param,
            Op::SetParam {
                target,
                old_val: next,
                new_val: current,
            },
        ],
        description: "bench: 两步批次的单步撤销".to_owned(),
    };
    report("undo_batch2", &sample_undo(&mut doc, &batch, iterations));

    println!(
        "BENCH baseline=004 note=\"纯计算, 本机 --release 读数有意义; 托管 runner 只给数量级, \
         达标需在指定参考硬件复跑同一命令\""
    );
}
