# `BASELINE-005`（音频硬件往返时延 ≤ 5.5 ms）台账：**机器那一半**、能力矩阵与诚实边界

- **台账类型**：能力矩阵 / 判据清单 / 实测证据 / 未决项（**不是规范**）
- **记录时刻**：2026 年，第 1 轮（本机 Apple M2 + 受限 `workspace-write` 沙箱）
- **工作线**：`line/audio-latency`（worktree `yeban/.worktrees/audio-latency`，main `2f79c09`）
- **所有者目录**：`crates/yeban-engine/**`（本台账是唯一新增的文档文件）
- **落地范围**：新增 `crates/yeban-engine/src/latency.rs`（**零 cpal 依赖**的纯计算与判定）、
  `crates/yeban-engine/examples/measure_latency.rs`（cpal 测量机器）、
  `crates/yeban-engine/tests/latency_cli_contract.rs`（运行期契约判据）；
  `src/lib.rs` 只加了模块注册与边界说明；`src/lib.rs` / `src/snapshot.rs` 另有**纯措辞**的
  过时口径更正（见 §7.4）
- **规范来源**：`docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` §5.2 的 `[BASELINE-005]` 原文、
  `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §3.5 的 `[ARCH-PDC-002]` 延迟预算分解表、
  `ARCH-PDC-001`、`ARCH-RT-001`、`ARCH-DET-001`、`AGENTS.md §2 红线 7`
- **零新增依赖**：cpal 0.18.2 已在依赖图里（`Cargo.lock` 钉死）；
  本线**没有**改根 `Cargo.toml` / `Cargo.lock` / `.github/**` / `scripts/**` / `docs/DEVELOPMENT_LEDGER.md`

> 本文件回答六个问题：**到底哪一半测得了、哪一半测不了**、
> **标称 / 驱动侧 / 实测三者的区别**、**判据是怎么变红的**、**CI 上会发生什么**、
> **还要在什么机器上做什么才可能判达标**、**哪些东西明确没做**。

---

## 0. 一句话结论（先读这段，别先读数字）

**`BASELINE-005` 依然是 `PENDING`，本线没有、也不可能把它变成绿。**

本线交付的是"**机器**"：能把三样东西变成可复现的读数 ——
① **设备报告的标称时延**；② **主机报告的驱动侧时延**（输出 `playback − callback`、
输入 `callback − capture`）；③ **回调调度抖动**；
并且把"**测不到**"与"**达标**"在机械层面分开：没有**回环证据**时，
`verdict` **永远不可能是** `within-target`（由判据钉死，见 §3.4 注入 B/D）。

⚠ **一次自我更正（留痕）**：本线**第一版**曾宣称"cpal 0.18.2 不暴露任何硬件时延信息"。
核对 `cpal-0.18.2/src/` 源码后确认**这个说法是错的**：每个回调携带的 `playback` / `capture`
时刻**就是**主机用厂商 API 算出来的时延（§1.1）。已更正措辞**并实现**了这条测量路径。
**真正的声学/DAC-ADC 往返时延仍然一次都没有被测过** —— 这一点第一版说对了，现在也没变。

---

## 1. 能力矩阵：能测 / 不能测（逐项）

| # | 量 | 能测？ | 事实来源 | 进入机器可读行？ |
| :---: | :--- | :---: | :--- | :--- |
| 1 | 设备名 / 驱动名 / 接口类型（`Built-in`/`USB`/…） | ✅ | cpal `DeviceTrait::description()` | `DEVICE` 行（人读） |
| 2 | 默认输入/输出配置：通道数、采样率、`f32` 格式 | ✅ | `default_input_config()` / `default_output_config()` | `DEVICE` 行（人读） |
| 3 | 设备报告的缓冲区间 `min..=max`，或 `Unknown` | ✅ | `supported_*_configs()` → `SupportedBufferSize` | `DEVICE` 行（人读） |
| 4 | 协商结果：`BufferSize::Fixed(n)` 还是 `Default` | ✅ | `device::negotiate()`（与本 crate 真实路径**同一份**纯函数） | 间接（标称行） |
| 5 | 后端报告的实际缓冲帧数 | ✅ | `DeviceTrait::buffer_size()`（打开流之后回读） | `buffer_frames_reported=` |
| 6 | **标称时延**（输入、输出**分别**）= 帧数 ÷ 采样率 | ✅ | 第 5 项（拿不到就退到请求的 `Fixed(n)`；都没有 ⇒ `none`） | `nominal_out_ms=` / `nominal_in_ms=` |
| 7 | **主机报告的驱动侧输出时延** = `playback − callback` | ✅ | 每个**输出**回调的 `OutputStreamTimestamp::playback`；主机折入了设备缓冲 + 设备时延 + 安全偏移（见 §1.1） | `driver_out_latency_p50/p99/max_ms` |
| 8 | **主机报告的驱动侧输入时延** = `callback − capture` | ✅ | 每个**输入**回调的 `InputStreamTimestamp::capture` | `driver_in_latency_p50/p99/max_ms` |
| 9 | **回调调度抖动**（CPU 侧）p50 / p99 / max / mean | ✅ | 真实跑流的回调时间戳间隔 vs 标称周期 | `jitter_*_p50/p99/max_ms` |
| 10 | 后端错误回调次数（XRun/设备忙/失效） | ✅ | 错误回调里的原子自增 | `backend_errors_*=` |
| 11 | **声学 / DAC-ADC 往返时延**（`BASELINE-005` 的本体） | ❌ | 需要**物理回环**（输出接输入）+ 采集比对 | `measured_roundtrip_ms=none` |
| 12 | ARCH §3.5 里"内部 DSP 拓扑调度 1.00 ms"那一格 | ❌ | 属引擎渲染路径的**离线**测量，不在本工具范围 | — |
| 13 | 驱动侧时延的**输入+输出合计** | ⛔ **故意不给** | 第 7+8 项可以相加，但**故意不输出**该字段 | 无（见 §1.2） |

### 1.1 第 7/8 项是怎么来的 —— 已按 crate 源码核对；**并附一次更正留痕**

**先说我第一版说错在哪**：我写的是"cpal 不暴露任何硬件时延查询 API，全 crate 里 `latency`
一词只出现在错误文案与文档注释里"。前半句（没有**显式的**查询 API）成立，后半句**是错的**：

```bash
# 核验方法（本机可用；CARGO_HOME 在受限沙箱里由 scripts/dev/local-env.sh 指到工作区）
CPAL_SRC=$(find "${CARGO_HOME:-$HOME/.cargo}/registry/src" -maxdepth 2 -type d -name 'cpal-0.18.2' | head -1)
grep -rn "OutputStreamTimestamp\|playback" "$CPAL_SRC/src/host/" | grep -v '://'
```

事实（cpal 0.18.2，本线逐文件读过）：

| 主机 | `playback`（输出）怎么算的 | 因此在算谁的账 |
| :--- | :--- | :--- |
| **CoreAudio** (macOS) | `callback + (device_buffer_frames + kAudioDevicePropertyLatency + kAudioDevicePropertySafetyOffset) / rate` | 设备缓冲 + **规范点名的 `kAudioDevicePropertyLatency`** + 安全偏移 |
| **WASAPI** (Windows) | `callback + buffered_frames + stream.stream_latency` | 已提交未消费帧 + **`IAudioClient::GetStreamLatency`** |
| **ALSA** (Linux) | `callback + delay_frames`（PCM 状态里的 delay） | **`snd_pcm_delay` 一族** |
| **PulseAudio** | `callback + (elapsed + 估算的流时延)` | 流时延**估计值**（注意：是估计） |
| PipeWire / JACK / ASIO / AAudio | 各自的流时间 + 缓冲 | 厂商/服务器报的值 |

⇒ 也就是说：规范 §5.2 为 macOS/Windows/Linux 点名的三个厂商查询，
**通过 `playback`（输出）与 `capture`（输入）已经能被 cpal 的公共 API 拿到**，
**不需要任何新依赖**。这是本线第一版漏掉的一整块能力。

**但仍然不等于"实测往返"**，原因有三，且每一条都写进了工具输出：

1. 它是**主机/驱动的预测**，不是测出来的声学量（声学路径、DAC/ADC 时钟域都不在里面）；
2. 有些主机**压根不报**（`playback == callback`）⇒ 工具输出字面量
   `driver_*_latency_p99_ms=unreported-or-zero`，并明确写"**这不是 0 ms 时延**"；
3. 它是**方向分开**的，且**故意不给合计**（见下）。

### 1.2 一个必须说清的口径陷阱：**任何合计都不能当往返**

ARCH §3.5 的预算表是这么拆的（@48 kHz / 64 帧）：

| 阶段 | 时延 | 本工具能提供吗 |
| :--- | ---: | :--- |
| 物理声卡输入缓冲（64 帧） | 1.33 ms | ✅ 标称 + ✅ 驱动侧（`callback − capture`） |
| 内部 DSP 拓扑调度 | 1.00 ms | ❌（不在本工具范围） |
| 物理声卡输出缓冲（64 帧） | 1.33 ms | ✅ 标称 + ✅ 驱动侧（`playback − callback`） |
| 系统驱动与总线余量 | 1.84 ms | ⚠ **取决于主机**：CoreAudio/WASAPI/ALSA 的那部分**已经折进**第 7/8 项；PulseAudio 是估计值 |
| **端到端回路总时延** | **5.50 ms** | ❌ **测不了**（缺声学往返） |

标称 1.33 + 1.33 = 2.67 ms、驱动侧 3.0 + 1.5 = 4.5 ms —— **两个都"看起来"低于 5.5 ms**。
正因如此，机器可读行里：

- **没有** `roundtrip_ms` 字段；实测往返只出现在 `measured_roundtrip_ms=`（本工具恒为 `none`）；
- `nominal_io_sum_ms` 旁边永远跟着 `nominal_io_sum_is_roundtrip=false`；
- **驱动侧根本没有合计字段**（只有 `driver_io_sum_is_roundtrip=false` 作声明）；
- 而且 `verdict` **不会**因为其中任何一个数字变成 `within-target`。

**谁把这些合计当成往返时延去宣布达标，谁就制造了一个假绿** —— 注入 B/D 就是这条的机械证明。

---

## 2. 标称 vs 驱动侧 vs 实测：这不是"精度差别"，是**性质差别**

| | 标称时延 | 驱动侧时延（本工具新增） | 实测往返时延 |
| :--- | :--- | :--- | :--- |
| 定义 | 缓冲帧数 ÷ 采样率 | 主机预测的"数据真正到达/离开设备"的时刻差 | 样本从 DAC 出去、经回环、再被 ADC 采回来 |
| 谁算的 | 我们（除法） | **主机/驱动** | 测量者（回环比对） |
| 含设备时延 / 安全偏移？ | ❌ | ✅（CoreAudio/WASAPI/ALSA；PulseAudio 为估计） | ✅ |
| 含声学路径 / 时钟域？ | ❌ | ❌ | ✅ |
| 方向 | 分输入/输出 | 分输入/输出（**故意不合计**） | 单一往返数 |
| 能否判定 `BASELINE-005`？ | **不能** | **不能**（除非人类按 ADR 裁决"原生 API 口径"即满足规范） | 能（必须有回环证据） |

工具输出现场就是三者分开的（`NOMINAL` / `DRIVER-LATENCY` / `measured_roundtrip_ms=none`）。

---

## 3. 工具与判据：清单、命令、本机 vs CI

### 3.1 三个文件

| 文件 | 依赖 cpal？ | 本机 (M2) 能真跑？ | 作用 |
| :--- | :---: | :---: | :--- |
| `crates/yeban-engine/src/latency.rs` | **否**（零依赖） | ✅ `rustc --test` 真跑 | 标称换算、驱动侧分布统计、抖动统计（最近秩百分位）、机器可读行 + 严格解析、判定与退出码 |
| `crates/yeban-engine/examples/measure_latency.rs` | **是** | ⚠ 不能对**真 cpal** 编译；用 API 仿真桩做了完整类型检查 + 真二进制跑**两条**路径（§3.5） | 枚举设备、开真流、跑真实回调、打印可读行 |
| `crates/yeban-engine/tests/latency_cli_contract.rs` | 否（只 `Command` 起子进程） | ✅（对 cpal 那半**响亮 SKIP**） | 跑**真的二进制**并对输出做契约断言 |

**为什么把纯计算抽出来**：本机纪律不允许编译 cpal，所以"判定逻辑"与"声卡 I/O"必须能分开验证。
抽出来的那半在本机是**真跑**的（不是"跳过所以绿"）。

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

### 3.3 判据清单（**6 条主题 / 18 条纯计算判据 + 5 条运行期判据**）

| 编号 | 判据（测试名） | 主题 | 本机真跑 | CI |
| :---: | :--- | :--- | :---: | :---: |
| ① | `nominal_latency_conversion_is_exact` | 标称换算正确（64/48k=1.3333、128/48k=2.6667、128/44.1k=2.9025；采样率 0 ⇒ `None`） | ✅ | ✅ |
| ② | `jitter_percentiles_are_exact_on_handmade_timestamps` | 人造时间戳 ⇒ 钉死 p50/p99/max/mean；**且 `p99 > mean`** | ✅ | ✅ |
| ② | `perfectly_regular_callbacks_have_zero_jitter` | 规律流 ⇒ 抖动恒为 0（工具不造噪声） | ✅ | ✅ |
| ② | `empty_or_degenerate_samples_never_invent_a_number` | "没采到" ≠ "抖动 0" | ✅ | ✅ |
| ② | `driver_latency_distribution_is_exact_on_handmade_values` | **新增**：驱动侧分布的最近秩口径（2.5/2.5/2.5/2.9 ⇒ p50=2.5、p99=2.9、mean=2.6）；空 ⇒ `None` | ✅ | ✅ |
| ③ | `the_no_device_path_reports_no_device_and_never_zero_ms` | 无设备 ⇒ `verdict=no-device`、退出码 3、文案含 `NOT a pass and NOT 0 ms`、行里没有 `0.0000` | ✅ | ✅ |
| ③ | `the_no_device_path_is_explicit_and_never_reports_zero_ms`（运行期） | 跑**真二进制** + `--force-no-device` | SKIP(loud) | ✅ |
| ④ | `device_present_reports_non_negative_nominal_and_jitter` | 有设备 ⇒ 标称 / 抖动 / **驱动侧**都打印且非负、`p99 ≥ p50`、`max ≥ p99` | ✅ | ✅ |
| ④ | `a_machine_with_devices_reports_non_negative_nominals_and_jitter`（运行期） | 真二进制；有设备就跑，没设备就**响亮**说明"本环境无法测量" | SKIP(loud) | ✅ |
| ⑤ | `bench_line_round_trips_through_the_strict_parser` | 写出 → 严格解析 → 逐字段对账（含驱动侧字段与来源口径） | ✅ | ✅ |
| ⑤ | `the_parser_rejects_malformed_lines` | 解析器**有判别力**：错前缀 / `baseline=002` / 游离 token / 缺字段 / 非数字 ⇒ 全拒 | ✅ | ✅ |
| ⑤ | `unknown_buffer_frames_produce_none_instead_of_a_guess` | 帧数未知 ⇒ `none`，不猜 | ✅ | ✅ |
| ⑤ | `exit_codes_separate_unmeasured_from_pass` | 3/4 与 0 分开；便利开关不制造达标 | ✅ | ✅ |
| ⑤ | `the_convenience_flag_never_changes_the_machine_readable_verdict` | 开关不改判定、不改行 | ✅ | ✅ |
| ⑤ | `labels_are_sanitized_so_the_line_stays_parseable` | 标签规范化（空白/`=`/非 ASCII） | ✅ | ✅ |
| ⑤ | `usage_errors_exit_two_and_help_exits_zero`（运行期） | 用法错误 ⇒ 2；`--help` ⇒ 0；都不输出读数 | SKIP(loud) | ✅ |
| ⑤ | `unreported_driver_latency_is_not_zero_and_never_a_pass` | **新增**：主机不报（全 0 样本）⇒ 字面 `unreported-or-zero`，**绝不写 `0.0000`**，人读行必须说"这不是 0 ms 时延" | ✅ | ✅ |
| ⑥ | `nominal_values_can_never_satisfy_the_baseline` | **没有回环证据 ⇒ 永不达标**（连 0.001 ms 的"标称"也不行） | ✅ | ✅ |
| ⑥ | `loopback_evidence_is_the_only_route_to_a_pass` | 只有回环证据能到达标/超目标，且 5.5 ms 边界精确（`≤`） | ✅ | ✅ |
| ⑥ | `a_tiny_driver_latency_still_cannot_satisfy_the_baseline` | **新增**：驱动侧读数（3 ns）也**不许**把判定变成达标；且行里**不许出现驱动侧合计字段** | ✅ | ✅ |
| ⑥ | `the_tool_never_claims_a_pass_and_always_prints_its_boundary`（运行期） | 真二进制：任何调用形态下 `verdict ≠ within-target`、`measured_roundtrip_ms=none`、`loopback=false`、驱动侧字段自洽，且必打印 `MEASURES` / `DOES-NOT-MEASURE` | SKIP(loud) | ✅ |
| ⑥ | `target_and_baseline_id_match_the_specification` | 5.5 / `BASELINE-005` / `005` 字面量被钉住 | ✅ | ✅ |

**本机实测（M2）**：
- `rustc --edition 2024 --test -D warnings` → **18 passed; 0 failed**；
- `cargo test -p yeban-engine --no-default-features`（仓库内轻量变体）→ 库 **116 passed**
  （含这 18 条）+ `latency_cli_contract` **5 passed**（4 条**响亮 SKIP**：本机 variant 不带 cpal）；
- `/tmp` 副本 + cpal API 仿真桩（`device` feature 打开，见 §3.5）→
  `cargo clippy -p yeban-engine --all-targets -- -D warnings` **绿**；
  `latency_cli_contract` **5 passed / 0 SKIP**（仿真设备把"有设备"那条路径也真跑了起来）。

### 3.4 注入记录（**5 条注入，全部"注入 → 变红 → 还原"**）

还原后 `diff` 与备份**逐字节相同**
（`sha256 = 0be5ecf4cf31f8756995fa34f7c1bd30fa2d2a4b2baadf7fa87fe7f69663ee9a`），
且 18 条判据重新全绿。

| 注入 | 做了什么 | 变红的判据 | 对工具输出的影响 |
| :---: | :--- | :--- | :--- |
| **A** | 把"没有设备"当成"0 ms ⇒ 达标"（`verdict_for` 的 `device_count == 0` 分支返回 `WithinTarget`） | `the_no_device_path_...`（17 passed / 1 failed） | `verdict=no-device` → **`within-target`**，退出码 `3` → **`0`**（正是"无设备 runner 记成通过"的假绿） |
| **B** | 把标称值当成实测往返（`verdict_for` 忽略 `evidence` + `measured_roundtrip_ms` 回落到标称） | `nominal_values_can_never_satisfy_the_baseline`、`bench_line_round_trips_...`（16/2） | `measured_roundtrip_ms=none` → **`1.3333`** |
| **C** | 抖动统计用均值冒充 p99 | `jitter_percentiles_are_exact_on_handmade_timestamps`（17/1） | p99 变成 mean（右偏下明显偏小） |
| **D** | **把驱动侧时延当成回环证据、并加总成"往返"** | `a_tiny_driver_latency_still_cannot_satisfy_the_baseline`、`bench_line_round_trips_...`（16/2） | 判定从 `UnmeasurableWithoutLoopback` → **`WithinTarget`**（`left: WithinTarget / right: UnmeasurableWithoutLoopback`）—— 这是**新能力**引入的最危险假绿，判据⑥当场挡住 |
| **E** | 把"主机不报"（全 0 样本）当成"真的是 0 ms" | `unreported_driver_latency_is_not_zero_and_never_a_pass`（17/1） | 字段从 `unreported-or-zero` 变成数字 ⇒ 无信息被当成零时延 |

### 3.5 含 cpal 的那一半，本机是怎么验的（以及它的**边界**）

本机按纪律不编译 cpal，但"本机不编译"**不等于**"不用验"。做了两层真跑，都在 `/tmp` 的一次性
副本里进行 —— **仓库里的 `Cargo.toml` / `Cargo.lock` 没有被改过**：

**第一层：API 仿真桩的类型检查 / lint**

把 cpal 0.18.2 用到的 API 表面**逐签名**抄成一个只做类型检查的仿真 crate
（来源：`traits.rs`、`timestamp.rs`、`lib.rs`、`device_description.rs`），
在副本里用 `[patch.crates-io] cpal = { path = ... }` 换掉，跑

```bash
cargo clippy -p yeban-engine --all-targets -- -D warnings   # 副本内，device feature 打开
```

于是**真实的** `device.rs` 与 `examples/measure_latency.rs` 被真正编译 + lint 了一遍。
它当场抓到 **4 类必然/可能让 CI 变红的缺陷**：

1. `E0596`：移进 `FnMut` 回调的 `EngineRuntime` 没声明 `mut`（**真 cpal 下也必然红**）；
2. `clippy::collapsible_if`（edition 2024 的 let-chain，**必然红**）；
3. `clippy::drop_non_drop` 两处：`drop(keep_alive)` 在真 cpal 下**必然红**；
   `drop(stream)` 取决于 `Stream` 是否 `needs_drop`（cpal 的 `src/platform/mod.rs` 里
   **没有** `impl Drop for Stream`，很可能也红）。修法与"是否实现 `Drop`"无关：
   改用**作用域**表达"回调线程先结束、快照锚后落地"，两处 `drop()` 全删；
4. `E0308`：判据里 `bench_line(run)` 传错了参数（这是**判据自己**的 bug，
   说明这一层对测试代码也有效）。

⚠ **边界**：仿真桩没有真后端，只证明"**能编译、能被 lint**"，**不证明**任何运行时行为，
也不证明真 cpal 的编译能过。

**第二层：真二进制跑两条路径**

在同一个副本里（`device` feature 打开）构建**真的** example 二进制：

```bash
cargo build -p yeban-engine --example measure_latency
cargo test  -p yeban-engine --test latency_cli_contract     # 5 passed / 0 failed / 0 SKIP
```

仿真桩后来**升级成会真的产设备、真的按 ~1 ms 周期回调**（输出 `playback − callback = 3 ms`，
输入 `callback − capture = 1.5 ms`），于是：

- "无设备"判据 ③ 与"有设备"判据 ④ **都在本机真跑**（不再有 SKIP）；
- 工具输出里 `DRIVER-LATENCY ... p50_ms=3.0000` / `1.5000` 与仿真桩的设定**精确吻合**
  ⇒ 驱动侧时延的**接线**（`playback − callback` / `callback − capture` → 分布 → 可读行）
  是被端到端验证过的；
- 真二进制 `--force-no-device` 的 `BENCH` 行与本机用纯模块渲染的行 **`diff` 为空**。

⚠ **边界**：这些数字是**仿真桩按构造给的**，不是任何真实设备的读数；
**真实设备读数必须由有声卡的机器（CI 或参考机）给出** —— 本线一次都没有产生过真实的
声学往返数字。

---

## 4. 工具的真实输出（样例）

### 4.1 无设备（托管 CI runner 的常态）——**真二进制的真实字节**

```text
MEASURES: nominal per-direction latency (device-reported buffer frames / sample rate); device-reported actual buffer frames after opening; host-reported driver-side latency (output playback-minus-callback, input callback-minus-capture) as p50/p99/max; callback arrival jitter (p50/p99/max) on this CPU
DOES-NOT-MEASURE: the acoustic / DAC-ADC roundtrip itself. The playback/capture instants are the HOST's own prediction (CoreAudio folds in kAudioDevicePropertyLatency + safety offset, WASAPI IAudioClient::GetStreamLatency, ALSA the PCM delay, PulseAudio an estimate) - they are not a measured loop. A real roundtrip needs a physical output->input loopback (or an ADR ruling that the native-API figures count for BASELINE-005).
NO-DEVICE: no audio device reported by this host: nothing was measured, this is NOT a pass and NOT 0 ms
NOTE: forced no-device path: no audio device reported by this host: nothing was measured, this is NOT a pass and NOT 0 ms

BENCH baseline=005 label=ci-runner verdict=no-device evidence=nominal-only devices=0 output_devices=0 input_devices=0 measured_roundtrip_ms=none target_ms=5.5000 nominal_out_ms=none nominal_in_ms=none nominal_io_sum_ms=none nominal_io_sum_is_roundtrip=false callbacks_out=none callbacks_in=none backend_errors_out=none backend_errors_in=none jitter_out_p50_ms=none jitter_out_p99_ms=none jitter_out_max_ms=none jitter_in_p50_ms=none jitter_in_p99_ms=none jitter_in_max_ms=none driver_out_latency_source=playback_minus_callback driver_out_latency_reported=none driver_out_latency_p50_ms=none driver_out_latency_p99_ms=none driver_out_latency_max_ms=none driver_in_latency_source=callback_minus_capture driver_in_latency_reported=none driver_in_latency_p50_ms=none driver_in_latency_p99_ms=none driver_in_latency_max_ms=none driver_io_sum_is_roundtrip=false loopback=false
# stderr: measure_latency: verdict=no-device (NOT within-target)
# exit = 3
```

### 4.2 有设备（真实代码路径 + **仿真设备**；数字是桩按构造给的）

⚠ 下面**不是**任何真实设备的读数：设备与 3.0 / 1.5 ms 都由本机的 cpal API 仿真桩提供，
用来证明**接线正确**与**输出形状**。真实的设备读数必须由有声卡的机器给出。

```text
NO-LOOPBACK: no loopback evidence: cpal 0.18.2 has no explicit latency query API, and although each callback carries a host-computed playback/capture instant that we DO report, that is a driver-side prediction, not an acoustic DAC-ADC roundtrip (which needs a physical output->input loopback)
DEVICE index=0 direction=output name="Fake Speakers" driver=unknown interface=Unknown channels=2 sample_rate=48000 format=F32 buffer_min=32 buffer_max=1024 buffer_unknown=false buffer_frames_reported=64
DEVICE index=1 direction=input name="Fake Microphone" driver=unknown interface=Unknown channels=2 sample_rate=48000 format=F32 buffer_min=32 buffer_max=1024 buffer_unknown=false buffer_frames_reported=64
NOMINAL direction=output buffer_frames=64 sample_rate=48000 latency_ms=1.3333 kind=nominal-not-roundtrip
NOMINAL direction=input buffer_frames=64 sample_rate=48000 latency_ms=1.3333 kind=nominal-not-roundtrip
CALLBACK direction=output callbacks=310 backend_errors=0 nominal_period_ms=1.3333 jitter_p50_ms=0.3333 jitter_p99_ms=0.3333 jitter_max_ms=0.3333 jitter_mean_ms=0.3333 kind=cpu-scheduling-not-roundtrip
DRIVER-LATENCY direction=output source=playback_minus_callback reported=true samples=310 p50_ms=3.0000 p99_ms=3.0000 max_ms=3.0000 mean_ms=3.0000 kind=host-reported-driver-side-not-acoustic
CALLBACK direction=input callbacks=319 backend_errors=0 nominal_period_ms=1.3333 jitter_p50_ms=0.3333 jitter_p99_ms=0.3333 jitter_max_ms=0.3333 jitter_mean_ms=0.3333 kind=cpu-scheduling-not-roundtrip
DRIVER-LATENCY direction=input source=callback_minus_capture reported=true samples=319 p50_ms=1.5000 p99_ms=1.5000 max_ms=1.5000 mean_ms=1.5000 kind=host-reported-driver-side-not-acoustic

BENCH baseline=005 label=fake-device verdict=unmeasurable-without-loopback evidence=nominal-only devices=2 output_devices=1 input_devices=1 measured_roundtrip_ms=none target_ms=5.5000 nominal_out_ms=1.3333 nominal_in_ms=1.3333 nominal_io_sum_ms=2.6667 nominal_io_sum_is_roundtrip=false callbacks_out=310 callbacks_in=319 backend_errors_out=0 backend_errors_in=0 jitter_out_p50_ms=0.3333 jitter_out_p99_ms=0.3333 jitter_out_max_ms=0.3333 jitter_in_p50_ms=0.3333 jitter_in_p99_ms=0.3333 jitter_in_max_ms=0.3333 driver_out_latency_source=playback_minus_callback driver_out_latency_reported=true driver_out_latency_p50_ms=3.0000 driver_out_latency_p99_ms=3.0000 driver_out_latency_max_ms=3.0000 driver_in_latency_source=callback_minus_capture driver_in_latency_reported=true driver_in_latency_p50_ms=1.5000 driver_in_latency_p99_ms=1.5000 driver_in_latency_max_ms=1.5000 driver_io_sum_is_roundtrip=false loopback=false
# exit = 4  （注意：驱动侧 3.0 + 1.5 = 4.5 ms "看起来达标"，但 verdict 仍是不可判定）
```

（`jitter_* = 0.3333` 是仿真桩按固定 1 ms 周期回调、而标称周期是 1.3333 ms 的构造结果，
不是真实抖动。）

**重要**：**没有任何一台真实设备的读数**。`measured_roundtrip_ms` 在整条线上从未是一个数字。

---

## 5. CI 上会发生什么

本线**没有**改 `.github/**` 与 `scripts/**`（集成者地盘）。在当前 CI（`ci.yml` / `gates-manual.yml`）下：

1. `cargo clippy --workspace --all-targets -- -D warnings` 会**真的编译**这个 example
   （cpal 在依赖图里）⇒ 本机"编译不了"的那一半在 CI 上被编译并做**契约判据**；
2. GitHub 托管 runner（`ubuntu-*` / `macos-*`）**没有音频设备** ⇒ 实际运行会命中
   **无设备路径**：`verdict=no-device`、退出码 3、`NO-DEVICE:` 文案；
   判据 `the_no_device_path_is_explicit_and_never_reports_zero_ms` 会**在真实无声卡环境里**
   验证这条路径 —— 这正是要的部署环境；
3. 因此 CI 得到的结论是「**本环境无法测量**」，**不是**"通过"。
   若以后接进流水线，请**显式**读 `verdict=`/退出码，并禁止把它渲染成绿勾；
4. 想让日志步骤自己不红，可以加 `--allow-unmeasurable`；
   但**必须**同时 grep `verdict=no-device`（否则就是把"没测到"洗成"通过"）。

---

## 6. 要在什么机器上、跑哪条命令，才**可能**判定达标（给人看的一句话）

> **今天没有任何一条命令能宣布 `BASELINE-005` 达标。** 工具能跑到的上限是
> `verdict=unmeasurable-without-loopback`。

要真正判定，必须先让**人类**在两条路里裁决一条（本线不擅自裁决）：

- **路 1（原生 API 口径）**：承认"主机报告的驱动侧时延"（本工具已经能测：输出
  `playback − callback`、输入 `callback − capture`）**满足规范 §5.2 的"原生系统 API 实测"**
  措辞。若走这条路，则在一台**有声卡的参考机**上跑：
  `cargo run --release -p yeban-engine --example measure_latency -- --label <machine> --frames 64`
  —— 但那仍然只是**驱动侧**数，ARCH §3.5 的"内部 DSP 1.00 ms"不在此列，需要单独对账。
- **路 2（硬件回环）**：接一条**物理回环**（输出 → 输入）或 `BlackHole`/`Loopback` 这类
  虚拟回环设备，并实现**尚未实现**的回环采集比对（发已知脉冲 → 采回来 → 算样本差）。
  只有这条路能给出真正的 `measured_roundtrip_ms`。

参考硬件（规范 §5.1）：Apple M2 Pro 12 核 16 GB / AMD Ryzen 7 7840HS 8 核 16 线程。
**在任何一条路被裁决并执行之前，`BASELINE-005` 只能保持 `PENDING`。**

---

## 7. 未实现项、边界与 needs

### 7.1 明确**没有**做的

1. **回环采集比对**未实现 —— `BASELINE-005` 的本体，也是本线**故意**划出的边界；
2. **直接调厂商 API**（绕过 cpal 的 `playback`/`capture`）未实现 —— 而且**大概率不必要**（§1.1）；
3. **`BASELINE-005` 的门禁状态行没有改** —— `docs/ledger/gate-status.md` 是集成者地盘。
   该行**现在仍然是 `PENDING`**，且**这是正确的**（门禁本体没被测过）；
4. **没有把 `BASELINE-005` 加进 `src/lib.rs` 的 `IMPLEMENTED_SPEC_IDS`** —— 加了就是越权宣称：
   本 crate 实现的是**工具**，不是那条时延指标；
5. **没有接进 CI 流水线**（`.github/**` 是集成者地盘）；
6. 引擎的**实时线程优先级**仍是既有缺口（`ROAD-M2-001`）⇒ 抖动绝对值在 macOS/ALSA 上
   可能被人为放大（CoreAudio 由内核约束，Linux-ALSA 无开关）。抖动**不能**当硬件结论用。

### 7.2 已知的解释边界（读数字前必看）

- `DeviceTrait::buffer_size()` 的文档明确写着「**不保证**每次回调都正好这么多帧」⇒
  "标称周期"是**名义**周期；用回调时间戳对它求偏差会把"帧数不齐"也算成抖动（这就是 CPU 侧
  抖动的定义），但**不要**把它解释成硬件时延；
- `playback` / `capture` 是**主机预测**：PulseAudio 是估计值；CoreAudio 在
  `kAudioDevicePropertyLatency` 查询失败时会退化成"按本次回调的缓冲深度估算"
  （`cpal-0.18.2/src/host/coreaudio/macos/device.rs` 里 `latency_frames == 0` 的分支）——
  **API 层看不出它退化过**，这是本工具无法消除的解释边界；
- 主机把 `playback` 报成与 `callback` 相同时，**"不报"与"真的是 0"在数据上不可区分**
  ⇒ 工具只输出字面量 `unreported-or-zero` 并明确说"这不是 0 ms 时延"；
- 托管 runner 上的任何读数（如果有）只是数量级参考，不是达标依据；
- 工具的回调本身也守**红线 7**：预分配的 `Box<[AtomicU64]>` × 2 + 原子写，
  回调内零分配 / 零锁 / 零阻塞 I/O（在音频线程上分配会自己制造被测的抖动）。

### 7.3 needs（需要谁做什么）

| # | 需要 | 谁 | 备注 |
| :---: | :--- | :--- | :--- |
| 1 | **裁决**：`BASELINE-005` 的"原生系统 API 实测"是否接受"主机报告的驱动侧时延"这条路（§6 路 1） | 人类 / ADR | 若不接受，就只剩路 2；本线不自行裁决 |
| 2 | 把本工具接进一个**手动档** CI 作业，并**显式**按 `verdict` 记成"本环境无法测量"而不是通过 | 集成者（`.github/**` / `scripts/**`） | 建议 `--allow-unmeasurable` + 断言 `verdict=no-device` |
| 3 | 提供**带声卡（+ 回环）**的参考机并跑上面那条命令 | 人类 | 没有它，路 1/路 2 都无法执行 |
| 4 | 在 `docs/ledger/gate-status.md` 的 `BASELINE-005` 行补一句"工具已就绪（标称 + 驱动侧 + 抖动），门禁本体仍需裁决或回环" | 集成者 | 本线不得编辑该文件 |
| 5 | 若要让 CI 在**有设备**的机器上跑（self-hosted runner），需要确认许可与设备独占策略 | 人类 | 共享模式会被 OS 混音器影响，读数口径不同 |

### 7.4 顺手修的过时口径（**只改注释，算术一行未动**）

集成者转达"`DeviceDefinition::latency_samples` 在 D43 后变成必需字段，`0` 就是真零延迟"，
并要求更正 `crates/yeban-engine/src/graph.rs` 里"`0` = 未上报"的措辞。
**核对后本线没有改 `graph.rs`**，因为在本工作树的 `main`（`2f79c09`）上该前提**不成立**：

```bash
$ sed -n '441,446p' crates/yeban-model/src/project.rs      # 非本线地盘
    /// `#[serde(default)]` 是刻意的：缺失时取 `0`，这样旧文档仍可读…**但 0 必须被理解为"未上报"**
    #[serde(default)]
    pub latency_samples: u32,
$ bash scripts/dev/cargo-local.sh test -p yeban-model latency_samples
test project::tests::device_latency_samples_defaults_to_zero_and_round_trips ... ok
```

⇒ 字段仍是 `#[serde(default)]`，模型层文档仍写"0 = 未上报"，且有**现役判据**
`device_latency_samples_defaults_to_zero_and_round_trips` 断言"旧文档缺该字段必须仍可读"。
所以 `graph.rs` 的 `0` = 未上报**在今天仍然准确**，改了反而会写成假的；已回告集成者。

**本线确实修掉的**（这些才是真过时 —— 它们说"`yeban-model` 里还没有该字段"）：
`crates/yeban-engine/src/lib.rs` 的"设计边界 §5"、`crates/yeban-engine/src/snapshot.rs` 的
`TrackParams::from_track` 文档。改后措辞为"字段**已经存在**，[`graph`] 按设备链汇总它"，
并把"0 = 未上报 vs 真零延迟不可区分"这一**仍存在的**语义缺口如实写明（它属模型层裁决）。

---

## 8. 修改文件清单（本线）

| 文件 | 性质 |
| :--- | :--- |
| `crates/yeban-engine/src/latency.rs` | **新增**：零 cpal 依赖的纯计算 + 驱动侧分布 + 判定 + 机器可读行（18 条判据） |
| `crates/yeban-engine/examples/measure_latency.rs` | **新增**：cpal 测量机器（枚举 + 真流 + 驱动侧 + 抖动 + 可读行 + 退出码） |
| `crates/yeban-engine/tests/latency_cli_contract.rs` | **新增**：跑真二进制的运行期契约判据（5 条） |
| `crates/yeban-engine/src/lib.rs` | **改动**：注册 `pub mod latency;`、模块地图一行、边界说明，并更正"模型层没有 `latency_samples`"的过时口径 |
| `crates/yeban-engine/src/snapshot.rs` | **改动**：`TrackParams::from_track` 的文档更正（同上，**纯措辞**） |
| `docs/ledger/audio-latency-notes.md` | **新增**：本文件 |

**没有**改动：根 `Cargo.toml` / `Cargo.lock`（零新增依赖）、`.github/**`、`scripts/**`、`deny.toml`、
`docs/DEVELOPMENT_LEDGER.md`、`docs/adr/**`、`docs/YEBAN_*.md`、`schemas/**`、
除上述两个文档更正外的其它 `crates/**`、`spikes/**`、法务文件、`README*.md`。
