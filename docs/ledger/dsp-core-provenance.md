# `yeban-dsp` 移植来源、许可与逐文件裁决台账

- **台账类型**：来源追溯 / 许可归属 / 逐文件裁决（**不是规范**）
- **记录时刻**: 2026-10-05
- **工作线**: `line/dsp-core`（worktree `yeban/.worktrees/dsp-core`）
- **所有者目录**: `crates/yeban-dsp/**`（本台账是唯一新增的文档文件）
- **上游来源库**: `/Users/crow/work/music/synth/crates/synth-core`（单 crate、零依赖、MIT）
- **上游审计结论**: [`legacy-reuse-audit.md`](legacy-reuse-audit.md) §2 复用纪律
- **本次落地的目标 crate**: `crates/yeban-dsp`（13 模块，6,680 行，108 单元测试 + 1 文档测试）

> 本文件回答四个问题：**每一行代码从哪来**、**改了哪里以及为什么**、
> **哪些来源文件被明确拒绝以及理由**、**许可归属要怎么写进 `THIRD_PARTY_LICENSES.md`**。
>
> 本文件由 `line/dsp-core` 这一条工作线撰写；`THIRD_PARTY_LICENSES.md` 是根级共享文件，
> 由集成者独占，因此 §6 给出的是**待集成者粘贴的条目原文**，而不是直接改写它。

---

## 1. 上游来源与许可

| 项 | 值 |
| :--- | :--- |
| 上游仓库 | `synth`（`/Users/crow/work/music/synth`） |
| 上游 crate | `synth-core` 1.0.0 |
| 许可 | MIT（`synth/LICENSE` 全文，`Copyright (c) 2026 GROOVE SYNTH GS-1 contributors`） |
| 与夜半的兼容性 | 兼容。MIT 可复制进 GPLv3 工程，条件是**保留版权与许可声明**（已逐文件保留） |
| 复用范围 | `src/dsp/**` 与 `src/fx_shaping.rs` |
| 未复用范围 | vendored DaisySP / Soundpipe / `c_bridge`（C/C++，与"纯血 Rust"宪章冲突）、`alloc_arena.rs`、`shim.rs`、`dual_filter.rs` |

每个移植文件的**第一段模块文档**都保留了这一行形状的声明（可机械核验）：

```
//! Ported from synth-core (MIT, Copyright (c) 2026 GROOVE SYNTH GS-1 contributors) <来源文件>
```

核验命令：

```bash
grep -l "Ported from synth-core (MIT, Copyright (c) 2026 GROOVE SYNTH GS-1 contributors)" \
  crates/yeban-dsp/src/*.rs
```

---

## 2. 逐文件裁决表（来源 → 目标）

行数一栏是**目标文件**的行数（含本 crate 新写的测试）。

| 来源（`synth-core`） | 目标（`crates/yeban-dsp/src/`） | 行数 | 改了哪里 | 为什么 |
| :--- | :--- | ---: | :--- | :--- |
| `src/dsp/adsr.rs` | `envelope.rs` | 454 | 类型改名 `Adsr`/`AdsrStage`；新增 `start_steal_fade()` 与 `STEAL_RELEASE_SECONDS = 3ms`；对 `NaN`/负数/0 参数做钳制；`set_sample_rate` 只在值真的变化时重算 | `Stage` 太泛；把 [ARCH-RT-004] 的"3ms 快速淡出"钉在 API 上，避免每个调用点各自记数字；退化参数会让系数变 `NaN` 并静默毒化整条包络 |
| `src/dsp/ladder.rs` | `filter.rs` | 362 | `set` → `configure`；`fast_tanh` 收敛到 `math::pade_tanh`；私有 `soft_clip` → `bounded_saturate`；**新增次正规数软件兜底** `DENORMAL_FLOOR = 1e-30` | 来源里有两个不同的 `soft_clip`（膝点 0.7 / 0.82），同名会让后来者接错线；[ARCH-RT-003] 要求根除次正规数慢路径，硬件 FTZ/DAZ 依赖平台，软件兜底是跨平台确定性的一道保险 |
| `src/dsp/wavetable.rs` | `oscillator.rs` | 1011 | `Table` → `Wavetable`、`Recipe` → `WaveRecipe`、`CycleError` → `WavetableError`；新增 `WavetableOscillator`（相位累加 + 按音高选 mip 级）；新增只读访问器 `level_samples`/`level_count`；`crate::dsp::util::fft` → `crate::math::fft` | 来源把相位累加器留在 `engine`/`voice` 里，但"选级 + 累加相位"正是抗混叠保证的落点——放在 DSP 层才能被单独判据锁住 |
| `src/dsp/lfo.rs` | `oscillator.rs`（同上） | — | `LfoWave` 枚举从 `crate::params` 搬进来并补文档；`render` 改为逐样本 `process` 的薄封装；新增 one-shot 回归测试 | `yeban-dsp` 不得依赖上层模型 crate；来源只在注释里声称 one-shot 行为而没有测试 |
| `src/dsp/noise.rs` + `src/dsp/util.rs` 的 `Rng` | `noise.rs` | 353 | RNG 与噪声颜色合并到一个模块；RNG 只保留**显式播种**构造；新增 `state()` 以支持渲染回放 | "噪声"与"随机数发生器"是同一个关注点；[ARCH-DET-001] 不允许隐式全局熵源 |
| `src/dsp/util.rs` | `math.rs` | 322 | 音高换算 / `soft_limit` / `lerp` / `fft` 全量移植；新增 `db_to_gain`/`gain_to_db`/`pade_tanh`/`sanitise_sample_rate`；`exp2` 增加 `NaN` 兜底 | 来源把 dB 换算散落在 `fx_shaping.rs` 与 `engine`；收敛成一处才能钉死"0 dB = 1.0、−∞ dB = 0.0" |
| `src/dsp/simd.rs` | `block.rs` | 115 | **只保留标量路径**；`unsafe` 的 `core::arch::wasm32` 向量路径整段删除；`debug_assert` 改为安全的长度截断 | [AGENTS.md 红线 8] 强制 `#![forbid(unsafe_code)]`；标量本就是非 wasm 目标的实际执行路径，向量化交给编译器自动向量化（不需要 `unsafe`） |
| `src/dsp/oversample.rs` | `oversample.rs` | 422 | 系数表的 9 位小数字面量改写为**最短往返表示**（f32 位模式逐一比对相同）；新增 `process_round_trip` 与 `latency_samples()` | 数值完全不变，只是不再触发 `clippy::excessive_precision`；把上/下两步收成一个调用，让"延迟由谁补偿"只有一个出口 [ARCH-PDC-001] |
| `src/dsp/delay.rs` | `delay.rs` | 656 | `setup` → `configure`（明确标注唯一的分配入口）；`process` 去掉冗余 `frames` 参数；新增 `is_configured`；删除来源里未被任何测试使用的 `peaks()` 辅助函数 | 来源的 `frames` 允许与切片长度不一致，那是实时路径上的越界 panic；死代码在 `-D warnings` 下会编译失败 |
| `src/dsp/comb.rs` | `comb.rs` | 353 | `set` → `tune`（与滤波器模块的 `set` 消歧）；五个 `*_for_debug` 访问器 → 正常只读 getter；`process` 改用 `zip`；两个测试模块合并；白噪声测试改用本 crate 的 `Rng` | "调试专用"不该出现在公开 API 上；`out[i]` 在长度不等时越界 panic |
| `src/dsp/reverb.rs` | `reverb.rs` | 686 | 存储真实采样率（来源用 `44_100.0 * sr_scale` 反推，在 `[22.05k, 96k]` 之外是错的）；`process` 增加"未配置即直通"守卫；`set_params` 对非有限值回落默认值；`process` 改用 `zip` | 未配置实例在来源里会索引空 `Vec` 而 panic；一个 `NaN` 旋钮会把整个梳状组的反馈变成 `NaN` |
| `src/fx_shaping.rs` | `shaping.rs` | 1260 | `gain_to_a` 收敛到 `math::db_to_gain` 的平方根；三个效果类型与三个参数结构补 `Default`；`process` 去掉冗余 `frames`；新增 `MAX_TRANSIENT_GAIN` 常量；**引擎层测试不移植**（见 §7） | `10^(db/40) == sqrt(10^(db/20))`，全 crate 只留一处 dB 换算；`yeban-dsp` 没有也不该有 `Engine` |
| `src/dsp/fmath.rs` | —（`math.rs` 只吸收了 Padé `tanh` 思路） | — | **不移植**，见 §4 | 它是"wasm32 C/C++ shim 的无 libc 数学"，含 `*mut i32` 裸指针 API；夜半不是 `no_std`，`f32::sin/exp/log2` 由 std 提供且更准 |
| `src/dsp/convolution.rs`、`src/dsp/sampler.rs`、`src/voice.rs` | — | — | **本次不移植**（非本次能力切片范围） | 按 `legacy-reuse-audit.md` §6：`sampler`/`convolution`/`voice` 应在 `yeban-sfz`/`yeban-engine` 就位后再移植；`voice.rs` 的 `MAX_VOICES = 32` 还必须按规范改为 512/1024 |

### 2.1 本 crate **新写**（无上游来源）的模块

| 目标 | 行数 | 规范依据 | 说明 |
| :--- | ---: | :--- | :--- |
| `smoothing.rs` | 354 | [ARCH-DSP-001]（§4.1 参数自动化平滑）/ [ARCH-RT-003] | 规范只给了公式 `y[n] = (1−α)x[n] + αy[n−1]`、τ ≈ 5 ms，没有给代码 |
| `loop_window.rs` | 258 | [ARCH-DSP-001]（§3.3 循环点微平滑窗） | 规范只给了公式 `w(n) = ½[1 − cos(πn/(N−1))]`、N = 64，没有给代码 |

---

## 3. 代码风格与"不逐行转译"的证据

来源与目标之间不是逐行对应，而是按夜半的约定重写。可机械核验的差异：

1. **采样率一律显式**：没有任何模块或算法把采样率当作隐式上下文；
   需要采样率的入口都以参数传入，并经过 `math::sanitise_sample_rate`
   （下限 `MIN_SAMPLE_RATE = 1000`）。来源的若干处各写各的
   （`.max(1000.0)` / `.max(8000.0)` / `sample_rate.max(1000.0)`），本次统一。
   唯一的例外是 `new()` 里**构造函数字段的默认值** 48 kHz（与来源一致，便于把实例
   放进 `static`）；每个实例在使用前仍必须调用 `set_sample_rate` / `configure` / `prepare`，
   未配置时 `Delay` 与 `Reverb` 都是显式直通（并各有判据锁住）。
2. **参数一律 `f32`**；频率 Hz、时间秒；`DelayParams`/`ReverbParams`/`EqParams` 等都以
   文档注释写明单位与钳制范围。
3. **无全局状态**：全 crate 无 `static mut`，无 `Mutex`，无 `OnceLock`。
4. **零堆分配**：`Vec` 只出现在构造期（`Wavetable` 的表、`Delay`/`CombFilter`/`Reverb` 的延迟线），
   且每个会分配的方法都带 `///` 注明"这是本类型唯一的分配入口，必须在音频回调之外调用"。
   `process*` 族内无 `push`/`Box::new`/`format!`/`collect`。
5. **`#![forbid(unsafe_code)]` + `#![deny(missing_docs)]`**：全部公开项、字段、枚举变体都有文档。
6. 每条模块文档都标注了它实现的规范 ID（`ARCH-RT-001`/`ARCH-RT-003`/`ARCH-RT-004`/
   `ARCH-PDC-001`/`ARCH-DSP-001`/`ARCH-DET-001`）。

---

## 4. 明确**不**移植的来源文件（逐条裁决）

| 来源 | 裁决 | 理由（证据） |
| :--- | :--- | :--- |
| `src/alloc_arena.rs` | 丢弃 | wasm 专用全局分配器；[ARCH-RT-001] 要求的是"预分配 + 回调内零分配"，不是自定义分配器 |
| `src/shim.rs` | 丢弃 | freestanding libc shim，只服务 `wasm32-unknown-unknown` 的无 libc 构建 |
| `src/dsp/fmath.rs` | 丢弃（只吸收 Padé `tanh` 思路） | 文件头自述"Freestanding math for the wasm32 C/C++ shim"，目的只是让 shim 不递归进 libcall；`frexpf(x, exp: *mut i32)` / `ldexpf` 是裸指针 API，移植它们必须引入 `unsafe`，直接违反 [AGENTS.md 红线 8]。夜半用 std 的 `f32::sin/exp/log2/powf`，精度更高 |
| `src/dsp/simd.rs` 的 `unsafe` 向量路径 | 丢弃 | 同上（`core::arch::wasm32` + `v128_load` 需要 `unsafe`）。只保留标量路径 → `block.rs` |
| `src/dual_filter.rs` | 丢弃 | `#[cfg(test)]` 专用测试台（审计 §2 已记载） |
| vendored `daisysp/`、`soundpipe/`、`c_bridge/` | 丢弃 | C/C++，与"纯血 Rust 原生"宪章冲突；且上游自己已经用 `dsp/ladder.rs`、`dsp/adsr.rs` 替换了其中会出问题的部分 |
| `src/dsp/convolution.rs`、`src/dsp/sampler.rs`、`src/voice.rs` | 本次不移植 | 归属 `yeban-sfz`/`yeban-engine`，等它们就位（审计 §6 待办 1） |

---

## 5. 规范与任务书之间的差异（裁决留痕）

> 按 [AGENTS.md](../../AGENTS.md) §5.7：**不得私自发明答案，也不得擅自改写 Normative 文档**。
> 这里只记录差异与本次采用的解释，最终裁决应由人类写进 `docs/adr/`（本工作线无权新增 ADR）。

### 5.1 循环点微平滑窗：π 还是 2π

| 出处 | 公式 | 端点与中点 |
| :--- | :--- | :--- |
| `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §3.3（**Normative**） | `w(n) = ½[1 − cos(πn/(N−1))]` | `w(0)=0`、`w(N−1)=1`、`w(32)=0.5125` |
| 本工作线任务书的示例判据 | "`w[0]==w[N-1]==0`、`w[N/2]≈1`" | 对应 `½[1 − cos(2πn/(N−1))]`（对称 Hann 窗） |

两者**不是同一条曲线**：任务书给的公式（π）与规范 §3.3 逐字一致，而任务书另外给的示例判据
描述的是 2π 的 Hann 窗。**本次采用 Normative 规范的 π 公式**，并按"一对互补窗"实现接缝平滑
（`fade_in` 用 `w`、`fade_out` 用 `w` 的镜像；由 `w(n) + w(N−1−n) ≡ 1` 保证接缝两侧都落到 0），
因为这才是"首尾相接无跳变"的正确构造。示例判据里的 `w[N/2]≈1` 已按规范改写为
"`w(N/2)` 略高于 ½"，并且该差异已被测试注释与本节同时记录。

**待人类裁决**：若产品意图实际是"两端为 0、中点为 1"的对称 Hann 窗，则需要一份
`docs/adr/` 记录并改动 §3.3 的公式；本次不擅自改规范。

### 5.2 `ARCH-RT-001` 家族 vs `ARCH-DSP-001`

任务书把参数平滑归到"`ARCH-RT-001` 家族"。规范正文里，τ ≈ 5 ms 的平滑滤波写在
**§4.1 [ARCH-DSP-001]**（纯 Rust 声学自愈与去爆音）；`ARCH-RT-001` 是"零堆分配、零阻塞锁"。
本实现**同时标注两者**：平滑算法本身对应 `ARCH-DSP-001`，零分配约束对应 `ARCH-RT-001`。

---

## 6. 许可与归属：请集成者写入 `THIRD_PARTY_LICENSES.md` 的条目原文

> `THIRD_PARTY_LICENSES.md` 是根级共享文件，由集成者独占（[DEV_WORKFLOW.md](../DEV_WORKFLOW.md)）。
> 以下是建议条目，**逐字**粘贴即可；许可全文来自 `/Users/crow/work/music/synth/LICENSE`。

```markdown
### synth-core (GROOVE SYNTH GS-1 DSP core)

- 上游: https://github.com/ 私有历史仓库 `synth`，crate `crates/synth-core`
- 许可: MIT
- 版权: Copyright (c) 2026 GROOVE SYNTH GS-1 contributors
- 夜半使用范围: `crates/yeban-dsp/src/{envelope,filter,oscillator,noise,math,block,
  oversample,delay,comb,reverb,shaping}.rs` 是 `synth-core` 的 `src/dsp/**` 与
  `src/fx_shaping.rs` 的改写移植（接口、命名与采样率传递方式均为夜半重写）。
- 逐文件裁决与差异记录: `docs/ledger/dsp-core-provenance.md`
- 许可全文:

```
MIT License

Copyright (c) 2026 GROOVE SYNTH GS-1 contributors

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```
```

**另外建议**（与本次移植相关但不属本工作线）：
`cargo deny check` 看不到源码级移植，因此 `THIRD_PARTY_LICENSES.md` 是本项目唯一的
MIT 归属载体；请在合并 `line/dsp-core` 的同一个 PR 里加上该条目，否则会构成许可瑕疵。

`allow`/`deny` 配置**无需改动**：`deny.toml` 只审依赖图，本次移植**没有新增任何依赖**
（`yeban-dsp` 的 `[dependencies]` 仍为空，只依赖 `std`）。

---

## 7. 测试口径：移植了什么、没移植什么

### 7.1 逐字/近似移植的来源回归测试（真 bug 的化石）

| 目标模块 | 来源测试 |
| :--- | :--- |
| `envelope.rs` | `attack_reaches_peak_then_decays_to_sustain`、`release_decays_to_idle_and_reports_inactive`、`zero_sustain_voice_finishes_while_key_held`、`retrigger_during_release_restarts_attack`、`release_override_is_used_for_stealing` |
| `filter.rs` | `stays_continuous_on_a_sine`（爆音连续性）、`low_pass_attenuates_above_the_cutoff`、`resonance_lifts_the_cutoff`、`full_resonance_stays_bounded`、**`is_transparent_below_the_knee`（手写加窗 DFT，断言 2–12 次谐波总能量 < −70 dB）** |
| `oscillator.rs` | `every_level_is_band_limited_to_its_own_nyquist`、`every_level_is_full_length_and_still_normalised`、`the_level_chosen_for_a_note_cannot_alias`、`sampling_wraps_and_interpolates`、`an_imported_cycle_keeps_its_harmonic_balance`、`an_imported_cycle_keeps_its_phase`、`an_imported_cycle_is_band_limited_at_every_level`、`an_imported_cycle_is_normalised_and_dc_free`、`a_short_cycle_is_resampled_rather_than_refused`、`unusable_cycles_are_rejected_with_a_reason`、`sine_stays_in_range_and_advances_phase`、`square_is_bipolar` |
| `noise.rs` | `pink_falls_at_three_db_per_octave`、`brown_falls_at_six_db_per_octave`、`white_is_flat`（三者都是**频域**倍频程斜率测量）、`stays_bounded`、`rng_is_deterministic_and_bipolar` |
| `math.rs` | `note_to_hz_matches_concert_pitch`、`soft_limit_is_transparent_then_bounded`、`the_shared_fft_round_trips`、`the_shared_fft_puts_a_sine_in_one_bin` |
| `block.rs` | `accumulate_matches_scalar_reference`、`mix2_handles_non_multiple_of_four`、`peak_finds_maximum` |
| `oversample.rs` | `decimation_removes_everything_above_the_base_nyquist`、`round_trip_is_unity_at_dc`、`upsample_kills_the_image_above_the_base_nyquist`、`round_trip_delays_by_exactly_the_reported_latency` |
| `delay.rs` | `ping_pong_alternates_the_channels`、`a_plain_delay_keeps_the_channels_apart`、`echoes_decay_at_the_feedback_rate`、`damping_darkens_later_repeats`、`a_time_change_slides_instead_of_jumping`、`an_unallocated_delay_is_a_passthrough`、`two_instances_do_not_share_time_or_feedback` |
| `comb.rs` | `rings_at_its_tuned_frequency`、`stays_bounded_at_full_feedback`、`noise_becomes_periodic` |
| `reverb.rs` | `impulse_tail_decays_and_stays_bounded`、`damping_darkens_the_tail`、`width_controls_the_stereo_spread`、`pre_delay_holds_the_tail_back`、`sustained_tones_do_not_blow_up`、`long_input_never_blows_up` |
| `shaping.rs` | `bit_depth_sets_the_quantisation_step`、`divisor_and_anti_alias_move_the_mirror`、`deeper_divisors_push_the_mirror_lower_in_level_and_frequency`、`eq_flat_is_unity`、`eq_band_gains_match_the_cookbook`、`eq_shelves_reach_their_gain_at_the_ends_and_slope_back` |

### 7.2 **未能移植**的来源测试（需要别的工作线承接）

来源 `fx_shaping.rs` 的测试模块里有一半通过 `crate::engine::Engine` / `crate::params` /
`FxKind` / `Wave` / `MOD_ROUTES` 渲染。`yeban-dsp` 没有（也不该有）`Engine`，因此以下五条
**没有**移植，其归属是 `yeban-engine`：

1. `engine_dry_path_is_bit_exact_when_the_mix_is_zero`
2. `engine_transient_attack_moves_the_onset`
3. `engine_transient_sustain_moves_the_release`
4. `engine_transient_is_harmonically_clean`
5. `engine_crush_quantises_and_mirrors`
6. `engine_eq_bands_match_their_gain`
7. `abrupt_changes_stay_bounded_and_finite` —— **意图已在 DSP 层重落地**为
   `shaping::tests::abrupt_parameter_slams_stay_bounded`（逐块猛砸全部连续控制量）

### 7.3 本工作线**新写**的判据，以及"故意弄红"的实测证据

> 判据纪律：一条判据必须能被弄红。下表每一行都是**实际执行过**的变异测试：
> 改坏实现 → 跑该条判据 → 确认失败 → 还原 → 复跑全绿。

| # | 判据（测试名） | 它钉住的性质 | 故意破坏的方式 | 结果 |
| ---: | :--- | :--- | :--- | :--- |
| 1 | `smoothing::a_step_never_jumps_instantly` | 阶跃下无瞬时跳变；上界由**规范**（τ=5ms@48k）算出，不读被测对象的 `alpha()` | `recompute` 里 `alpha = 0.0`（关掉平滑） | **RED** |
| 2 | `smoothing::the_time_constant_is_five_milliseconds` | 63.2% 落在 1τ = 240 样本；α == `exp(-1/(τ·fs))` | 同 #1（α=0） | **RED** |
| 3 | `smoothing::degenerate_parameters_never_produce_nan_or_a_stuck_smoother` | 退化 τ/采样率不得产生 `NaN` α | 把守卫改成 NaN 不安全的 `if tau <= 0.0` | **RED** |
| 4 | `loop_window::the_window_matches_the_specified_curve` | 规范 π 公式的端点/中点/单调性 | 分母 `N−1` → `N` | **RED** |
| 5 | `loop_window::a_loop_seam_wraps_without_a_step` | 首尾相接无跳变（对照组未加窗时跳变 ≈1.0） | `fade_out` 的逆序遍历改成顺序 | **RED** |
| 6 | `oscillator::a_high_note_has_no_energy_outside_its_harmonics` | 端到端抗混叠：非谐波频点能量 < 基频 −40 dB（并给出"强行用第 0 级"的对照，证明测量看得见混叠） | `reselect_level` 固定 `level = 0` | **RED** |
| 7 | `oversample::round_trip_latency_is_reported_and_phase_linear` | `OS_LATENCY` == 群延迟；系数表对称 + 直流增益 1 | `OS_LATENCY = OS_CENTRE + 1` | **RED**（另 `round_trip_delays_by_exactly_the_reported_latency` 同步变红） |
| 8 | `delay::the_delay_time_is_seconds_not_samples` | 96 kHz 下同一条 50 ms 延迟落在 4800 样本 | `configure` 里采样率写死 48 kHz | **RED** |
| 9 | `comb::one_wavelength_per_delay_at_any_sample_rate` | 延迟长度 = 一个波长，且 96 kHz 下真的在 200 Hz 鸣响 | `tune` 里采样率写死 48 kHz | **RED** |
| 10 | `filter::never_emits_subnormal_tail_values` | 20 万静音样本内不得输出次正规数 | 删掉 `flush_denormals`（阈值→`false`） | **RED** |
| 11 | `reverb::an_unconfigured_reverb_is_a_passthrough` | 未配置实例直通且不 panic | 删掉 `!self.configured \|\| pre[0].is_empty()` 守卫 | **RED** |
| 12 | `shaping::the_transient_shaper_is_bit_exact_when_neutral` | 中性时增益路径严格是恒等 | 增益指数扰动 0.01 dB | **RED** |

**两条被自己推翻的判据（诚实记录，已写进对应测试的文档注释）**：

- `shaping::the_transient_shaper_is_bit_exact_when_neutral`：最初的文档声称"删掉 `neutral`
  短路会变红"——**假的**。`0.0 * x == 0.0`、`exp2(0.0) == 1.0`、`x * 1.0 == x`，因此那个
  短路在算术上冗余。判据本身仍然有效（破坏增益路径即红），但理由已更正。
- `smoothing::a_step_never_jumps_instantly`：最初的上界写成 `(1 − smoother.alpha()) + 1e-6`，
  即**从被测对象读系数**。于是把 α 改成 0 时上界恰好变成 1.0，"瞬时跳满"也能通过
  ——判据退化成同义反复。已改为由规范常量推导。

### 7.4 一次真实缺陷（本工作线新判据抓到的，不是来源的问题）

`smoothing.rs` 最初的吸附门限是固定的绝对量 `1e-9`。实测发现
`ParamSmoother::process` 在 τ = 5 ms @48 kHz、target = 1 时会**永久卡在 0.99999285**
（差值 7.15e-6）：此时每样本增量 `(1−α)·diff ≈ 3e-8` 小于 `value` 的半个 ulp，
加法被舍入掉，而绝对门限 1e-9 永远到不了。修法是改成相对门限
`1e-5 · max(|value|, |target|, 1)`（比浮点停摆点高一个量级，又远低于任何可听误差），
并把它写进了模块文档。这条缺陷由新判据 `a_step_never_jumps_instantly` 的
"30τ 后必须 `is_settled()`"一半暴露。

---

## 8. 边界、needs 与 pending

### 8.1 依赖上提（`TODO(hoist)`）

**无。** `crates/yeban-dsp/Cargo.toml` 的 `[dependencies]` 保持为空（只用 `std` +
crate 内部模块），因此：

- 没有新增任何依赖，`cargo deny check` 的依赖图不变；
- 没有需要写进根 `[workspace.dependencies]` 的条目，故 **本台账没有 `TODO(hoist)` 行**。
- **`Cargo.lock` 未被改动**（实测：`git status --short` 里没有它）。原因是 `yeban-dsp`
  本来就是 workspace 的 glob 成员且已有 `[lib]`，只是源码从占位骨架变成了实现，
  依赖图没有变化——这与"工作线允许一并提交 `Cargo.lock`"的授权不冲突，只是本次用不上。

### 8.2 边界（本工作线**没有**做、也不该做的事）

1. 未改动根 `Cargo.toml`、`AGENTS.md`、`README.md`、`.github/**`、`scripts/**`、
   `docs/DEVELOPMENT_LEDGER.md`、`THIRD_PARTY_LICENSES.md` 及任何法务/治理文件；
2. 未改动其它 `crates/**`、`spikes/**`、`schemas/**`；
3. 未新增 `docs/adr/` 条目（§5 的两处差异需要人类裁决后再落 ADR）；
4. 未移植 `convolution.rs` / `sampler.rs` / `voice.rs`（归属别的 crate）；
5. `yeban-dsp` 仍是**纯数学**层：没有实现 `ARCH-PDC-001` 的总线级延迟对齐、
   `ARCH-DSP-002` 的采样率转换、`ARCH-DSP-004` 的弹性拉伸——这些需要上层编排。

### 8.3 needs（交给集成者 / 其它工作线）

1. **必须**：把 §6 的条目原文加入 `THIRD_PARTY_LICENSES.md`（与合并同一个 PR）；
2. **需要裁决**：§5.1 的 π / 2π 窗差异 —— 若产品意图是对称 Hann 窗，需改规范 §3.3 并落 ADR；
3. 承接 §7.2 里七条引擎层测试（归属 `yeban-engine`）；
4. `voice.rs` 的声部抢占（`MAX_VOICES = 32` → 512/1024）需要 `yeban-engine` 落地时，
   用本 crate 的 `envelope::Adsr::start_steal_fade()` + `envelope::STEAL_RELEASE_SECONDS`
   兑现 [ARCH-RT-004]，用 `oversample::OS_LATENCY` / `Oversampler2x::latency_samples()`
   兑现 [ARCH-PDC-001] 的延迟上报。

### 8.4 pending（未验证 / 未拿到判决）

1. 本机的 `bash scripts/gates/run-gates.sh crate yeban-dsp` 为**本地**绿灯（fmt + 11 条红线守卫 +
   clippy `-D warnings` + 108 单元测试 + 1 文档测试）。按 [AGENTS.md](../../AGENTS.md) §5.5，
   只有 CI 判决算数；本台账不把本地绿写成"通过"。
2. `cargo deny check` 未在本机运行（需要 `cargo-deny` 二进制；见 `docs/DEV_WORKFLOW.md`），
   交给 CI 的 `deny` 档位。
3. 跨架构（x86_64）确定性对账未在本机运行（CI 专属，[ARCH-DET-001]）。
4. 未跑基准（本机禁止 benchmark；且 `yeban-dsp` 尚无 `benches/`）。

---

## 9. 接口速查（给集成者接线用）

```
envelope::Adsr             new/set_sample_rate/set_params/set_release/start_steal_fade/
                           reset/gate_on/gate_off/is_active/value/stage/process(gate)
envelope::STEAL_RELEASE_SECONDS = 0.003
filter::LadderFilter       new/configure(sr, cutoff_hz, resonance, drive)/reset/
                           process/process_block/coefficient/feedback
oscillator::Wavetable      from_recipe/from_cycle/level_for/level_len/level_count/
                           level_samples/sample
oscillator::WavetableOscillator  new(sr)/set_sample_rate(sr, &table)/
                           set_frequency(&table, hz)/set_phase/reset/process(&table)/
                           process_block(&table, out)/frequency/level/phase
oscillator::Lfo            new/reset/retrigger/shape/process(wave, rate_hz, sr)/
                           render(wave, rate_hz, sr, out)；字段 value / one_shot
oversample::Oversampler2x  new/reset/latency_samples/upsample/downsample/process_round_trip
oversample::{OS_TAPS=63, OS_CENTRE=31, OS_LATENCY=31}
delay::Delay               new/configure(sr)/is_configured/reset/max_seconds/
                           process(params, left, right)
comb::CombFilter           new/prepare(sr)/tune(sr, freq, resonance)/reset/
                           process(in, out)/feedback/damp/delay_samples/buffer_len
reverb::Reverb             new/set_sample_rate(sr)/set_params/is_configured/is_active/
                           params/process(left, right)
shaping::BitCrusher        new/reset/process(in_l, in_r, out_l, out_r, params, sr)
shaping::ShapingEq         new/reset/process(...)
shaping::TransientShaper   new/reset/process(...)
           参数结构：CrushParams / EqParams / TransientParams（都有 Default）
smoothing::ParamSmoother   with_default_time(sr)/new(sr, tau)/set_sample_rate/
                           set_time_constant/set_target/snap_to/value/target/alpha/
                           is_settled/process/render_block
loop_window::LoopWindow    new/len/is_empty/values/at/fade_in/fade_out
loop_window::raised_cosine / LOOP_WINDOW_LEN = 64
math::{note_to_hz, semitone_ratio, exp2, db_to_gain, gain_to_db, lerp, soft_limit,
      pade_tanh, fft}
noise::{Rng, NoiseGen, NoiseColour}
block::{accumulate, scale_into, mix2_into, peak}
```
