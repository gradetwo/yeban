# l1-digest 工作线账本

> 分支 `line/l1-digest`，工作树 `.worktrees/l1-digest`（main `b8fe21e`），地盘
> `crates/yeban-render/**`。本文件只由本工作线维护；`docs/DEVELOPMENT_LEDGER.md`、
> `docs/ledger/gate-status.md`、根 `Cargo.toml`、`scripts/**`、`.github/**` 均由**集成者独占**，
> 本线**未触碰**。
>
> 使命：为 **`MUST-GATE-003`（跨架构 L2 一致性 `< 1e-6`）** 铺出**可执行的**路径。

---

## 1. 这一线要解决的真实阻塞：不是硬件，是产物

`MUST-GATE-003` 一直以 `PENDING` 挂在 `docs/ledger/gate-status.md`，理由从来是"需要 x86_64 与
AArch64 两条真跑后对账"。GitHub 现在提供 `ubuntu-24.04-arm` runner —— **缺的不是第二个架构**，
缺的是一份**规范化的、可跨机比较的 L1 读数**，以及一个**能被 CI 直接调用的比较器**。

本线交付的就是这两件东西，让"x86_64 跑一次 + ARM 跑一次 → 比对"变成**两条命令**。

---

## 2. 落地文件与规范 ID

| 文件（相对 `crates/yeban-render/`） | 内容 | 规范 ID |
| :--- | :--- | :--- |
| `examples/support/l1_receipt.rs` | **收据格式（v1）**：模型、确定性序列化、严格解析、判决、报告。零第三方依赖 | `MUST-GATE-002`、`MUST-GATE-003`、`ARCH-DET-001/002`、ADR-0001 **D32** |
| `examples/support/reference_project_a.rs` | **参考工程 A 的唯一夹具**（图构造 + 样本源），`bench_render` 与收据导出**共用同一份文件** | `BASELINE-001`、`ARCH-DET-001` |
| `examples/support/export_pipeline.rs` | 渲染 → 收据的**共用装配管线**（example 与 CI 判据同源） | `MUST-GATE-002/003`、`ARCH-PDC-001` |
| `examples/export_l1_receipt.rs` | **读数导出**（CI 里跑的那条命令） | `MUST-GATE-003` |
| `examples/compare_l1_receipts.rs` | **比较器**（退出码 0/1/2，CI 直接调用） | `MUST-GATE-002/003` |
| `examples/support/l1_receipt_tests.rs` | **本机零依赖验证脚手架**（`rustc --test`，18 条判据，**不是** cargo 目标） | — |
| `tests/l1_digest_contract.rs` | **真渲染的端到端判据**（10 条，只由 CI 判定） | `ARCH-DET-002`、`ARCH-PDC-001` |
| `examples/bench_render.rs`（**已改**） | 改为引用共用夹具，消除"基准读数"与"确定性读数"两份夹具漂移的可能 | `BASELINE-001` |

**没有新增任何依赖**，因此没有 `TODO(hoist)`。收据格式模块只用 `core`/`std`
（用 `BTreeMap` 而非 `HashMap`：`crates/**` 的 G01 守卫禁止后者，且确定性本来就要求有序容器）。

---

## 3. 收据格式（`yeban-l1-receipt v1`）

### 3.1 语法

行式 UTF-8 文本。第一个非注释行必须是 `yeban-l1-receipt v1`。空行与 `#` 开头的行被忽略。
其余每行是 `<键><一个空格><值>`，值的语法**按键固定**（解析器严格：未知字段、坏十六进制、
未知类别码、不可比数字、下标不递增、样本表不完整、自相矛盾 —— 一律报错，**绝不 panic**）。

| 键 | 语义 | 判定用途 |
| :--- | :--- | :--- |
| `fixture` | 夹具名（本线固定 `reference-a`） | 指纹（必须相同） |
| `tracks` / `frames` / `channels` / `sample_rate` / `block_size` / `seed` | 工程与选项指纹 | 指纹（必须相同） |
| `gain_db` | `none` 或 `f32`：夹具的边增益设置 | 指纹（必须相同） |
| `latency <ULID> <帧>` | 每节点注入的 PDC 延迟，按键升序；可 0 行 | 指纹（必须相同） |
| `threads` | `auto` 或 `N` | **不算指纹**：`ARCH-DET-002` 的实测结论就是"线程数不改变母带字节"，比较器必须能表达这件事 |
| `target_arch` / `target_os` / `target_env` / `target_endian` / `target_pointer_width` | `cfg!` 的编译期事实（`target_env` 为空时写 `-`） | 收据自证架构（非空） |
| `target_triple` | `rustc -vV` 的 `host:`（取不到时由 arch/os 合成，仍非空） | 判定"同目标 vs 跨目标" |
| `rustc_version` | `rustc -vV` 的 release + commit + 日期 | 审计 |
| `digest` | `RenderOutput::digest`：每个 `f32` 取位型做 SHA-256，64 位小写十六进制 | 位级判决 |
| `longest_path_frames` | `RenderPlan::longest_path_frames()`（PDC `L_max`） | 判据 ⑧ |
| `pdc_expected_frames` | 导出侧**独立复算**的 `L_max`；与上一字段不等时收据被拒 | 判据 ⑧ |
| `sample_count` | **全部**样本数（= `frames * channels` = 样本表长度） | 防抽样假绿 |
| `abs_max` / `sum_squares` | `max\|s\|`、`Σs²`（`f64` **顺序**累加；`{:.17e}` 精确往返） | 人读的幅值统计量 |
| `class E ieee-exact` / `class T transcendental` | D32 的两个类别，**无条件**都要声明 | D32 分策 |
| `op <类别码> <运算@位置>` | 路径上的每一个运算及其类别，如 `op T libm::powf@db-to-linear` | D32 分策（"哪些运算是超越函数类"的显式声明） |
| `sample <起始下标> <类别码> <位型,...>` | 一行最多 16 个样本，**行内同类**、行间下标严格递增；位型是 8 位小写十六进制 | 逐样本判决 |

**为什么用位型而不是数值**：`-0.0` 与 `+0.0` 数值相等但位型不同；NaN 的载荷也是位级一致的一部分。
（真实夹具不会产出 NaN/Inf：`Receipt::validate` 见到非有限样本即拒绝整份收据。）

**为什么收据里没有时间戳**：判据 ① 要求"同一台机器两次导出 ⇒ 收据**逐字节相同**"。
任何时间/耗时/主机名都会破坏它。因此收据只有"输入指纹 + 输入无关的读数 + 工具链身份"。

**为什么不做抽样**：`MUST-GATE-003` 判的是"**最大**样本绝对误差 `< 1e-6`"。抽样会把
"某个没被抽到的样本差了 1e-3"变成绿 —— 那正是本仓库反复警惕的假绿
（`docs/ledger/gate-status.md` 的元判据、`L12/D25` 的空转教训）。因此收据携带固定长度渲染的
**全部**样本位型；长度由夹具的 `frames` 决定（默认 8192 帧 ≈ 171 ms，约 300 KB）。

### 3.2 完整样例（**可解析**）

下面这份样例由本线交付的序列化器 `to_text` **真实输出**，并被 `parse` 成功读回；
但它的输入样本是脚手架合成的（本机不跑渲染，见 §8），因此 `digest` 与样本位型**不是真实渲染读数**。
真实收据只能由 `export_l1_receipt` 在 CI（或具备完整工具链的机器）上生成。

```text
yeban-l1-receipt v1
fixture reference-a
tracks 2
frames 3
channels 2
sample_rate 48000
block_size 128
threads auto
seed 24301
gain_db none
latency 00000000000000000000000001 0
latency 00000000000000000000000002 64
target_arch aarch64
target_os macos
target_env -
target_endian little
target_pointer_width 64
target_triple aarch64-apple-darwin
rustc_version 1.99.0 (b940084d7 2026-09-28)
digest 1111111111111111111111111111111111111111111111111111111111111111
longest_path_frames 64
pdc_expected_frames 64
sample_count 6
abs_max 5.00000000000000000e-1
sum_squares 3.32031250000000000e-1
class E ieee-exact
class T transcendental
op E cast@tone-integer-to-f32
op E div@tone-noise-normalize
op E sub@tone-center
op E mul@tone-carrier-scale
op E add@tone-mix
op E mul@edge-gain-apply
op E add@master-serial-reduction
op E copy@pdc-delay-line
sample 0 E 3f000000,be800000,3e000000,bd800000,00000000,80000000
```

（`00000000`/`80000000` 就是 `+0.0`/`-0.0` —— 位型表格的意义在此可见。）

真实夹具的收据会多出 `op T libm::powf@db-to-linear`（当 `--gain-db` 非 `none` 时），
样本行会有 `16384 / 16 = 1024` 行。

---

## 4. 比较器与三种判决

```text
cargo run --release -p yeban-render --example compare_l1_receipts -- <收据A> <收据B>
```

### 4.1 退出码（CI 语义）

| 码 | 含义 |
| :--- | :--- |
| `0` | 通过：`L1-bit-exact`（位级相同）或 `L2-within-budget`（差异只在超越函数类且落在预算内） |
| `1` | **`FAIL`**：IEEE 精确类出现差异 / 绝对误差 `>= 1e-6` / 超过 D32 的 4096 ulp 预算 |
| `2` | **无法判决**：收据缺失、格式错误、两份**不可比**（不是"通过"，也不是"样本超差"） |

报告是 `key=value` 文本，首行 `VERDICT <判决>`（便于 `grep`），但**判决只以退出码为准**。

### 4.2 判决顺序（每一步都对应一条规范）

1. `digest` 与逐样本位型**都**相同 ⇒ `L1-bit-exact`（同目标是 `MUST-GATE-002` 的读数；
   跨目标就是 `MUST-GATE-003` 的**最好情况**）；
2. 两者不一致（digest 同而样本异、或反之）⇒ 收据自相矛盾 ⇒ 退出码 2；
3. 任何 **`E`（IEEE 精确类）** 样本不同 ⇒ `FAIL`，`reason=ieee-exact-sample-differs`
   （**ADR-0001 D32：这类跨架构零容差**，哪怕只有 1 ulp）；
4. 最大绝对误差 `>= 1e-6` ⇒ `FAIL`，`reason=abs-budget-exceeded`（`MUST-GATE-003` 的预算）；
5. 最大 ulp 距离 `> 4096` ⇒ `FAIL`，`reason=d32-ulp-budget-exceeded`（D32 的 ulp 预算）；
6. 否则 ⇒ `L2-within-budget`（`reason=transcendental-only-within-budget`）。

"不可比"是**另一类**失败：`fixture`/形状/采样率/块长/种子/增益/延迟表/路径运算声明任一不同，
或两份收据的 `threads` 之外的指纹不一致，或收据自身不自洽 —— 此时任何结论都是假的，
因此返回退出码 2 而不是猜一个判决。

### 4.3 三种判决的真实报告（由本机脚手架**真跑**产生）

判决 1（两份相同 ⇒ 跨目标位级相同）：`VERDICT L1-bit-exact`，
`reason=digests-and-samples-identical`，`gate=MUST-GATE-003`，`same_target=false`。

判决 2（超越函数类上的 **1 ulp**）：

```text
VERDICT L2-within-budget
reason=transcendental-only-within-budget
gate=MUST-GATE-003
same_target=false
digest_equal=false
samples_equal=false
sample_count=4
max_abs_diff=1.49011611938476562e-8
max_abs_diff_index=2
max_abs_diff_class=T
max_ulp_diff=1
limit_abs=9.99999999999999955e-7
limit_ulp=4096
abs_budget_ok=true
ulp_budget_ok=true
ieee_exact_differences=0
transcendental_differences=1
offenders_total=1
offender index=2 class=T a=3e000000 b=3e000001 abs=1.49011611938476562e-8 ulp=1
```

判决 3（大幅偏差，**点名样本**；下为摘录，省略了指纹/线程/摘要等行）：

```text
VERDICT FAIL
reason=abs-budget-exceeded
gate=MUST-GATE-003
max_abs_diff=6.25000000000000000e-2
max_abs_diff_index=3
max_abs_diff_class=T
max_ulp_diff=8388608
abs_budget_ok=false
offender index=3 class=T a=bd800000 b=be000000 abs=6.25000000000000000e-2 ulp=8388608
```

### 4.4 另一条反向判据（D32 的"零容差"）

`E` 类样本上**只差 1 ulp** 时，报告是 `VERDICT FAIL` / `reason=ieee-exact-sample-differs`，
且 `abs_budget_ok=true` —— 红的原因是**类别**，不是幅度。这条判据是 D32 在比较器里的落点。

---

## 5. ADR-0001 D32 的分类如何体现

D32 的裁决是**按运算类别分策**：

1. **IEEE 精确类**（`add`/`sub`/`mul`/`div`/`sqrt`/`abs`/`min`/`max`/钳位/整数→`f32` 转换）：
   跨架构**零容差**；
2. **超越函数类**（`log`/`exp`/`powf`/`sin`/…）：给预算（D32 取 **4096 ulp**），
   另有 `MUST-GATE-003` 的**绝对**预算 `< 1e-6`。

收据把它落成三件事：

- **类别字母表**：`class E ieee-exact` + `class T transcendental` 无条件声明；
- **运声明**：`op <类别码> <运算@位置>` 逐条列出路径上的运算 —— 这就是"哪些运算是超越函数类"的
  显式声明。参考工程 A 的默认形态（`gain_db none`）**没有任何** `op T`，即"这条路径跨架构应当逐位相同"；
- **逐样本类别码**：`T` 表示**污染沿数据流传播** —— 哪怕最终那一步乘法是 IEEE 精确的，
  只要它的操作数来自 `powf`，这个样本就只受预算保护。参考工程 A 的 `gain_db != none` 形态
  因此**全部样本是 `T`**；默认形态**全部是 `E`**。

`gain_db` 刻意用具名 `EdgeGain`（`Identity` / `Db(f32)`）而不是 `Option<f32>`：`None`（不调 `powf`）
与 `Some(0.0)`（调 `powf` 但结果恰为 1.0）在收据里必须**可区分**，否则"路径上有没有超越函数"这件事
就被抹掉了 —— 而那正是 D32 分策的依据。

**已知的口径张力（登记为 needs）**：两条预算同时施加。若真实跨架构读数出现
"绝对预算通过、但 ulp 预算超（或反之）"，那是 D32 与 `MUST-GATE-003` 措辞之间的口径差异，
需要人类裁决（本线**不**擅自放宽任何一条）。

---

## 6. 跨架构怎么用（两条命令 + 门禁接线）

### 6.1 手动跑（任何具备完整工具链的机器）

```bash
# x86_64 runner
cargo run --release -p yeban-render --example export_l1_receipt -- --out receipt-x86.txt
# ubuntu-24.04-arm runner（两者的 --tracks/--frames/--gain-db/--latency 必须完全一致）
cargo run --release -p yeban-render --example export_l1_receipt -- --out receipt-arm.txt
# 比较（可在任意一台机器上做）
cargo run --release -p yeban-render --example compare_l1_receipts -- receipt-x86.txt receipt-arm.txt
echo $?   # 0 = 通过（L1-bit-exact 或 L2-within-budget）; 1 = FAIL; 2 = 无法判决
```

`--gain-db 3` 会把路径推到"超越函数类"，是**最可能给出 `L2-within-budget`** 的形态；
默认（`none`）是纯 IEEE 精确类形态，跨架构应当给出 `L1-bit-exact` —— 两者都值得各跑一次。

### 6.2 接进 CI（**由集成者做**，本线不改 `.github/**`）

建议在 `gates-manual.yml` 增加一个手动档（`workflow_dispatch` 输入 `arm`），
两腿分别 `runs-on: ubuntu-24.04` / `ubuntu-24.04-arm`，各自 `--out receipt-<arch>.txt`
并 `actions/upload-artifact`；然后在**任意一腿**（或第三个 job，跑在 x86_64 上就够，
比较器与架构无关）下载两份 artifact 并跑 `compare_l1_receipts`：

```yaml
- name: compare L1 receipts (MUST-GATE-003)
  run: |
    cargo run --release -p yeban-render --example compare_l1_receipts -- \
      receipt-x86.txt receipt-arm.txt
```

退出码即门禁结论（`1` = 超预算，`2` = 收据缺失/不可比）。**不要把输出管道给 `tail`/`head`**
（本仓 L6 纪律：管道会吞掉退出码）。建议同时把报告写进 job summary，
并保留两份收据 artifact 作为**证据**（`gate-status.md` 的"非 PENDING 必须有证据"要求）。

自动档（`ci.yml`）不建议默认加 ARM 腿：ARM runner 的排队/费用与"每次 push 都跑"不成比例。
本线**没有**修改 `ci.yml`（它是集成者的文件）。

---

## 7. 判据清单（并明确哪些本机跑过）

### 7.1 本机真跑：`rustc --edition 2024 --test`（18 条，零重依赖）

```bash
rustc --edition 2024 --test -D warnings -W missing_docs \
  crates/yeban-render/examples/support/l1_receipt_tests.rs -o /tmp/l1-receipt-tests && /tmp/l1-receipt-tests
```

| # | 判据 | 测试名 |
| :--- | :--- | :--- |
| 1 | 序列化 ↔ 解析往返**逐字节**无损 | `round_trip_is_byte_exact` |
| 2 | 极端位型往返（`-0.0`、最小次正规、最大规格化、±1、1e±30） | `extreme_bit_patterns_round_trip` |
| 3 | 同一输入两次导出 ⇒ 文本逐字节相同（确定性，判据 ①） | `two_exports_of_the_same_input_are_byte_identical` |
| 4 | 收据含**非空**的目标三元组与架构（判据 ②） | `receipt_carries_a_non_empty_target_identity` |
| 5 | 完全相同 ⇒ `L1-bit-exact`（判据 ③） | `identical_receipts_are_l1_bit_exact` |
| 6 | 超越类 1 ulp ⇒ `L2-within-budget` 且点名样本（判据 ④） | `one_ulp_on_a_transcendental_sample_is_within_budget` |
| 7 | 大幅偏差 ⇒ `FAIL` 且**点名样本**（判据 ⑤） | `a_large_deviation_fails_and_names_the_sample` |
| 8 | **IEEE 精确类上 1 ulp 也必须 `FAIL`**（D32 零容差） | `one_ulp_on_an_ieee_exact_sample_fails` |
| 9 | 绝对预算通过但 ulp 超预算 ⇒ 仍 `FAIL` | `d32_ulp_budget_fails_even_when_the_absolute_budget_passes` |
| 10 | **线程数不同不导致不可比**（判据 ⑦ 的纯逻辑形态，`ARCH-DET-002`） | `thread_count_does_not_make_receipts_incomparable` |
| 11 | 字段缺失 ⇒ 明确错误（不是 panic，判据 ⑥） | `missing_required_fields_are_clear_errors` |
| 12 | 格式错误（首行/未知字段/坏十六进制/未知类别码/坏数字）⇒ 明确错误 | `malformed_receipts_are_errors_not_panics` |
| 13 | **抽样/截断**的收据被拒（防"最大误差"假绿） | `sampled_or_truncated_receipts_are_refused` |
| 14 | PDC 读数不自洽 ⇒ 收据被拒（判据 ⑧） | `pdc_inconsistency_is_refused` |
| 15 | 指纹不同 ⇒ **不可比**（退出码 2 的语义） | `fingerprint_mismatch_is_incomparable` |
| 16 | `digest` 与样本表互相矛盾 ⇒ 不可比 | `digest_sample_contradiction_is_incomparable` |
| 17 | 有 `T` 样本却无 `op T` 声明 ⇒ 收据被拒 | `transcendental_sample_without_declaration_is_refused` |
| 18 | 报告首行机器可读 + 失败点名越界样本 | `report_is_machine_readable_and_names_offenders` |

结果：**18 passed; 0 failed**。另跑工作区 `[lints]` 的逐条等价集合：

```bash
clippy-driver --edition 2024 --test -D warnings -D clippy::all -D clippy::dbg_macro \
  -D clippy::undocumented_unsafe_blocks -D rust_2018_idioms -W missing_docs \
  crates/yeban-render/examples/support/l1_receipt_tests.rs -o /tmp/l1-receipt-clippy
```

结果：**0 告警**。

### 7.2 只由 CI 判定：`tests/l1_digest_contract.rs`（10 条，真渲染）

`yeban-render` 含 `rayon` ⇒ `run-gates.sh crate yeban-render` 在本机 **SKIP**（设计如此）。
这 10 条要求真的调用 `RenderPlan::execute`，因此**本机从未执行过**：

| # | 判据 | 测试名 |
| :--- | :--- | :--- |
| 1 | 同一台机器两次**真实**导出 ⇒ 收据逐字节相同（判据 ① 的真实形态） | `two_real_exports_are_byte_identical` |
| 2 | 收据自证架构：`target_arch == std::env::consts::ARCH`，`target_triple == rustc -vV` 的 host（判据 ②） | `receipt_self_identifies_the_target` |
| 3 | **线程数 1/2/4 ⇒ digest 相同**，判决 `L1-bit-exact` 且 `gate=MUST-GATE-002`（判据 ⑦，`ARCH-DET-002`） | `thread_counts_do_not_change_the_digest` |
| 4 | 收据的 digest / 统计量 / 样本位型与 `RenderOutput` **逐位一致**（读数不是编的） | `receipt_readings_come_from_the_render_output` |
| 5 | `longest_path_frames == 注入的 PDC L_max == pdc_expected_frames`（判据 ⑧） | `longest_path_frames_matches_the_injected_l_max` |
| 6 | 注入延迟**真的改变了音频**（否则判据 5 是空转） | `latency_injection_actually_changes_the_audio` |
| 7 | `gain_db none` ⇒ 全部样本 `E` 且无 `op T` 声明（D32 分类如实） | `ieee_only_fixture_declares_no_transcendental_ops` |
| 8 | `gain_db 3` ⇒ 全部样本 `T` 且有 `op T` 声明 | `gain_fixture_declares_transcendental_ops` |
| 9 | 真实夹具上超越类的 1 ulp 扰动 ⇒ `L2-within-budget`（`MUST-GATE-003` 的可达形态） | `a_one_ulp_transcendental_perturbation_lands_in_the_l2_budget` |
| 10 | 收据文本往返 = 模型相等；指纹不同 ⇒ 明确不可比 | `receipt_text_round_trips` |

**CI 实测**：run `37242814049` 的 `rust (yeban-render)` 步 `clippy (-D warnings)` 与 `test`
**双双成功** ⇒ 这 10 条判据在真实工具链（`rayon` + 真渲染）上通过。逐条输出未读到，原因见 §12。

---

## 8. 本机真跑 / 编译级检查 / CI 的**严格区分**

| 类别 | 内容 | 是否证据 |
| :--- | :--- | :--- |
| ✅ **本机真跑** | §7.1 的 18 条判据 + clippy-driver 零告警 + `run-gates.sh light` 绿 | 是（但只覆盖纯逻辑） |
| ⚠️ **本机编译级检查（不是证据）** | 为了让"本机不编译重依赖"与"尽早发现类型错误"不冲突，本线用**签名照抄真实源码的 `yeban_model` / `yeban_render` 替身**，把 `export_l1_receipt.rs`、`compare_l1_receipts.rs`、`bench_render.rs`、`tests/l1_digest_contract.rs` 在本机做了 `rustc -D warnings` 与 `clippy-driver -D clippy::all` 的**编译级**检查（全绿）。替身不是真实现（它不排序、不做 L1 块调度），**因此这不是渲染证据**，只降低"CI 因类型错误变红"的概率 | **否** |
| ❌ **本机未执行** | 任何 `RenderPlan::execute`（真实渲染）、`--workspace`、benchmark、fuzz | — |
| 🎯 **只有 CI 算数** | §7.2 的 10 条 + §10 的判决 | 是 |

---

## 9. 注入 → 变红 → 还原（3 条判据的判别力自证）

注入在 `/tmp` 的**独立副本**里做（不动仓库）。每条注入先断言"锚点恰好命中 1 次"
（否则就是账本 L3 的"注入没生效"），再"改坏 → 确认红 → 还原 → 确认绿"。

| # | 注入 | 预期变红的判据 | 实测 |
| :--- | :--- | :--- | :--- |
| I1 | 删掉 `judge` 的 **IEEE 精确类零容差**分支（D32 失效） | `one_ulp_on_an_ieee_exact_sample_fails` | **红 ✓**（16 passed / 2 failed；连带 `report_...` 也红，因为 `reason` 变了） |
| I2 | 绝对预算 `1e-6` → `1.0`（放过大偏差） | `a_large_deviation_fails_and_names_the_sample` | **红 ✓**（16 passed / 2 failed） |
| I3 | 删掉 `validate` 的 **PDC 自洽检查**（判据 ⑧ 失去判别力） | `pdc_inconsistency_is_refused` | **红 ✓**（17 passed / 1 failed） |

三次注入后均**还原为 18 passed / 0 failed**。锚点命中次数与失败清单见本线的提交说明与报告。

---

## 10. 未实现 / 边界（本线**没有**证明什么）

- **没有**任何真实跨架构对账结果：本机只有 aarch64-apple-darwin 一份读数（且本机不跑渲染），
  因此 `MUST-GATE-003` 在拿到两份**真实**收据之前**仍然是 `PENDING`**。
- 收据的样本表是"全量、无压缩"的文本：默认 8192 帧约 300 KB；`--frames 1440000`（30 秒）约 37 MB。
  没有实现二进制/压缩载体（"自包含、可 git diff"的优先级更高）。
- 收据的类别是**每样本一个字母**（`E`/`T`），没有区分"某个样本受几个超越函数影响"。
  对当前夹具（统一路径）足够；更细的传播分析未实现。
- 参考工程 A 仍是 `bench_render` 的合成夹具（注入式 `AudioSource`），**不是**真实乐器/效果链。
- 没有实现"把两份收据的差异进一步定位到某个轨道/某个块"的诊断（只有样本下标与位型）。
- 比较器的 ulp 预算按 `f32` 位型单调序计算；对 NaN 载荷未定义（收据本身已拒绝非有限样本）。

---

## 11. needs / pending

| 类型 | 条目 | 说明 |
| :--- | :--- | :--- |
| **needs（阻塞式）** | **ARM 手动档门禁** | 请集成者在 `gates-manual.yml` 增加 `arm` 输入/腿（`runs-on: ubuntu-24.04-arm`），两腿各产出一份收据 artifact，再跑 `compare_l1_receipts`。**这是让 `MUST-GATE-003` 从 PENDING 变成有证据的唯一路径**；本线不改 `.github/**` |
| **needs** | 更新 `docs/ledger/gate-status.md` 的 `MUST-GATE-002` / `MUST-GATE-003` 行 | 该表由集成者独占。建议：`003` 在拿到两份真实收据前保持 `PENDING`，但把"缺什么"改成"缺两份收据 + 一条 arm 腿"（本线已交付格式与比较器）；`002` 可补上 `tests/l1_digest_contract.rs` 的线程不变性/确定性判据作为证据 |
| **needs** | D32 的 4096 ulp 与 `MUST-GATE-003` 的 `1e-6` 口径若冲突 | 比较器**同时**施加两条预算（更严）。若真实读数落在两者之间，需人类裁决以哪条为准 |
| pending | 把收据格式模块提升为公共 API | 目前它在 `examples/support/`（任务书只允许在 `examples/**`、`tests/**` 新建文件）。若集成者希望 `yeban-mcp`/工具链复用，应提升为 `crates/yeban-render/src/l1_receipt.rs` 并在 `lib.rs` 声明（**本线刻意未动 `src/`**，避免与其它线争抢同一个文件） |
| pending | `bench_render` 的 `BENCH` 行加 `fixture`/`gain_db` 字段 | 现在 `bench_render` 与收据共用夹具，但基准行没有标注夹具版本；可选增强 |
| 记录（不是待办） | `MUST-GATE-002` 的"跨机器 SHA-256 全同"仍未做成门禁 | 收据给出了可比较的载体；同目标两份收据相同 ⇒ `L1-bit-exact`，这正是 `002` 的跨机器形态 |

---

## 12. CI 判决

### 第 1 轮（代码提交 `80dc87d`）—— run [37242814049](https://github.com/gradetwo/yeban/actions/runs/37242814049)：**全绿 ✅**

| job | 结论 | 对本线的意义 |
| :--- | :--- | :--- |
| `plan (受影响集合)` | ✅ 4s | 受影响集合正确推导为只含 `yeban-render` |
| `checks (fmt / 红线守卫 / schema)` | ✅ 1m46s | **格式检查**（`cargo fmt --all --check`）通过 ⇒ 新增的 6 个文件（含 `examples/support/**` 的非目标文件）都是 rustfmt 规范的；红线守卫、文档链接、许可清单漂移也都通过（**没有新增依赖**） |
| `lockfile (确定性 Cargo.lock)` | ✅ 19s | 未改任何清单 ⇒ 锁文件无漂移 |
| `deny (cargo-deny 开源合规)` | ✅ 49s | 无新依赖 ⇒ 无新许可 |
| `rust (yeban-render)` | ✅ 55s | **`clippy (-D warnings)` 成功 + `test` 成功** —— 这是 §7.2 那 10 条**真渲染**判据（含线程不变性、PDC `L_max`、逐位读数、E/T 分类、超越类 1 ulp 落 L2 预算）的判决所在 |
| `rust (workspace 全量)` / `windows (...)` | skipped | 计划器的动态腿，本线不涉及 |

**证据的边界（如实登记）**：本机没有 `GH_TOKEN`，`ci-verdict.sh` 走匿名 REST API，
只能读到 **job/step 级结论**（上表），读不到原始日志。因此"`test` 步成功"是本轮判据的
**结论性证据**，但测试用例的逐条输出（多少个 passed）**未被本线读到**。
如需逐条输出，请配置 `GH_TOKEN` 后跑 `scripts/dev/ci-verdict.sh --logs 37242814049`。

**这一轮证明的**：收据导出器/比较器/CI 判据**在真实工具链上编译零告警并全部通过**。
**这一轮没有证明的**：任何**跨架构**读数 —— `MUST-GATE-003` 需要两份不同 `target_triple`
的真实收据，那要等 §11 的 `arm` 腿落地。

### 第 2 轮（文档提交 `6982828`）—— run [37243008800](https://github.com/gradetwo/yeban/actions/runs/37243008800)：**全绿 ✅**

纯 `docs/**` 变更：`plan` ✅5s / `lockfile` ✅17s / `deny` ✅40s / `checks` ✅38s；
`rust (...)` / `windows (...)` 三条腿被计划器正确 **skipped**（没有受影响 crate）。
`checks` 通过即证明本文件（`docs/ledger/l1-digest-notes.md`）没有引入坏链接、且 fmt 仍然干净。

> **记账纪律**：本文件随文档提交一起推送，而每次推送都会触发新的 run ——
> 若要求"把每一次判决都回写进本文件"，就会变成"回写→推送→新 run→再回写"的无限循环。
> 因此本线在第 2 轮之后**停止回写**：第 2 轮之后的那次推送（第 3 轮，仍是纯文档记账）
> 的判决由本线的最终报告给出，不回写到这里。

---

## 13. 修改文件的绝对路径清单

```text
/Users/crow/work/music/yeban/.worktrees/l1-digest/crates/yeban-render/examples/bench_render.rs          (已改：引用共用夹具)
/Users/crow/work/music/yeban/.worktrees/l1-digest/crates/yeban-render/examples/export_l1_receipt.rs     (新)
/Users/crow/work/music/yeban/.worktrees/l1-digest/crates/yeban-render/examples/compare_l1_receipts.rs   (新)
/Users/crow/work/music/yeban/.worktrees/l1-digest/crates/yeban-render/examples/support/l1_receipt.rs    (新)
/Users/crow/work/music/yeban/.worktrees/l1-digest/crates/yeban-render/examples/support/reference_project_a.rs (新)
/Users/crow/work/music/yeban/.worktrees/l1-digest/crates/yeban-render/examples/support/export_pipeline.rs     (新)
/Users/crow/work/music/yeban/.worktrees/l1-digest/crates/yeban-render/examples/support/l1_receipt_tests.rs    (新，非 cargo 目标)
/Users/crow/work/music/yeban/.worktrees/l1-digest/crates/yeban-render/tests/l1_digest_contract.rs        (新)
/Users/crow/work/music/yeban/.worktrees/l1-digest/docs/ledger/l1-digest-notes.md                        (新，本文件)
```

未触碰：根 `Cargo.toml`、`Cargo.lock`、`.github/**`、`scripts/**`、`deny.toml`、
`docs/DEVELOPMENT_LEDGER.md`、`docs/adr/**`、`docs/YEBAN_*.md`、`schemas/**`、其它 `crates/**`、`spikes/**`、法务文件。
