# `BASELINE-005`（音频硬件往返时延 ≤ 5.5 ms）台账：**机器那一半**、能力矩阵与诚实边界

- **台账类型**：能力矩阵 / 判据清单 / 实测证据 / 未决项（**不是规范**）
- **记录时刻**：2026 年，第 1 轮（本机 Apple M2 + 受限 `workspace-write` 沙箱）
- **工作线**：`line/audio-latency`（worktree `yeban/.worktrees/audio-latency`，main `2f79c09`）
- **所有者目录**：`crates/yeban-engine/**`（本台账是唯一新增的文档文件）
- **落地范围**：新增 `crates/yeban-engine/src/latency.rs`（**零 cpal 依赖**的纯计算与判定）、
  `crates/yeban-engine/examples/measure_latency.rs`（cpal 测量机器）、
  `crates/yeban-engine/tests/latency_cli_contract.rs`（运行期契约判据）；
  `src/lib.rs` 只加了模块注册与一段边界说明
- **规范来源**：`docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` §5.2 的 `[BASELINE-005]` 原文、
  `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §3.5 的 `[ARCH-PDC-002]` 延迟预算分解表、
  `ARCH-PDC-001`、`ARCH-RT-001`、`ARCH-DET-001`、`AGENTS.md §2 红线 7`
- **零新增依赖**：cpal 0.18.2 已在依赖图里（`Cargo.lock` 钉死）；
  本线**没有**改根 `Cargo.toml` / `Cargo.lock` / `.github/**` / `scripts/**` / `docs/DEVELOPMENT_LEDGER.md`

> 本文件回答六个问题：**到底哪一半测得了、哪一半测不了**、**标称与实测的区别**、
> **判据是怎么变红的**、**CI 上会发生什么**、**还要在什么机器上做什么才可能判达标**、
> **哪些东西明确没做**。

---

## 0. 一句话结论（先读这段，别读数字）

**`BASELINE-005` 依然是 `PENDING`，本线没有、也不可能把它变成绿。**

本线交付的是"**机器**"：能把**设备报告的标称时延**与**回调调度抖动**变成可复现的读数，
并且把"**测不到**"和"**达标**"在机械层面分开 —— 没有回环证据时，
`verdict` 字段**永远不可能是** `within-target`（这一条由判据钉死，见 §4 的注入 B）。

**真正的声学/DAC-ADC 往返时延一次都没有被测过。** 全仓库唯一真正需要音频硬件的门禁，
现在有了一把**不会说谎的尺子**，而不是一个达标数字。

---

## 1. 能力矩阵：能测 / 不能测（逐项）

| # | 量 | 能测？ | 事实来源 | 进入机器可读行？ |
| :---: | :--- | :---: | :--- | :--- |
| 1 | 设备名 / 驱动名 / 接口类型（`Built-in`/`USB`/…） | ✅ | cpal `DeviceTrait::description()` | `DEVICE` 行（人读） |
| 2 | 默认输入/输出配置：通道数、采样率、`f32` 格式 | ✅ | `default_input_config()` / `default_output_config()` | `DEVICE` 行（人读） |
| 3 | 设备报告的缓冲区间 `min..=max`，或 `Unknown` | ✅ | `supported_*_configs()` → `SupportedBufferSize` | `DEVICE` 行（人读） |
| 4 | 协商结果：`BufferSize::Fixed(n)` 还是 `Default` | ✅ | `device::negotiate()`（与本 crate 真实路径**同一份**纯函数） | 间接（标称行） |
| 5 | **后端报告的实际缓冲帧数** | ✅ | `DeviceTrait::buffer_size()`（**打开流之后**回读） | `buffer_frames_reported=` |
| 6 | **标称时延**（输入、输出**分别**）= 帧数 ÷ 采样率 | ✅ | 第 5 项（拿不到就用请求的 `Fixed(n)`；都没有 ⇒ `none`） | `nominal_out_ms=` / `nominal_in_ms=` |
| 7 | **回调调度抖动**（CPU 侧）p50 / p99 / max / mean | ✅ | 真实跑流的回调时间戳（`InputStreamTimestamp::callback`） | `jitter_*_p99_ms=` 等 |
| 8 | 后端错误回调次数（XRun/设备忙/失效） | ✅ | 错误回调里的原子自增 | `backend_errors_*=` |
| 9 | **驱动层报告的额外延迟**（ARCH §3.5 的 1.84 ms 那一格） | ❌ | cpal **不暴露** | 恒为 `driver_reported_extra_latency_ms=unknown` |
| 10 | **物理输入缓冲 + 输出缓冲的真实流水线时延** | ❌ | 需要 `snd_pcm_delay` / CoreAudio / WASAPI 厂商 API | — |
| 11 | **声学 / DAC-ADC 往返时延**（`BASELINE-005` 的本体） | ❌ | 需要**物理回环**（输出接输入）或虚拟回环设备 + 采集比对 | `measured_roundtrip_ms=none` |
| 12 | ARCH §3.5 里"内部 DSP 拓扑调度 1.00 ms"那一格 | ❌ | 属引擎渲染路径的**离线**测量，不在本工具范围 | — |

### 1.1 为什么第 9–11 项是 ❌ —— 已按 crate 源码核对，不是"懒得做"

`cpal` 版本由 `Cargo.lock` 钉死为 **0.18.2**（`cpal = { workspace = true, optional = true }`，
feature `device`）。核验方法（不需要声卡）：

```bash
# 从 registry 源码里确认 cpal 有没有暴露硬件时延查询
grep -rni "latency" "$(find ~/.cargo/registry/src -maxdepth 2 -type d -name 'cpal-0.18.2' | head -1)/src/"
```

结论（本线逐文件读过 `cpal-0.18.2/src/`）：**全 crate 里 `latency` 一词只出现在错误文案与
文档注释里**。`DeviceTrait` 的能力面只有
`description()` / `id()` / `supports_input()/supports_output()` / `supported_*_configs()` /
`default_*_config()` / `build_*_stream(_raw)()` / `play()` / `pause()` / **`buffer_size()`** / **`now()`**。
`now()` 只给"流自己的单调时钟"，**不是**设备流水线延迟。

⇒ 规范点名的三个厂商查询（macOS `kAudioDevicePropertyLatency` /
`kAudioStreamPropertyLatency`、WASAPI `IAudioClient::GetStreamLatency`、ALSA `snd_pcm_delay`）
**一个都拿不到**，除非新写 FFI（= **新依赖裁决**，不由本工作线单独决定，见 §7 needs-2）。

### 1.2 一个必须说清的口径陷阱：标称**不能**加总成往返

ARCH §3.5 的预算表是这么拆的（@48 kHz / 64 帧）：

| 阶段 | 时延 | 本工具能提供吗 |
| :--- | ---: | :--- |
| 物理声卡输入缓冲（64 帧） | 1.33 ms | 只能给**标称**（帧数 ÷ 采样率） |
| 内部 DSP 拓扑调度 | 1.00 ms | ❌（不在本工具范围） |
| 物理声卡输出缓冲（64 帧） | 1.33 ms | 只能给**标称** |
| 系统驱动与总线余量 | 1.84 ms | ❌ **cpal 不暴露** |
| **端到端回路总时延** | **5.50 ms** | ❌ **测不了** |

标称 1.33 + 1.33 = 2.67 ms **看起来**远低于 5.5 ms —— 正因如此，
机器可读行里**故意没有** `roundtrip_ms` 字段；`nominal_io_sum_ms` 旁边永远跟着
`nominal_io_sum_is_roundtrip=false`，且 `verdict` 绝不会因为这个小数字变成 `within-target`。
**谁把 `nominal_io_sum_ms` 当成往返时延去宣布达标，谁就制造了一个假绿。**

---

## 2. 标称 vs 实测：这不是"精度差别"，是**性质差别**

| | 标称时延（本工具能测） | 实测往返时延（本工具**测不了**） |
| :--- | :--- | :--- |
| 定义 | 设备报告的缓冲帧数 ÷ 采样率 | 一个样本从 DAC 出去、经模拟/数字回环、再被 ADC 采回来的时间 |
| 含驱动流水线？ | ❌ | ✅ |
| 含硬件 FIFO / 时钟域？ | ❌ | ✅ |
| 含回环路径本身？ | ❌ | ✅ |
| 会受 XRun / 调度抖动影响？ | ❌ | ✅ |
| 能否判定 `BASELINE-005`？ | **不能** | 能（且必须**同时**有回环证据） |

机器可读行里两者的**形状**就已经分开了：

- 标称：`nominal_out_ms=1.3333`（可以有数字，因为它是设备自己报的帧数）；
- 实测：`measured_roundtrip_ms=none` —— **本工具的所有路径上恒为 `none`**；
  只有 `Evidence::LoopbackMeasured` + 一个有限数字同时成立，判定才会离开
  `unmeasurable-without-loopback`。

---

## 3. 工具与判据：清单、命令、本机 vs CI

### 3.1 三个文件

| 文件 | 依赖 cpal？ | 本机 (M2) 能真跑？ | 作用 |
| :--- | :---: | :---: | :--- |
| `crates/yeban-engine/src/latency.rs` | **否**（零依赖） | ✅ `rustc --test` 真跑 | 标称换算、抖动统计（最近秩百分位）、机器可读行 + 严格解析、判定与退出码、"没测到 ≠ 达标" |
| `crates/yeban-engine/examples/measure_latency.rs` | **是** | ⚠ 不能对**真 cpal** 编译；用 API 仿真桩类型检查 + 真二进制跑无设备路径（见 §3.5） | 枚举设备、开真流、跑真实回调、打印可读行 |
| `crates/yeban-engine/tests/latency_cli_contract.rs` | 否（只 `Command` 起子进程） | ✅（对 cpal 那半**响亮 SKIP**） | 跑**真的二进制**并对输出做契约断言 |

**为什么把纯计算抽出来**：本机纪律不允许编译 cpal，所以"
判定逻辑"与"声卡 I/O"必须能分开验证。抽出来的那半在本机是**真跑**的
（不是"跳过所以绿"），含 cpal 的那半明确交给 CI，并额外用 §3.5 的两层手段
在本机做**编译/lint + 无设备路径**的验证 —— 但仍然**不声称**验证了真 cpal 的运行时行为。

### 3.2 命令

```bash
# 纯计算判据（**本机真跑**，无需声卡；CI 上等价于 cargo test -p yeban-engine）
rustc --edition 2024 --test -D warnings crates/yeban-engine/src/latency.rs -o /tmp/latency_pure && /tmp/latency_pure

# 工具本身（**必须有音频设备**；托管 CI runner 上会如实报 no-device）
cargo run --release -p yeban-engine --example measure_latency -- \
    --label <machine-name> --frames 64 --seconds 2 --sample-rate 48000

# 确定性"无设备"缝（任何机器上都能跑，用于判据 ③）
cargo run --release -p yeban-engine --example measure_latency -- --force-no-device
```

退出码：`0` 实测达标（本工具不产生）｜`1` 实测超目标（本工具不产生）｜`2` 用法错误｜
`3` **无设备**（没测到，**不是**达标）｜`4` **有设备但无回环**（不可判定，**不是**达标）｜
`5` 工具未能给出读数（编译时关掉了 `device` feature，或自检发现机器可读行坏了）。

`--allow-unmeasurable` 只把 3/4 映射成 0，好让 CI 的日志步骤不红：
**机器可读行的 `verdict=` 与 `measured_roundtrip_ms=` 一个字节都不会变**
（判据 `allow_unmeasurable_changes_the_exit_code_but_not_the_verdict` 直接比较两行的字符串相等）。

### 3.3 判据清单（**6 条主题 / 15 条纯计算判据 + 5 条运行期判据**）

| 编号 | 判据（测试名） | 主题 | 本机真跑 | CI |
| :---: | :--- | :--- | :---: | :---: |
| ① | `nominal_latency_conversion_is_exact` | 标称换算正确（64/48k=1.3333、128/48k=2.6667、128/44.1k=2.9025；采样率 0 ⇒ `None`） | ✅ | ✅ |
| ② | `jitter_percentiles_are_exact_on_handmade_timestamps` | 人造时间戳 ⇒ 钉死 p50/p99/max/mean；**且 `p99 > mean`**（右偏） | ✅ | ✅ |
| ② | `perfectly_regular_callbacks_have_zero_jitter` | 规律流 ⇒ 抖动恒为 0（工具不造噪声） | ✅ | ✅ |
| ② | `empty_or_degenerate_samples_never_invent_a_number` | "没采到" ≠ "抖动 0"（空 ⇒ `None`） | ✅ | ✅ |
| ③ | `the_no_device_path_reports_no_device_and_never_zero_ms` | 无设备 ⇒ `verdict=no-device`、退出码 3、文案含 `NOT a pass and NOT 0 ms`、行里没有 `0.0000` | ✅ | ✅ |
| ③ | `the_no_device_path_is_explicit_and_never_reports_zero_ms`（运行期） | 跑**真二进制** + `--force-no-device` | SKIP(loud) | ✅ |
| ④ | `device_present_reports_non_negative_nominal_and_jitter` | 有设备 ⇒ 标称与抖动都打印且非负、`p99 ≥ p50`、`max ≥ p99` | ✅ | ✅ |
| ④ | `a_machine_with_devices_reports_non_negative_nominals_and_jitter`（运行期） | 真二进制；有设备就跑，没设备就**响亮**说明"本环境无法测量" | SKIP(loud) | ✅ |
| ⑤ | `bench_line_round_trips_through_the_strict_parser` | 写出 → 严格解析 → 逐字段对账（含带空格的标签被规范化） | ✅ | ✅ |
| ⑤ | `the_parser_rejects_malformed_lines` | 解析器**有判别力**：错前缀 / `baseline=002` / 游离 token / 缺字段 / 非数字 ⇒ 全拒 | ✅ | ✅ |
| ⑤ | `unknown_buffer_frames_produce_none_instead_of_a_guess` | 帧数未知 ⇒ `none`，不猜 | ✅ | ✅ |
| ⑤ | `exit_codes_separate_unmeasured_from_pass` | 3/4 与 0 分开；便利开关不制造达标 | ✅ | ✅ |
| ⑤ | `the_convenience_flag_never_changes_the_machine_readable_verdict` | 开关不改判定、不改行 | ✅ | ✅ |
| ⑤ | `labels_are_sanitized_so_the_line_stays_parseable` | 标签规范化（空白/`=`/非 ASCII） | ✅ | ✅ |
| ⑤ | `usage_errors_exit_two_and_help_exits_zero`（运行期） | 用法错误 ⇒ 2；`--help` ⇒ 0；都不输出读数 | SKIP(loud) | ✅ |
| ⑥ | `nominal_values_can_never_satisfy_the_baseline` | **没有回环证据 ⇒ 永不达标**（连 0.001 ms 的"标称"也不行） | ✅ | ✅ |
| ⑥ | `loopback_evidence_is_the_only_route_to_a_pass` | 只有回环证据能到达标/超目标，且 5.5 ms 边界精确（`≤`） | ✅ | ✅ |
| ⑥ | `the_tool_never_claims_a_pass_and_always_prints_its_boundary`（运行期） | 真二进制：任何调用形态下 `verdict ≠ within-target`、`measured_roundtrip_ms=none`、`loopback=false`，且必打印 `MEASURES` / `DOES-NOT-MEASURE` | SKIP(loud) | ✅ |
| ⑥ | `target_and_baseline_id_match_the_specification` | 5.5 / `BASELINE-005` / `005` 字面量被钉住 | ✅ | ✅ |

**本机实测（M2，`rustc --edition 2024 --test -D warnings`）**：`15 passed; 0 failed`。
**本机实测（`cargo test -p yeban-engine --no-default-features`，即仓库内的轻量变体）**：
库 116 passed（含这 15 条）+ `latency_cli_contract` **5 passed**
（4 条打印了**响亮 SKIP**：本机 variant 不带 cpal）+ 其余既有判据全绿。
**本机实测（`/tmp` 副本 + cpal API 仿真桩，`device` feature 打开，见 §3.5）**：
`cargo clippy -p yeban-engine --all-targets -- -D warnings` 绿；
`latency_cli_contract` **5 passed**（只剩 1 条响亮 SKIP：仿真桩报 0 设备）。

### 3.4 注入记录（**3 条注入，全部"注入 → 变红 → 还原"**）

还原后 `diff` 与备份**逐字节相同**（`sha256 = fe29d276db00a749400bc61e15b0cd6b46bd0f1de5103a153ca412c98e3f558f`），
且 15 条判据重新全绿。

**注入 A —— "把'没有设备'当成 0 ms ⇒ 达标"**（改 `verdict_for` 的 `device_count == 0` 分支）：

```text
test tests::the_no_device_path_reports_no_device_and_never_zero_ms ... FAILED
assertion `left == right` failed
  left: WithinTarget
 right: NoDevice
```
对工具输出的影响（同一份纯模块渲染）：
`verdict=no-device` → **`verdict=within-target`**，退出码 `3` → **`0`**。
⇒ 这正是"无设备 runner 记成通过"的假绿，判据③挡住了。

**注入 B —— "把标称值当成实测往返"**（改 `verdict_for` 忽略 `evidence` + 让
`measured_roundtrip_ms` 回落到 `nominal_out_ms`）：

```text
test tests::nominal_values_can_never_satisfy_the_baseline ... FAILED
test tests::bench_line_round_trips_through_the_strict_parser ... FAILED
test result: FAILED. 13 passed; 2 failed
```
对工具输出的影响：`measured_roundtrip_ms=none` → **`measured_roundtrip_ms=1.3333`**
（一个**标称**值被写进了"实测往返"字段）。⇒ 判据①⑥挡住了。

**注入 C —— "抖动统计用均值冒充 p99"**（`jitter_stats` 的 `p99_ms` 改成 `mean`）：

```text
test tests::jitter_percentiles_are_exact_on_handmade_timestamps ... FAILED
test result: FAILED. 14 passed; 1 failed
```
⇒ 判据②挡住了（那条判据专门断言右偏分布下 `p99 > mean`）。

### 3.5 含 cpal 的那一半，本机是怎么验的（以及它的**边界**）

本机按纪律不编译 cpal，但"本机不编译"**不等于**"不用验"。所以做了两层真跑的验证，
两层都在 `/tmp` 的一次性副本里进行 —— **仓库里的 `Cargo.toml` / `Cargo.lock` 没有被改过**：

**第一层：API 仿真桩的类型检查 / lint**

把 cpal 0.18.2 用到的 API 表面**逐签名**抄成一个只做类型检查的仿真 crate
（来源：`cpal-0.18.2/src/traits.rs`、`src/timestamp.rs`、`src/lib.rs`、`src/device_description.rs`），
在副本里用 `[patch.crates-io] cpal = { path = ... }` 换掉，然后跑

```bash
cargo clippy -p yeban-engine --all-targets -- -D warnings   # 副本内，device feature 打开
```

于是**真实的** `device.rs` 与 `examples/measure_latency.rs`（含 cpal 的那一半）
被真正编译 + lint 了一遍。它当场抓到 **3 类必然/可能让 CI 变红的缺陷**：

1. `E0596`：移进 `FnMut` 回调的 `EngineRuntime` 没声明 `mut`（**真 cpal 下也必然红**）；
2. `clippy::collapsible_if`（edition 2024 的 let-chain，**真 cpal 下也必然红**）；
3. `clippy::drop_non_drop` 两处：`drop(keep_alive)` 在真 cpal 下**必然红**
   （`EngineKeepAlive` 无 `Drop` 实现）；`drop(stream)` 取决于 cpal 的 `Stream` 是否
   `needs_drop`（cpal 的 `src/platform/mod.rs` 里**没有** `impl Drop for Stream`，
   所以很可能也是红的）。修复方式与"是否实现 `Drop`"无关：改用**作用域**
   （`{ let stream = ...; }`）表达"回调线程先结束、快照锚后落地"，两处 `drop()` 全部删掉。

⚠ **这一层的边界（必须说清）**：仿真桩没有真后端，它只证明"**能编译、能被 lint**"，
**不证明**任何运行时行为、也不证明真 cpal 的编译能过。

**第二层：真二进制跑无设备路径**

在同一个副本里（`device` feature 打开）构建**真的** example 二进制并跑判据：

```bash
cargo build -p yeban-engine --example measure_latency
cargo test  -p yeban-engine --test latency_cli_contract     # 5 passed / 0 failed
```

5 条运行期判据全部在**真二进制**上通过；"有设备"那一条按设计**响亮 SKIP**
（仿真桩报告 0 个设备）。附带一个强证据：真二进制 `--force-no-device` 的 `BENCH` 行与本机
用纯模块渲染的行 **`diff` 为空（逐字节相同）** ⇒ §4.1 贴的那一行不是手抄的。

⚠ **这一层的边界**：设备枚举走的是仿真桩 ⇒ 它验证的是"**无设备/无回环**"这条路径的
端到端行为（参数解析 → Report → 判定 → 退出码 → 可读行），
**不是**"有真实声卡"那一条。后者仍然只有 CI / 有声卡的机器能给。

---

## 4. 工具的真实输出（样例）

### 4.1 无设备（托管 CI runner 的常态）——**真实的，本机渲染**

```text
MEASURES: nominal per-direction latency (device-reported buffer frames / sample rate); device-reported actual buffer frames after opening; callback arrival jitter (p50/p99/max) on this CPU
DOES-NOT-MEASURE: acoustic / DAC-ADC roundtrip. That needs a physical output->input loopback or a vendor API (CoreAudio kAudioDevicePropertyLatency, WASAPI IAudioClient::GetStreamLatency, ALSA snd_pcm_delay). cpal 0.18.2 exposes none of these.
NO-DEVICE: no audio device reported by this host: nothing was measured, this is NOT a pass and NOT 0 ms
NOTE: forced no-device path: no audio device reported by this host: nothing was measured, this is NOT a pass and NOT 0 ms
BENCH baseline=005 label=ci-runner verdict=no-device evidence=nominal-only devices=0 output_devices=0 input_devices=0 measured_roundtrip_ms=none target_ms=5.5000 nominal_out_ms=none nominal_in_ms=none nominal_io_sum_ms=none nominal_io_sum_is_roundtrip=false callbacks_out=none callbacks_in=none backend_errors_out=none backend_errors_in=none jitter_out_p50_ms=none jitter_out_p99_ms=none jitter_out_max_ms=none jitter_in_p50_ms=none jitter_in_p99_ms=none jitter_in_max_ms=none loopback=false driver_reported_extra_latency_ms=unknown
# stderr: measure_latency: verdict=no-device (NOT within-target)
# exit = 3
```

⚠ 这一段是**真实输出**（`Report::human_summary()` / `Report::bench_line()` 的真实字节），
但产生方式要如实说明：本机不允许编译 cpal，所以它是用纯模块 + 空设备列表渲染的 ——
也就是 example 的 `--force-no-device` 分支所走的**同一份**代码路径。
真实二进制走真实枚举的那一条只在 CI / 有声卡的机器上产生；
**无设备时两者逐字节相同**（`--force-no-device` 只是跳过枚举，让设备列表为空）。

### 4.2 有设备但**没有回环**（**形状示范，SYNTHETIC**）

下面的设备条目与数字是**人造的**，只用来展示输出形状 —— 本机没有声卡、也没有真实读数：

```text
NO-LOOPBACK: no loopback evidence: cpal 0.18.2 exposes no hardware roundtrip latency API, and this tool does not capture from a physical output->input loopback
DEVICE index=0 direction=output name="Synthetic Speakers" driver=synthetic interface=Built-in channels=2 sample_rate=48000 format=F32 buffer_min=32 buffer_max=1024 buffer_unknown=false buffer_frames_reported=64
NOMINAL direction=output buffer_frames=64 sample_rate=48000 latency_ms=1.3333 kind=nominal-not-roundtrip
CALLBACK direction=output callbacks=1500 backend_errors=0 nominal_period_ms=1.0000 jitter_p50_ms=0.0000 jitter_p99_ms=0.0200 jitter_max_ms=0.0200 jitter_mean_ms=0.0053 kind=cpu-scheduling-not-roundtrip
BENCH baseline=005 label=synthetic-with-device verdict=unmeasurable-without-loopback evidence=nominal-only devices=1 output_devices=1 input_devices=0 measured_roundtrip_ms=none target_ms=5.5000 nominal_out_ms=1.3333 nominal_in_ms=none nominal_io_sum_ms=none nominal_io_sum_is_roundtrip=false callbacks_out=1500 callbacks_in=none backend_errors_out=0 backend_errors_in=none jitter_out_p50_ms=0.0000 jitter_out_p99_ms=0.0200 jitter_out_max_ms=0.0200 jitter_in_p50_ms=none jitter_in_p99_ms=none jitter_in_max_ms=none loopback=false driver_reported_extra_latency_ms=unknown
# exit = 4
```

**本机没有真实音频设备读数**（Apple M2 开发机按纪律不编译 cpal）。
`BASELINE-005` 的**实测往返**在整条线上从未产生过一个数字 —— 这是事实，不是遗漏。

---

## 5. CI 上会发生什么

本线**没有**改 `.github/**` 与 `scripts/**`（集成者地盘）。在当前 CI（`ci.yml` / `gates-manual.yml`）下：

1. `cargo clippy --workspace --all-targets -- -D warnings` 会**真的编译**这个 example
   （cpal 在依赖图里）⇒ 本机"编译不了"的那一半在 CI 上被编译并做**契约判据**
   （`latency_cli_contract` 会跑真二进制）；
2. GitHub 托管 runner（`ubuntu-*` / `macos-*`）**没有音频设备** ⇒ 实际运行会命中
   **无设备路径**：`verdict=no-device`、退出码 3、`NO-DEVICE:` 文案；
   判据 `the_no_device_path_is_explicit_and_never_reports_zero_ms` 会**在真实无声卡环境里**
   验证这条路径 —— 这正是要的部署环境；
3. 因此 CI 得到的结论是「**本环境无法测量**」，**不是**"通过"。
   如果以后把它接进流水线，请**显式**读 `verdict=`/退出码，并禁止把它渲染成绿勾；
4. 想让 CI 日志步骤自己不红，可以加 `--allow-unmeasurable`；
   但**必须**同时grep `verdict=no-device`（否则就是把"没测到"洗成"通过"）。

---

## 6. 要在什么机器上、跑哪条命令，才**可能**判定达标（给人看的一句话）

> **今天没有任何一条命令能判定 `BASELINE-005` 达标。**
> 工具现在能跑到的**上限**是 `verdict=unmeasurable-without-loopback`。

要真正判定，需要**同时**满足两件事，二者本线都**没有**：

1. **一台规范指定的参考机 + 有真实声卡**（`docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md`
   §5.1：Apple M2 Pro 12 核 16 GB / AMD Ryzen 7 7840HS 8 核 16 线程）；
   在该机上以 64 帧缓冲跑
   `cargo run --release -p yeban-engine --example measure_latency -- --label <machine> --frames 64`，
   拿到**标称**与**抖动**；
2. **一条物理或虚拟回环**（输出 → 输入；例如 `BlackHole` / `Loopback` 虚拟设备），
   以及一段**尚未实现**的回环采集比对（发送已知脉冲 → 采集回来 → 算样本差 ⇒ 真实往返），
   或改走厂商 API（`kAudioDevicePropertyLatency` / `GetStreamLatency` / `snd_pcm_delay`）。

**只有 1+2 同时具备时，`measured_roundtrip_ms=` 才可能出现一个数字**，
`verdict` 才可能变成 `within-target` / `over-target`。在那之前，
任何"5.5 ms 达标"的说法都是拿**标称**冒充**实测**。

---

## 7. 未实现项、边界与 needs

### 7.1 明确**没有**做的

1. **回环采集比对**（第 2 步）未实现 —— 这是 `BASELINE-005` 的本体，也是本线**故意**划出的边界；
2. **厂商 API 探测**未实现（需要新的 FFI 依赖 ⇒ 依赖图裁决）；
3. **驱动层额外时延**（ARCH §3.5 的 1.84 ms）拿不到，行里恒为 `unknown`；
4. **`BASELINE-005` 的门禁状态行没有改** —— `docs/ledger/gate-status.md` 是集成者地盘，
   本线不得编辑。该行**现在仍然是 `PENDING`**，且**这是正确的**（门禁本体没被测过）；
5. **没有把 `BASELINE-005` 加进 `src/lib.rs` 的 `IMPLEMENTED_SPEC_IDS`** —— 加了就是越权宣称：
   本 crate 实现的是**工具**，不是那条时延指标；
6. **没有接进 CI 流水线**（`.github/**` 是集成者地盘）；
7. 引擎的**实时线程优先级**仍是既有缺口（`ROAD-M2-001`）⇒ 抖动的绝对值在 macOS/ALSA 上
   可能被人为放大（CoreAudio 由内核约束，Linux-ALSA 无开关）。抖动**不能**当硬件结论用。

### 7.2 已知的解释边界（读数字前必看）

- `DeviceTrait::buffer_size()` 的文档明确写着「**不保证**每次回调都正好这么多帧」⇒
  "标称周期"是**名义**周期；用回调时间戳对它求偏差，会把"帧数不齐"也算成抖动 ——
  这是刻意的（它就是 CPU 侧调度抖动的定义），但**不要**把它解释成硬件时延；
- 抖动是**CPU 侧**量：不含驱动流水线、不含 DAC/ADC、不含回环；
- `nominal_*_ms` 用的是**后端回读**的帧数；后端不报时退到请求值并在 `NOTE` 里说明；
  两者都没有 ⇒ `none`（**不猜**）；
- 托管 runner 上的任何读数（如果有）只是数量级参考，不是达标依据；
- 工具的回调本身也守**红线 7**：预分配的 `Box<[AtomicU64]>` + 原子写，
  回调内零分配 / 零锁 / 零阻塞 I/O（在音频线程上分配会自己制造被测的抖动）。

### 7.3 needs（需要谁做什么）

| # | 需要 | 谁 | 备注 |
| :---: | :--- | :--- | :--- |
| 1 | 把本工具接进一个**手动档** CI 作业（`bench` 或新的 `latency` 档），并**显式**按 `verdict` 记成"本环境无法测量"而不是通过 | 集成者（`.github/**` / `scripts/**`） | 本线无权改；建议用 `--allow-unmeasurable` + 断言 `verdict=no-device` |
| 2 | 裁决"是否为厂商时延 API 引入新的 FFI 依赖" | 人类 / ADR | 这是 `BASELINE-005` 从"标称+抖动"走向"实测"的两条路之一 |
| 3 | 提供**带声卡 + 回环**的参考机并执行回环比对（或授权虚拟回环设备方案） | 人类 | 另一条路；没有它这条门禁只能永远 PENDING |
| 4 | 在 `docs/ledger/gate-status.md` 的 `BASELINE-005` 行里补一句"工具已就绪（标称+抖动），门禁本体仍需硬件回环" | 集成者 | 本线不得编辑该文件 |
| 5 | 若要让 CI 在**有设备**的机器上跑（self-hosted runner），需要确认许可与设备独占策略 | 人类 | 共享模式会被 OS 混音器影响，读数口径不同 |

---

## 8. 修改文件清单（本线）

| 文件 | 性质 |
| :--- | :--- |
| `crates/yeban-engine/src/latency.rs` | **新增**：零 cpal 依赖的纯计算 + 判定 + 机器可读行（15 条判据） |
| `crates/yeban-engine/examples/measure_latency.rs` | **新增**：cpal 测量机器（枚举 + 真流 + 抖动 + 可读行 + 退出码） |
| `crates/yeban-engine/tests/latency_cli_contract.rs` | **新增**：跑真二进制的运行期契约判据（5 条） |
| `crates/yeban-engine/src/lib.rs` | **改动**：注册 `pub mod latency;` + 模块地图一行 + 一段边界说明（明确"这不是门禁本身"） |
| `docs/ledger/audio-latency-notes.md` | **新增**：本文件 |

**没有**改动：根 `Cargo.toml` / `Cargo.lock`（零新增依赖）、`.github/**`、`scripts/**`、`deny.toml`、
`docs/DEVELOPMENT_LEDGER.md`、`docs/adr/**`、`docs/YEBAN_*.md`、`schemas/**`、
其它 `crates/**`、`spikes/**`、法务文件、`README*.md`。
