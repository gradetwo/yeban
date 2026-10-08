# decode-limits 工作线笔记 —— `HD-24`：PCM 体积上限的定性与改建

> 工作线: `line/decode-limits` · 工作树 `.worktrees/decode-limits` · 拥有目录 `crates/yeban-decode/**`
> 规范 ID: **HD-24**（`MAX_PCM_BYTES = 2 GiB` 不够：96 kHz 立体声 ≈46 min、96 kHz 8 声道 ≈11.6 min）、
> **ARCH-SEC-003**（不可信输入的资源上限必须仍然生效）、**ARCH-DET-001**（同输入 → 同输出；
> 也是"预算不随运行机器变化"的依据）、**ARCH-DSP-002**（重采样长度契约）、
> **ADR-0001 D43**（1.0.0 之前没有兼容包袱：更优解直接推翻旧设计）、
> **ADR-0001 D26**（symphonia/rubato 实际 API）、**ADR-0001 D32**（超越函数类分策）、
> AGENTS.md §2 **红线 7**（实时零分配；解码是离线路径，但分配不得出现在实时回调里）。
> 本文件是 `docs/ledger/` 下的**工作线附件**，不是 Normative 规范；与规范冲突时以规范为准。

---

## 1. 上限的定性结论与依据（本线最重要的产出）

**结论：`MAX_PCM_BYTES = 2 GiB` 是一个「安全闸门 / 内存预算」，不是「实现限制」。**
因此它的正确形态是**显式可配置 + 默认值有依据 + 与"可用内存"相关**的参数化预算，
而不是一个写死的数字，也**不是**把它改成 32 GiB 了事。

### 1.1 为什么它是闸门（不能删）

它守的是**不可信输入**：容器头可以声明任意帧数/声道数/采样率，一个几百字节的畸形文件
就能让"先按声明分配再说"的实现把进程吃掉。这个威胁真实存在（同族事故见
[`decode-core-notes.md`](decode-core-notes.md) §2.3 的"解封装器不报错也不推进"），
所以改建**没有**放松任何一道闸门：五道闸门（输入字节 / PCM 字节 / 声道数 / 采样率 / 时长）
全部保留，而且判据更细。

### 1.2 为什么它不是实现限制（凭据逐条）

| 凭据 | 事实 | 出处 |
| :--- | :--- | :--- |
| 没有任何缓冲需要"一次装下 ≤ 2 GiB" | 解码产物是 `Vec<f32>`，只受地址空间限制；分块增长（`try_reserve` + `resize`） | [`decode.rs`](../../crates/yeban-decode/src/decode.rs) |
| 算术不受 2 GiB 约束 | `frames × channels` 走 `checked_mul`，长度契约走 `u128` 精确有理数，无浮点/超越函数要求 | [`limits.rs`](../../crates/yeban-decode/src/limits.rs) |
| 重采样不要求 2 GiB | `rubato` 只吃切片（`InterleavedSlice`），输出长度由 `process_all_needed_output_len` 算出后 `try_reserve` | [`resample.rs`](../../crates/yeban-decode/src/resample.rs) |
| **数字的来源不是解码** | 2 GiB 直接沿用 [ARCH-SEC-003] 对**归档单条目**的 "≤ 2 GB"；改建前的注释自述"同一个数字只在一个地方被裁决" | 改建前的 `limits.rs:26-27`、[`decode-core-notes.md`](decode-core-notes.md) §4 |

也就是说：**这个数字是从别处的信任上下文借来的**（容器层的归档条目上限），
而不是从"解码器真的需要多少内存"推出来的 —— 这正是 `HD-24` 暴露出来的口径缺陷。

### 1.3 它同时**不是**纯粹的容器层闸门：它是一份"内存预算"

要诚实说明另一面：`DecodedAsset` 是**整份不可变资产**（`Arc` 后给实时线程只读消费，
[ARCH-TOP-002] / [红线 7]），重采样又要在输入之外再分配一份输出。所以这条上限**确实**
在约束离线管线的峰值内存 —— 但它约束的是**内存预算**，不是"实现要求 2 GiB"。
改建后这个预算被显式化、可配置化（见 §2、§4）。

### 1.4 为什么**不**自动探测可用内存

"与可用内存相关"被落实为**参数**，而不是运行时探测，理由是确定性：
自动探测会让"同输入 → 同输出"（[ARCH-DET-001]）变成**运行机器的函数** ——
同一个文件在 32 GB 机器上解码成功、在 16 GB 机器上被预算拒绝，资产内容随环境变化。
正确分工：本 crate 提供**有依据的默认值**与 `PcmBudget::for_layout()` 这个
"从产品要求反推预算"的构造函数；**应用层**按自己的内存情况显式给值。

---

## 2. 默认值怎么来的（可被判据复算）

### 2.1 推导（写进代码，不是写在注释里）

产品要求（两条，取 PCM 字节数**较大**者）：

| 档 | 要求 | 精确字节数 |
| :--- | :--- | :--- |
| 整场工程 | 96 kHz **立体声 3 小时** | `3×3600 × 96000 × 2 × 4` = **8 294 400 000** |
| 多声道母带 | 96 kHz **8 声道 30 分钟** | `1800 × 96000 × 8 × 4` = 5 529 600 000 |

⇒ 默认 `max_pcm_bytes = 8 294 400 000` B（≈7.73 GiB，是旧值的 **3.86×**）；
`max_input_bytes = max_pcm_bytes + 1 MiB`（容器开销余量）= 8 295 448 576 B。

**同一预算折算成时长的等价关系**（判据逐条复算）：

| 布局 | 旧默认 2 GiB | 新默认 8 294 400 000 B |
| :--- | :--- | :--- |
| 96 kHz 立体声 | ≈46 分钟 | **3 小时**（定义值） |
| 96 kHz 8 声道 | ≈11.6 分钟 | **45 分钟**（= 3 h × 2/8） |
| 48 kHz 立体声 | ≈1.55 小时 | **6 小时整**（与时长闸门同时到达） |
| 44.1 kHz 立体声 | ≈1.69 小时 | ≈6.53 小时（**时长闸门先跳闸**在 6 h） |
| 44.1 kHz 单声道 | ≈3.38 小时 | ≈13.06 小时（**时长闸门先跳闸**在 6 h） |

这就是"为什么是这个数"的答案：它**不是**一个取整的容量，而是
"96 kHz 立体声 3 小时"这一条产品要求的**闭式函数**，判据用同一个函数复算。

### 2.2 五道闸门与各自默认值

| 闸门 | 默认值 | 依据 |
| :--- | :--- | :--- |
| 输入容器字节 | PCM 预算 + 1 MiB | 未压缩 WAV 只多 44 B 量级；1 MiB 容纳合法元数据块。压缩容器天然更小 |
| 交织 `f32` PCM 字节 | 8 294 400 000 | §2.1 |
| 声道数 | 64 | 启用容器常见布局 ≤ 7.1（8 声道）的 8× 余量；挡"声明 65535 声道" |
| 采样率 | 768 000 Hz | DXD 之上再留一倍 |
| **时长** | **6 小时** | **新增的独立闸门**：低采样率 × 少声道"字节便宜、时间昂贵"，字节预算管不住时间轴。6 h 故意**松于**默认字节预算在 96 kHz 立体声下的 3 h（那里字节先跳闸），又**故意紧于**它在 44.1 kHz 立体声下的 6.53 h（那里时长先跳闸） |

### 2.3 峰值内存模型（诚实声明：`max_pcm_bytes` 是**单份 PCM** 预算）

| 路径 | 峰值（份数 × PCM） | 说明 |
| :--- | :--- | :--- |
| `decode_bytes` / `decode_path` | ×1 | 只累积一份交织 `f32` |
| `import_path` / `import_bytes` | ×1 + 容器字节 | 为了 SHA-256 摘要，输入容器整份驻留（`AssetHash::of_bytes` 要求完整字节） |
| 重采样（`*_with_budget`） | ×2 | 输入资产 + 输出缓冲（上采样还会放大输出长度） |
| `DecodedAsset::pcm_hash` | ×2 | 现有实现 `Vec::with_capacity(32 + len*4)` 再逐样本填充，等于**再复制一整份** |

⇒ 最坏情形（3 h 96 kHz 立体声 WAV，`import_path` + 重采样）峰值 ≈ 容器(8.29 GB) +
资产(8.29 GB) + 输出(8.29 GB) ≈ 25 GB。**默认值不按峰值推导**（那需要知道调用方
是否要哈希、是否要重采样、机器有多少内存）；调用方应当用
"`PcmBudget::for_layout(...)` 给的基准 × 自己的峰值因子"来定。
把峰值压到 O(块) 需要流式化（见 §7 `needs`）。

---

## 3. 改建前后对照

| 维度 | 改建前 | 改建后 |
| :--- | :--- | :--- |
| 上限形态 | 5 个写死 `pub const`（`MAX_PCM_BYTES` / `MAX_INPUT_BYTES` / `MAX_INTERLEAVED_SAMPLES` / `MAX_CHANNELS` / `MAX_SAMPLE_RATE`） | 1 个显式结构 [`PcmBudget`](../../crates/yeban-decode/src/limits.rs)（5 个字段）；旧常量**全部删除**，不留兼容层 |
| 默认 PCM 上限 | 2 GiB（借自归档条目上限） | 8 294 400 000 B（96 kHz 立体声 3 h，由产品要求推导） |
| 默认输入上限 | 2 GiB（与上述同一个数字） | PCM 预算 + 1 MiB（换算而来） |
| 默认时长上限 | **不存在**（没有独立闸门） | 6 小时（新增，独立生效） |
| 可配置性 | 只有 `decode` 侧两个裸字段（`DecodeOptions.max_input_bytes` / `max_pcm_bytes`）；`resample` / `asset` / `propcheck` 直接吃写死常量 | 所有使用点走同一个 `PcmBudget`：`DecodeOptions.budget`、`decode`、`asset::import_*`、`resample_*_with_budget`、`propcheck` |
| 重采样预算 | **绕过调用方**（写死 `MAX_PCM_BYTES`）；恒等路径完全不查预算 | 走调用方 `budget`；恒等路径也查（它同样复制一整份 PCM） |
| 错误变体 | `TooManyFrames { frames, channels, samples, limit }` | `PcmBudgetExceeded { frames, channels, samples, limit_samples }` + 新增 `DurationTooLong { frames, sample_rate, seconds, limit_secs }` |
| 错误分类（上游可见） | `DecodeError::Budget` → MCP `RENDER_FAILED` + `data.decodeError = "budget"` | **不变**（未新增/删除 `DecodeError` 变体，`decode_error_class` 的穷尽 match 不需要改） |
| 默认值可判据复算 | ✗（只是一个常数） | ✓（`pcm_bytes_for` 是唯一换算函数，判据用同一条公式复算等价时长 3 h / 45 min / 6 h） |
| 上游源码形状 | `decode_bytes(bytes, &DecodeOptions::default())`、`resample_interleaved(&s, ch, a, b)` | **签名不变**（4 参/2 参入口保留为"默认预算入口"）；新增 `*_with_budget` 显式入口 |

---

## 4. 上游该怎么配合（`yeban-mcp` 的 `yeban_render_master`）

现状（`crates/yeban-mcp/src/domain/render.rs`）：音频片段走
`yeban_decode::decode_bytes(bytes, &DecodeOptions::default())` → 裁编码器延迟 →
`resample_interleaved(&trimmed, ch, src_rate, tgt_rate)`。

**本线对上游的影响，三条：**

1. **不需要改代码就能继续编译、继续工作。** 两个入口的签名没变；`DecodeOptions::default()`
   现在携带 §2 推导出来的默认预算（比旧默认宽松 3.86×，对小素材行为完全相同）。
   本线在集成判据里用**函数指针**钉住了这三个签名（C65），所以"上游被悄悄改红"
   这件事本身有判据。
2. **建议的配合（按优先级）：**
   - **把预算从项目/服务层传下来**：不要让渲染路径永远吃"库默认值"。
     `DecodeOptions { budget, ..Default::default() }` 与
     `resample_interleaved_with_budget(..., &budget)` 是同一份预算的两个入口。
     应用层最清楚自己有多少内存，例如
     `PcmBudget::for_layout(项目最长片段秒数, 目标采样率, 声道数)` 再乘峰值因子（§2.3）。
   - **渲染前先算峰值**：`decode` ×1、`resample` ×2。若一次渲染要对多个片段做预算控制，
     应在**片段层面**串行（当前实现就是逐片段 prepared），而不是把 N 个片段的预算相加。
   - **错误分类不用改**：预算失败仍然是 `DecodeError::Budget` → `RENDER_FAILED` +
     `data.decodeError = "budget"`；`detail` 现在带上精确数字（帧数/声道数/样本数/上限），
     可自纠。`schemas/mcp-tools.schema.json` 的错误码枚举**不需要**新增条目。
3. **`audio-render-notes.md` 里"`resampler` 口径"的稳定性**：`Async::new_sinc` 的参数
   （`sinc_len=256` / `BlackmanHarris2` / `chunk=1024` / `max_relative_ratio=1.0`）与
   长度契约（`resample_len_contract`）本线**一个字都没改**；新增的只是"输出缓冲要过
   调用方预算"这一道检查（默认预算下不会触发）。C71 / C72 分别钉住"重采样走调用方
   预算"与"把预算收到恰好够时不改样本值"。

---

## 5. 判据清单（C48–C58）

| # | 判据 | 钉住什么 | 执行位置 |
| :--- | :--- | :--- | :--- |
| C48 | `limits::default_budget_is_recomputed_from_the_product_requirements` | **默认值的依据可复算**（3 h/45 min/6 h 等价关系 + 有限性上界）—— **注入 ② 的靶子** | 本机 + CI |
| C49 | `limits::for_layout_admits_exactly_the_requirement_it_was_derived_from` | `for_layout` **恰好**容纳它声明的要求（闭区间）；退化参数返回 `None` | 本机 + CI |
| C50 | `limits::input_byte_budget_is_enforced` | 输入字节闸门闭区间（`>` 不是 `>=`）—— **注入 ③ 的靶子之一** | 本机 + CI |
| C51 | `limits::every_budget_gate_trips_on_its_own` | 声道数 / 采样率 / **时长** / PCM 字节**各自独立**跳闸 | 本机 + CI |
| C52 | `limits::layout_budget_rejects_an_asset_over_the_pcm_cap` | PCM 字节闸门闭区间 + **可配置**（小预算对小素材生效）—— **注入 ①/③ 的靶子** | 本机 + CI |
| C53 | `limits::layout_product_overflow_is_detected_not_wrapped` | `frames × channels` 溢出被检出（不回绕） | 本机 + CI |
| C54 | `propcheck::pcm_size_conversion_is_exact_or_refused_never_wrapped`（属性测试） | 尺寸换算精确或明确拒绝 | CI（需 proptest） |
| C55 | `propcheck::layout_budget_admits_exactly_the_documented_envelope` | 四道闸门合取 = 预算通过；**预算通过 ⇒ PCM 字节 ≤ `max_pcm_bytes`（内存上界）** | CI（需 proptest） |
| C56 | `decode::the_duration_gate_fires_independently_of_the_byte_budget` | 时长闸门在**真实解码**路径上独立生效（含精确错误文本） | CI |
| C57 | `decode::the_channel_and_rate_gates_fire_on_their_own` | 声道/采样率闸门在真实解码路径上独立生效 | CI |
| C58 | `decode::the_pcm_budget_boundary_is_closed_through_the_decoder` | 解码器上的闭区间边界（少 4 字节即拒） | CI |
| C59 | `decode::the_default_options_carry_the_derived_budget` | 单一事实源（`DecodeOptions::default().budget == PcmBudget::default()`） | CI |
| C60 | `resample::the_resampler_budget_is_the_callers_budget_not_a_hard_coded_cap` | 重采样走**调用方**预算（含"只够理想输出也会被拒"：闸门在真实分配长度上） | CI |
| C61 | `resample::the_resampled_length_gate_is_independent_of_the_byte_budget` | 重采样输出的时长闸门独立 | CI |
| C62 | `resample::the_identity_path_still_obeys_the_budget` | 恒等路径也查预算（它同样复制一整份 PCM） | CI |
| C63 | `resample::resample_asset_with_budget_refuses_output_over_the_budget` | 转换后的新资产过同一预算 | CI |
| C64 | `resample::the_plain_entry_points_are_the_default_budget_entry_points` | 裸入口 = 默认预算入口（逐位相同） | CI |
| C65 | `tests/pcm_budget.rs::upstream_call_forms_still_typecheck_against_the_budget_api` | **上游源码兼容**（函数指针核对三个旧签名）+ 默认预算单一事实源 | CI |
| C66 | `tests/pcm_budget.rs::default_budget_is_recomputed_from_the_product_requirements` | 公开 API 上的默认值复算 | CI |
| C67 | `tests/pcm_budget.rs::default_budget_admits_the_documented_session_and_refuses_one_more_frame` | 公开 API 上的闭区间边界 | CI |
| C68 | `tests/pcm_budget.rs::a_small_pcm_budget_stops_a_real_wav_with_a_precise_error` | 可配置小预算 + **精确错误文本**（帧数/声道数/字节数） | CI |
| C69 | `tests/pcm_budget.rs::the_input_byte_cap_is_precise_and_precedes_probing` | 输入字节闸门精确到字节、在探测之前 | CI |
| C70 | `tests/pcm_budget.rs::the_duration_cap_is_independent_and_precise` | 时长闸门独立 + 精确文本 + 闭区间 | CI |
| C71 | `tests/pcm_budget.rs::the_resample_path_obeys_the_callers_budget` | `audio-render` 口径的重采样路径走调用方预算 | CI |
| C72 | `tests/pcm_budget.rs::tightening_the_budget_to_the_exact_pcm_size_does_not_change_the_samples` | **解码正确性回归**：预算收到"恰好够"时 `pcm_hash` 与逐位样本不变 | CI |

对账：**25 条新增/改写判据**（C48–C72）。加上继承自 `decode-core` 的既有判据
（长度契约、`IdleGuard`、格式/位深矩阵、`pcm_hash` 敏感性……）全部保留未动。

---

## 6. 本机真跑 vs CI（严格区分）

**本机真跑（可执行的那一层，零第三方依赖）：**

1. `rustc --edition 2024 --test -D warnings` 单独编译 [`limits.rs`](../../crates/yeban-decode/src/limits.rs)：
   **13 passed / 0 failed**。
   ```bash
   cd .worktrees/decode-limits
   source scripts/dev/local-env.sh
   rustc --edition 2024 --test -D warnings target/local-verify/pure_limits.rs \
         -o target/local-verify/pure_limits && ./target/local-verify/pure_limits
   ```
   （驱动 `target/local-verify/pure_limits.rs` **不入库**，只含 `#[path] mod limits;`。）
2. **额外**用工作区同款 lint 配置（`clippy::all = deny` / `rust_2018_idioms` /
   `#![deny(missing_docs)]`）在本机跑了一遍 `limits.rs` 的 clippy —— 做法是在 `target/`
   下建一个**零依赖**的小工程（`[workspace]` 自成一体）把 `limits.rs` `#[path]` 进去：
   ```bash
   bash scripts/dev/cargo-local.sh clippy \
        --manifest-path target/local-verify/clippycheck/Cargo.toml --all-targets
   ```
   这一步**当场抓到 2 处 `clippy::identity_op`**（`u64::MAX & !3` 里 `u64::MAX &` 无效果），
   修成 `!3u64` 后本机即绿 —— 若不做这一步，这两条会以 CI 红的形式出现（本机不能编译
   重依赖 ⇒ clippy 是唯一必须靠 CI 才能看到的门禁，见 `decode-core-notes.md` §7.3）。
3. `bash scripts/gates/run-gates.sh light` → **通过**（提交前）。
4. `bash scripts/dev/cargo-local.sh fmt -p yeban-decode --check` → 通过。

**明确只由 CI 执行（本机不编译 symphonia/rubato）：**
`decode.rs` / `asset.rs` / `resample.rs` / `propcheck.rs` 与 `tests/pcm_budget.rs` 的
全部判据（C56–C72，含 `clippy --all-targets` 对这几个文件）。本机对它们的把握来自
逐处静态复核与"零依赖层同款 lint 已跑过"，**不是**"本机验证过"。
`run-gates.sh crate yeban-decode` 会因重依赖 **SKIP**（政策：本机不跑重活）。

### 6.1 注入 → 变红 → 还原（3 次，全部在本机可执行的那一层）

做法：把 `limits.rs` 复制到 `target/local-verify/limits_injected.rs` 打补丁，
用 `rustc --edition 2024 --test -D warnings` 编译执行；**真实源文件全程未被改动**，
每次注入后都用原始文件重新编一次确认回到绿。

| # | 注入内容 | 变红的判据 | 读数 |
| :--- | :--- | :--- | :--- |
| ① | `if samples > limit_samples` → `if false && samples > limit_samples`（PCM 字节闸门**永远通过**） | `every_budget_gate_trips_on_its_own`、`layout_budget_rejects_an_asset_over_the_pcm_cap`、`for_layout_admits_exactly_the_requirement_it_was_derived_from` | **10 passed / 3 failed** |
| ② | 默认预算的 PCM 上限改成 `u64::MAX` 量级（`pcm_bytes.saturating_add(u64::MAX)`） | `default_budget_is_recomputed_from_the_product_requirements`（含"有限性上界"）、`input_byte_budget_is_enforced`、`layout_budget_rejects_an_asset_over_the_pcm_cap` | **10 passed / 3 failed** |
| ③ | `if samples > limit_samples` → `if samples >= limit_samples`（边界从 `>` 改成 `>=`） | 同上三条（都是闭区间判据） | **10 passed / 3 failed** |

还原后：**13 passed / 0 failed**（见 §6 第 1 条）。

**诚实边界**：三次注入都落在零依赖层，因为那是本机唯一可执行的层。
CI 专属的集成判据（C56–C72）与注入 ①/③ **共享同一个** `check_layout` 判定，
因此注入 ①/③ 的红可以视为对它们的**间接**取证，但这不等同于"注入过集成路径"。

---

## 7. CI 判决读数

（本节在**代码提交被 CI 读完判决之后**回填；按 L23/L26，文档改动不与代码提交同批推送，
以免 `concurrency.cancel-in-progress` 把代码那一轮的 run 吃掉。）

| 轮 | 提交 | 分支 tip SHA | run | 读数 |
| :--- | :--- | :--- | :--- | :--- |
| 1 | `c5d7430`（代码 + 集成判据） | `c5d7430d4d5c420a6da524efec8041c50b61fd6a` | [37247431884](https://github.com/gradetwo/yeban/actions/runs/37247431884) | 见下（全绿） |

**第 1 轮逐腿读数**（`gh run view --job=<id> --log`，不是只看第一条）：

| 腿 | 结果 | 说明 |
| :--- | :--- | :--- |
| `plan (受影响集合)` | ✓ | 判定为**受影响集合**（`yeban-decode` + 下游 `yeban-mcp` / `yeban-ui-mcp` + `windows`），**不是** workspace 全量 ⇒ "本线只碰 `crates/yeban-decode/**`"的声明成立 |
| `lockfile` / `deny` / `checks` | ✓ | 未新增依赖、未改根清单/契约，三条是"没被动过"的绿 |
| `rust (yeban-decode)` | ✓ | `clippy -p yeban-decode --all-targets --locked -- -D warnings` ✓；`test --all-targets`：lib 单元判据 **80 passed / 0 failed**（改建前 68 ⇒ 本线净增 12 条），集成 `tests/pcm_budget.rs` **8 passed / 0 failed** |
| `rust (yeban-mcp)` | ✓ | **上游源码兼容的正面证据**：`yeban-mcp` 一行未改，却在新的公共 API 形状下编译并通过它自己的判据（C65 的函数指针判据是同一件事的 crate 内版本） |
| `rust (yeban-ui-mcp)` / `windows (...)` | ✓ | 与本线无关的腿，全绿 |
| `rust (workspace 全量)` | 跳过 | `plan` 判定非全量（对比 `decode-core` 当年改了 `Cargo.lock` 而被判全量） |

`bash scripts/dev/ci-verdict.sh line/decode-limits` 退出码 **0** ⇒ 判决**属于本线 tip**
（脚本内置 `assert_verdict_matches_tip`；L23 要求的"这个 run 属于哪个 SHA"因此有机械答案）。
本线**不**超出这个范围宣称任何东西：`rust (workspace 全量)` 这一腿被跳过，所以
"全量工作区"没有被这次 run 覆盖 —— 那是 `plan` 的正常行为，不是本线可以宣称的绿。

**文档提交的节奏（L23/L26）**：本文件（只改 `docs/ledger/**`）是在**读完上面这一轮判决
之后**才推送的第二个提交，避免 `ci.yml` 的 `concurrency.cancel-in-progress` 把代码那一轮
的 run 吃掉。若 CI 对这次 docs-only 推送判定"无受影响 crate"，本线的**代码**判决仍然是
上面这一轮（`c5d7430`）。

---

## 8. needs / pending / 未实现项

| 类型 | 条目 | 说明 |
| :--- | :--- | :--- |
| `needs` | **流式 / 分块解码** | 本线选择的是"可配置上限"而不是"改流式"，因为 `DecodedAsset` 是**整份不可变资产**（`Arc` 后给实时线程只读），把它改成流式意味着实时侧的消费契约（`yeban-engine` 的资产指针交换）与 `yeban-mcp` 的渲染路径一起改 —— 超出 `crates/yeban-decode/**` 的地盘，且会破坏 [红线 7] 的"实时线程只读已就绪资产"边界。接口草图：`DecodeSession::next_chunk(&mut self, frames) -> Result<Option<ChunkRef>>` + `AssetSink`；峰值内存目标 O(chunk)。**判据口径**：以"峰值分配与文件长度无关"作为接受标准（例如按 `try_reserve` 调用次数/最大单次请求量的计数判据），本线**没有**做出这条证据，因此不声称流式。 |
| `needs` | **增量 / 流式 SHA-256** | `AssetHash::of_bytes` 要求完整字节，因此 `import_path` 必须把容器整份读进内存（§2.3 峰值 ×1 + 容器）；`DecodedAsset::pcm_hash` 现在还会**再复制一整份**样本字节。建议在 `yeban-model` 侧提供 `AssetHasher`（`update`/`finalize`），本 crate 就能把两条路径都压到 O(块)。属跨 crate 变更，登记为 needs。 |
| `needs` | **`docs/ledger/human-decisions.md` 的 `HD-24` 行自相矛盾** | 该行"裁决"列写 **B**（提高上限或改流式），但结论列写"**保持 2 GiB，超限返回 `IO_ERROR`**"，与 `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md`（"裁决：提高上限或改为流式，列为待实现项"）和 `docs/DEVELOPMENT_LEDGER.md`（"排队"）冲突。本线**未改**该共享文档（不在本线地盘），请集成者/人类把结论列就地改写成"B：已由 `PcmBudget` 落地"。另注：`IO_ERROR` 与实现也不符 —— 预算失败走 `DecodeError::Budget` → MCP `RENDER_FAILED` + `decodeError="budget"`（`IO_ERROR` 只留给真正的 I/O 失败）。 |
| `needs` | **ARCH-SEC-003 的口径解耦需要在规范侧落字** | 规范 §5.3 的"单个解压条目 ≤ 2 GB"约束的是**容器层**（`yeban-model` 归档解包，Zip-Slip/炸弹那一族），解码侧的读缓冲是**另一个信任上下文**（用户直接导入的文件）。本线把解码侧改成"PCM 预算 + 1 MiB"并写明区别；规范正文的修订属人类职责（Agent 不擅改 `docs/YEBAN_*.md`）。 |
| `needs` | **预算的"峰值因子"应由应用层提供** | 本 crate 只给单份 PCM 预算（§2.3）；`yeban-app` / `yeban-services` 若能提供"可用内存 → 预算"的设置项（含用户覆盖），本线的 `for_layout` 就是它的换算函数。**不**在 crate 内探测内存（破坏 [ARCH-DET-001]，见 §1.4）。 |
| `pending` | **跨架构确定性对账（[ARCH-DET-001] L2）** | 继承 `decode-core` 的 pending：需要双 runner 比对最大样本差。本线的预算判定是纯整数运算（无超越函数），因此与 D32 的"超越函数类"无关。 |
| `pending` | **基准（`BASELINE-*`）** | 本线未加基准；默认预算变大后"大素材的解码/重采样吞吐"没有打点。 |
| `pending` | **OGG/Vorbis 与 ADPCM 的字节级夹具** | 继承 `decode-core` 的 pending，与本线无关。 |
| 已关闭 | `HD-24` 的"上限形态"问题 | 本线给出定性 + 可配置实现 + 默认值依据 + 判据 + 注入；`HD-24` 在实现侧可以结案（文档侧见上面 needs）。 |

> ⚠️ **2026-10-08 更正注（上面的 `needs`「增量 / 流式 SHA-256」点名的两条路径**都已落地**；原句保留不改写）**
> 该 `needs` 只说对一个"缺口"，而两条路径都已经改成**流式**（不新增 `AssetHasher` 的公开面）：
> - **① `pcm_hash`**：由 `ccee870`（`perf(decode): stream the PCM digest instead of copying the whole asset [ARCH-DET-001]`）落地。复核：`crates/yeban-decode/src/asset.rs:258` 的 `pcm_hash(&self) -> AssetHash` 现在按分块喂哈希（`:274` 的 `hasher.update(&staging[..filled])`）；只有头部字段（通道数 / 采样率 / 帧数）在分块之前喂入。
> - **② `import_path` 的容器摘要**：由 `ab42c82`（`perf(decode): stream the CAS digest of import_path instead of buffering the container [MODEL-AST-007]`）落地。复核：`crates/yeban-decode/src/asset.rs:333` 的 `fn hash_reader<R: Read>(mut reader: R) -> DecodeResult<AssetHash>` 用固定缓冲循环读（`:341` 的 `hasher.update(&buffer[..filled])`），因此容器**不再整份进内存**；判据 `crates/yeban-decode/tests/import_streaming.rs`（172 行，`ab42c82` 新增）从"真实文件"一侧复算同一摘要。
> ⇒ 该 `needs` 的验收标准（"两条路径都压到 O(块)"）**已满足**；本表里剩下的流式相关条目只有「流式 / 分块解码」（峰值目标与文件长度无关）。

---

## 9. 修改文件清单与净行数

**第 1 个提交 `c5d7430`（代码 + 集成判据）：7 个文件，+1134 / −129**（`git show --numstat`）：

| 文件 | 变化 | 内容 |
| :--- | :--- | :--- |
| [`crates/yeban-decode/src/limits.rs`](../../crates/yeban-decode/src/limits.rs) | +511 / −79 | `PcmBudget` + 默认值推导（`pcm_bytes_for` / `for_layout`）+ 五道闸门 + 13 条单元判据 |
| [`crates/yeban-decode/src/decode.rs`](../../crates/yeban-decode/src/decode.rs) | +153 / −19 | `DecodeOptions.budget` + 五道闸门接线 + 4 条集成判据 |
| [`crates/yeban-decode/src/resample.rs`](../../crates/yeban-decode/src/resample.rs) | +171 / −12 | `*_with_budget` + 恒等路径预算 + 5 条判据 |
| [`crates/yeban-decode/src/asset.rs`](../../crates/yeban-decode/src/asset.rs) | +11 / −5 | `import_path` 走 `budget` |
| [`crates/yeban-decode/src/propcheck.rs`](../../crates/yeban-decode/src/propcheck.rs) | +28 / −5 | 预算包络属性 + 内存上界 + 尺寸换算不回绕 |
| [`crates/yeban-decode/src/lib.rs`](../../crates/yeban-decode/src/lib.rs) | +22 / −9 | 上限口径表改写 + 新入口 re-export |
| [`crates/yeban-decode/tests/pcm_budget.rs`](../../crates/yeban-decode/tests/pcm_budget.rs) | +238 / −0 | **新增**：8 条集成判据（含上游签名核对） |

**第 2 个提交（本文件）**：[`docs/ledger/decode-limits-notes.md`](decode-limits-notes.md)，
313 行，只改 `docs/ledger/**`（按 L23/L26 与代码提交分开推送）。

**没有**改根 `Cargo.toml` / `Cargo.lock`（**零新增依赖**：`Cargo.toml` 的
`[dependencies]` 一个字都没动）、`.github/**`、`scripts/**`、`deny.toml`、`docs/adr/**`、
`docs/YEBAN_*.md`、`schemas/**`、其它 `crates/**`、`spikes/**`、法务文件、`README*.md`。
