# sfz-core 工作线笔记（口径 / 来源 / 缺口）

> 工作线: `line/sfz-core` · 工作树 `.worktrees/sfz-core` · 拥有目录 `crates/yeban-sfz/**`
> 规范 ID: **ROAD-M2-005**（零拷贝 SFZ v2 解析器 + 预分配声部池）、**ROAD-M2-006**（声学自愈平滑的
> 池侧口径）、**ARCH-RT-001**（RT 零分配）、**ARCH-RT-004**（确定性声部窃取 + 3ms 淡出）、
> **ARCH-DET-001**（确定性，禁随机）、**MUST-GATE-011**（格式解析零崩溃）。
> 本文件是 `docs/ledger/` 下的**工作线附件**，不是 Normative 规范；与规范冲突时以规范为准。

---

## 1. 交付物与规范 ID 映射

| 文件 | 内容 | 规范 ID |
| :--- | :--- | :--- |
| [`crates/yeban-sfz/src/error.rs`](../../crates/yeban-sfz/src/error.rs) | 统一错误类型，解析路径零 panic | MUST-GATE-011, ROAD-M2-005 |
| [`crates/yeban-sfz/src/parser.rs`](../../crates/yeban-sfz/src/parser.rs) | 单遍零拷贝词法/结构解析、`ParseLimits`、`#include` 沙箱 + glob、`#define` 文本层、类型化读取 | ROAD-M2-005, ARCH-RT-001 |
| [`crates/yeban-sfz/src/instrument.rs`](../../crates/yeban-sfz/src/instrument.rs) | `<region>` 归约、`region_for` / `region_for_with`、确定性轮替 | ROAD-M2-005, ARCH-DET-001 |
| [`crates/yeban-sfz/src/voice_pool.rs`](../../crates/yeban-sfz/src/voice_pool.rs) | 固定容量预分配声部池、确定性窃取、3ms 指数淡出 | ARCH-RT-001, ARCH-RT-004 |
| [`crates/yeban-sfz/tests/include_sandbox.rs`](../../crates/yeban-sfz/tests/include_sandbox.rs) | 沙箱安全红线集成测试 | ROAD-M2-005 |
| [`crates/yeban-sfz/tests/malformed_inputs.rs`](../../crates/yeban-sfz/tests/malformed_inputs.rs) | 畸形输入不 panic / 显式上限 / 确定性 | MUST-GATE-011 |
| [`crates/yeban-sfz/tests/support/mod.rs`](../../crates/yeban-sfz/tests/support/mod.rs) | std-only 临时目录夹具（不引入 dev-dependency） | — |
| [`crates/yeban-sfz/fuzz/fuzz_targets/sfz_parse.rs`](../../crates/yeban-sfz/fuzz/fuzz_targets/sfz_parse.rs) | cargo-fuzz 目标 `sfz_parse`（本机未执行） | MUST-GATE-011 |

依赖纪律：**运行时依赖只有 `thiserror`**（已在根 `[workspace.dependencies]` 登记），
**没有任何 dev-dependency** —— 测试夹具用 std-only 的
[`crates/yeban-sfz/tests/support/mod.rs`](../../crates/yeban-sfz/tests/support/mod.rs)
（`std::env::temp_dir()` + 进程号 + 原子计数器）替代 `tempfile`，以守住「本机可随便编译测试」
这条优势。因此**没有 `TODO(hoist)`**。

---

## 2. 已核验的 SFZ 格式事实与出处

| 事实 | 出处 |
| :--- | :--- |
| 段头集合与版本：`<region>`/`<group>` SFZ v1；`<control>`/`<global>`/`<curve>`/`<effect>` SFZ v2；`<master>`/`<midi>` 为 ARIA 扩展 | <https://sfzformat.com/headers/> |
| `#include` 路径**必须**双引号包围；被包含文件在**出现位置**被粘贴；相对**主 SFZ 文件**路径解析；可嵌套但**严禁递归成环**；扩展名 `.sfz` / `.sfzh` | <https://sfzformat.com/opcodes/include/> |
| `#define $VAR value`；变量名以 `$` 开头；同一变量在文件不同位置重复定义在 ARIA 下行为不可靠 | <https://sfzformat.com/opcodes/define/> |
| `key=k` 等价于 `lokey=hikey=pitch_keycenter=k`；**`pitch_keycenter` 显式值永远优先**（ARIA）；`key=-1` 表示不被音符触发；`key=c5` ≡ `key=72` ⇒ **`C4 = 60`（IPN）** | <https://sfzformat.com/opcodes/key/> |
| `loop_mode` 取值 `no_loop` / `one_shot` / `loop_continuous` / `loop_sustain`；缺省取决于采样文件是否带 loop 元数据；`loopstart`/`loopend` 是 `loop_start`/`loop_end` 的别名 | <https://sfzformat.com/opcodes/loop_mode/> |
| `default_path` 只在 `<control>` 下；ARIA 下新的 `<control>` 会**重置** `default_path`；与 `sample` 拼接时要注意斜杠 | <https://sfzformat.com/opcodes/default_path/> |
| `seq_length` 默认 1（范围 1..100），`seq_position` 默认 1；播放器维护「从 1 开始、到 `seq_length` 归零」的内部计数器 | <https://sfzformat.com/opcodes/seq_length/> |
| 机器可读的 opcode 默认值/范围表（本次核验用了它）：`pitch_keycenter` 默认 60、`lokey` 0、`hikey` 127、`lovel` 0、`hivel` 127、`lochan` 1、`hichan` 16、`tune` 0（±100 cents）、`transpose` 0（±127）、`volume` 0 dB（-144..6）、`pan` 0 %（±100）、`seq_position`/`seq_length` 1、`off_time` 默认 **0.006 s**（voice-stealing 淡出，ARIA） | `_data/sfz/syntax.yml` @ <https://github.com/jlearman/sfzformat.github.io> |
| 「一个 opcode 的取值应解析到空白，而不是行尾」—— 说明现实播放器**允许一行多个 opcode**，且各家解析不一致 | <https://github.com/andamira/sofiza/issues/2> |
| `#include` 的**通配符/递归预载**只出现在 sfizz 社区讨论里，**不在规范正文**（见第 6 节「需要人类裁决」） | <https://github.com/sfztools/sfizz/discussions/1210> |

历史项目 `groove/src/audio/sfz/**`（TypeScript, 2,979 行）按
[`docs/ledger/legacy-reuse-audit.md`](legacy-reuse-audit.md) 的结论**不复用代码**：本次只借鉴其
keyswitch / include / define / CC gate / 轮替的**语义**，全部用 Rust 重写。

---

## 3. 零拷贝与「一行多 opcode」的口径

- **零拷贝**：无 `$VAR` 替换时，opcode 名与取值都以 `Cow::Borrowed` 借用调用方的源缓冲
  （`Instrument<'a>` / `Region<'a>` 直接借用 `Vec<SfzSource>`）。只有真的发生宏替换时才
  `Cow::Owned`。`parse_text` / `parse_sources` 都**不做任何 I/O**。
- **include 的两步 API**：`IncludeResolver::resolve` 先做 I/O 并把 `#include` 在**出现位置**切成
  `SfzSource` 片段序列，再由 `parse_sources(&sources, …)` 借用解析。这样既保持核心解析器是纯函数、
  可 fuzz、可重入，又符合规范「在该点粘贴」的语义（`<region>` 可以跨片段延续）。
  代价：**调用方必须让 `Vec<SfzSource>` 活得比 `Instrument<'_>` 久**（见第 6 节 needs）。
- **一行多 opcode**：取值从 `=` 之后延伸到「空白 + 标识符 + `=`」出现处，因此
  `sample=My Drums/kick.wav key=36` 会切出两个 opcode，且保留文件名里的空格。
  这是对规范模糊处的**工程裁决**：规范只说「解析到空白」（见上表），
  文件名含 ` key=` 字面量的文件会被切错 —— 记为已知限制。

---

## 4. 显式上限口径（DoS 防线）

实现常量在 [`parser.rs`](../../crates/yeban-sfz/src/parser.rs) 顶部；**改这里必须同步改本表**。

| 上限 | 默认值 | 触发的错误 | 为什么需要 |
| :--- | ---: | :--- | :--- |
| `max_line_bytes` | 64 KiB | `LineTooLong` | 一行 16 MiB 无换行是典型 DoS |
| `max_regions` | 65 536 | `TooManyRegions` | region 数量线性放大内存 |
| `max_opcodes_per_header` | 4 096 | `TooManyOpcodes` | 攻击者可无限造不同 opcode 名 |
| `max_defines` | 4 096 | `TooManyDefines` | 宏表无界增长 |
| `max_macro_expansions_per_line` | 64 | `MacroExpansionExceeded` | `#define` 文本炸弹 |
| `max_source_bytes` | 16 MiB | `SourceTooLarge` | 单文件读入内存前先拦 |
| `max_include_depth` | 16 | `IncludeDepthExceeded` | 深链 include |
| `max_include_files` | 1 024 | `IncludeCountExceeded` | include 扇出 |
| `max_glob_matches` | 4 096 | `GlobMatchesExceeded` | `*.sfz` 匹配爆炸 |
| `max_glob_depth` | 16 | （静默截断） | 目录树深度 |
| `max_glob_scanned` | 65 536 | `GlobScanExceeded` | 目录项遍历炸弹 |
| `max_warnings` | 256 | `Warning::Truncated` | 恶意输入刷警告 |

`ParseLimits::unlimited()` 仅供测试；生产代码不得使用。

---

## 5. 已实现 vs 明确未实现

**已实现**：`<control>`/`<global>`/`<group>`/`<region>` 段头（含「段头后同行跟 opcode」）、
三级作用域继承（region → group → global）、`//` 行注释（引号内不截断）、`opcode=value`
（一行多个）、取值去引号、`#define $VAR`（定义时展开一层 + 使用处单遍替换）、`#include`
（相对路径 + glob + 沙箱 + 环/深度/数量上限）、`key`/`lokey`/`hikey`/`pitch_keycenter`/
`lovel`/`hivel`/`lochan`/`hichan`/`loop_start`/`loop_end`/`loop_mode`/`tune`/`transpose`/
`volume`(=`gain`)/`pan`/`seq_position`/`seq_length`/`group`(=`polyphony_group`)/`off_by`(=`offby`)、
`sw_last`/`sw_lokey`/`sw_hikey`/`sw_down`/`sw_up`、`loccN`/`hiccN` CC 门控、
`default_path` 拼接，以及固定容量声部池 + 确定性窃取 + 3ms 指数淡出。

**明确未实现**（都记在这里，不静默降级）：

1. **未建模的段头**：`<master>`(ARIA)、`<curve>`、`<effect>`、`<midi>`、`<sample>`。识别到即产生
   `Warning::IgnoredHeader` 并**丢弃其 opcode**，绝不当作 region。因此用 `<master>` 做分层映射的
   商业音色库会**丢失 master 级 opcode**（这是本切片最大的兼容性缺口）。
2. **合成/调制类 opcode**：`ampeg_*`/`fileg_*`/`pitcheg_*`、滤波（`cutoff`/`resonance`）、
   LFO、`egN_*` 等一律忽略（引擎侧后续切片）。
3. **`trigger` / `off_mode` / `off_time` / `loop_count` / `direction` / `offset` / `count`** 未建模；
   特别地：规范里 `off_time` 默认 **0.006 s**，而 `ARCH-RT-004` 规定窃取淡出 **3 ms** ——
   本实现按 Normative 的 3 ms，且不读 per-region `off_time`（见第 6 节冲突项）。
4. **随机轮替** `lorand`/`hirand`：**故意不实现**，因为 `ARCH-DET-001` 禁止随机数。
5. **参数化宏** `#define $F(a) …`：未实现（规范正文未定义，记为缺口）。
6. **递归宏展开**：只做「定义时一层 + 使用处单遍」，不做递归 —— 这是为了杜绝
   `#define $A $A` 类的爆炸与死循环。
7. **`loop_mode` 缺省**：规范说缺省取决于采样文件是否带 loop 元数据；本 crate 不解码音频，
   因此**一律缺省 `LoopMode::NoLoop`**。
8. **`note_offset`/`octave_offset`/`set_ccN`/`sw_note_offset`** 未实现（CC 门控只读 `loccN`/`hiccN`）。
9. **段头只能出现在行首**（其后可紧跟 opcode）。`…<region>…` 出现在取值中间时会被当成取值的一部分。
10. **音名八度**固定 IPN（`C4 = 60`）；不提供 Yamaha/`C3 = 60` 兼容开关（见第 6 节）。

---

## 6. 需要人类裁决的歧义 / 缺口

1. **音名八度记法**：sfzformat 的 `key` 页明确 `key=c5` ≡ `key=72`，故本实现取 `C4 = 60`（IPN）。
   但 AKAI 系与部分老音色库沿用 `C3 = 60`（Yamaha）。是否需要「兼容模式」开关，或是否只认 MIDI 号？
2. **`#include` 的 glob（`*`/`?`/`**`）不是规范正文的一部分**：任务要求支持，本实现已支持
   （绝对安全：只在沙箱内遍历、不跟随符号链接、结果排序去重）。请裁决这是否要写进 Normative
   规范，还是标注为「夜半扩展」。
3. **固定容量池下的 3ms 淡出语义**：池没有 slack 槽位同时容纳「正在淡出的旧声部」与「新声部」。
   本实现选择**立即接管**：`note_on` 超限时把 victim 槽位直接给新声部，并把 victim 句柄返回给
   渲染器，由渲染器用 `StealFade` 对 victim 残余输出做 3ms 指数淡出。另一条路「先淡出再复用」
   会把新音符延后 ≤3 ms。请裁决：哪个是 `ARCH-RT-004` 的本意？（本实现两条测试都覆盖：
   立即接管 + `retire`→`process` 归零复用。）
4. **`off_time`（SFZ 默认 6 ms） vs `ARCH-RT-004`（3 ms）** 的冲突：本实现按架构文档 3 ms。
   是否要在 SFZ 路径上尊重 per-region `off_time`？
5. **轮替计数器的作用域**：规范说「每个 region 维护自己的内部计数器」，而现实中同一
   `seq_length` 组内多个 region 共享一个计数器。本实现把「第几次触发」`occurrence: u64` 作为
   **调用方输入**，组长度取「第一个完整匹配 region 的 `seq_length`」，再选
   `seq_position == (occurrence % L) + 1`。请裁决：引擎侧计数器应按「(note, velocity) 组」还是
   「region」维护。
6. **`SfzSource` 生命周期**：`IncludeResolver::resolve → Vec<SfzSource>` 与
   `parse_sources(&sources) → Instrument<'_>` 是两步 API，引擎必须让 `sources` 活得比
   `Instrument` 久。真正的「自持有」包装（`LoadedInstrument { sources, instrument }`）在安全
   Rust 里做不到（需要自引用类型 + unsafe 或 ouroboros 依赖）。请裁决引擎侧的持有方案：
   ① 引擎持有 `Vec<SfzSource>` 并在加载期重建 region 索引；② region 模型改为「只存索引 + 拥有
   字符串池」；③ 引入自引用库（需要评估依赖预算与 `forbid(unsafe_code)` 冲突）。
7. **`<master>` 支持**：本切片忽略 `<master>`，会丢 opcode。是否需要在下一切片补
   `global → master → group → region` 四级继承？
8. **一行多 opcode 的切分规则**是工程裁决（见第 3 节）。若必须与某个具体播放器 bit-for-bit 对齐，
   请指定「对齐目标」（sfizz / ARIA / Cakewalk），因为它们的规则互相不同。

---

## 7. 如何在 CI 手动档跑 fuzz（MUST-GATE-011）

本机（Apple M2）**不跑** fuzz（用户硬性纪律）。fuzz 目标已就位但**尚未执行**，
因此 `MUST-GATE-011` 记账为 **pending**。手动档步骤：

```bash
# 需要 nightly 工具链 + cargo-fuzz（CI 上可用 taiki-e/install-action 或 cargo install）
cd crates/yeban-sfz
cargo +nightly fuzz run sfz_parse -- -max_total_time=900 -rss_limit_mb=2048
# MUST-GATE-011 的「千万次变异」口径：
cargo +nightly fuzz run sfz_parse -- -runs=10000000 -max_len=4096 -rss_limit_mb=2048
```

重复崩溃的复现：`cargo +nightly fuzz tmin sfz_parse artifacts/sfz_parse/crash-*`。

**needs（集成者执行）**：`.github/workflows/gates-manual.yml` 需要新增一个 `workflow_dispatch`
job 来跑上面的命令。`.github/**` 属于集成者独占文件，本条工作线**不能**修改它，
所以这里只给出可复制的命令。本地可先跑的替代判据是
`tests/malformed_inputs.rs` 里的固定畸形语料 + 两路确定性伪随机输入（共 8 000 例，纯 CPU 毫秒级）。

---

## 8. 本机判据结果与「注入 → 变红」记录

判据纪律（SKILL 规则 2）：**从没红过的判据是穿着测试外衣的注释**。下面每条判据都做了
「故意改坏实现 → 观察变红 → 改回」的注入。全部注入已还原，工作区无残留（`grep INJECTED` 为空）。

| # | 判据（测试名） | 注入的坏改动 | 结果 |
| :-- | :--- | :--- | :--- |
| 1 | `absolute_include_is_rejected` | 关掉 `check_relative` 里的绝对路径拦截 | **红**（错误从 `IncludeAbsolutePath` 退化成 `IncludeNotFound`，且 `Path::join` 会把绝对路径接管） |
| 2 | `symlink_escape_is_rejected_even_though_the_path_looks_relative` | 关掉 `canonical_include` 的 `starts_with(base)` 断言 | **红**（真的读到了基准目录外的 `secret.wav`） |
| 3 | `parent_directory_escape_is_rejected` | 关掉 `..` 词法拦截（单独关 → 仍绿，被规范化断言兜住） → 再关掉规范化断言 | **红**（读到 `../outside.sfz`）。说明两层互为纵深防御，只有两层都坏才漏 |
| 4 | `oversized_line_is_rejected` | 关掉 `max_line_bytes` 检查 | **红** |
| 5 | `too_many_regions_hits_the_explicit_limit` | 关掉 `max_regions` 检查 | **红** |
| 6 | `round_robin_selection_is_deterministic_and_cycles` | 把 `target` 写死为 1（忽略 `occurrence`） | **红**（第 2 次触发返回 k1 而非 k2） |
| 7 | `capacity_is_never_exceeded` | 池满时 `slots.push` 而不是窃取 | **红**（容量从 8 涨到 200） |
| 8 | `fixed_malformed_corpus_never_panics` | 在 `parse_text` 里对 `sample=` 输入 `panic!` | **红** |

绿色基线（还原后，`cargo test -p yeban-sfz`）：**25 lib + 16 include_sandbox + 12 malformed_inputs
+ 3 doc-tests = 56 全绿**；`clippy -p yeban-sfz --all-targets -- -D warnings` 零告警。

判据 → 报告里承诺的 5 条 (a)…(e) 的对应关系：

- (a) `#include` 逃逸必须被拒绝 → include_sandbox 的 absolute / parent / symlink 三条（注入 1–3）
- (b) 恶意超长 / 超多 region 触发显式上限而不是 OOM → `oversized_line_is_rejected`、
  `too_many_regions_hits_the_explicit_limit`、`too_many_opcodes_hits_the_explicit_limit`、
  `too_many_defines_hits_the_explicit_limit`、`oversized_source_file_hits_the_explicit_limit`、
  `include_depth_limit_is_enforced`、`include_file_count_limit_is_enforced`（注入 4–5）
- (c) 轮替选择确定性 → `round_robin_selection_is_deterministic_and_cycles`、
  `round_robin_is_deterministic_across_independent_parses`、
  `include_resolution_is_deterministic_across_runs`（注入 6）
- (d) 声部池容量不被突破 → `capacity_is_never_exceeded`（注入 7）；
  另有 `steal_order_is_deterministic_across_runs`、`retire_fade_zeroes_the_slot_and_makes_it_reusable`、
  `steal_fade_envelope_decays_exponentially_to_silence`
- (e) 任意字节输入不 panic → `fixed_malformed_corpus_never_panics`（注入 8）、
  `random_bytes_never_panic`、`structured_random_input_never_panics_and_is_deterministic`；
  cargo-fuzz 目标另计（pending）

### 未证明的部分（诚实记账）

- **「`process`/`note_on` 路径零分配」没有用分配器计数机械证明**。代码结构上保证
  `Vec<Slot>` 长度恒定、REC 路径只做标量字段赋值与 `saturating_sub`，且 `capacity()` 在
  200 次 `note_on` 后仍为 8（间接证据）。真正的证明需要一个 `unsafe impl GlobalAlloc` 计数测试
  或 `dhat`/`allocation-counter` 类依赖：前者与本 crate 的 safe-by-default 取向冲突，
  后者要动根 `[workspace.dependencies]`。**pending**，留给集成者裁决。
- **跨架构浮点确定性**（`ARCH-DET-002`）未验证：`StealFade::gain_at` 用 `f64::powf`，
  只在同架构内确定。跨架构对账属于 CI 上的重活。
- **真实音色库兼容性**未验证：没有任何 >10MB 的注册采样用于端到端试听（红线 9 禁止未登记资产）。

---

## 9. 边界（这次变更**不**证明什么）

- 不证明能正确播放任何真实商业 SFZ 音色库（未做端到端试听；`<master>` / 调制 opcode 未实现）。
- 不证明 `region_for` 的性能达到 `BASELINE-*` 指标（未跑 benchmark；本机禁止）。
- 不证明 fuzz 千万次零崩溃（目标已就位但未执行）。
- 不证明 RT 路径零分配的机械结论（见上）。
- 不证明与任一具体播放器（sfizz / ARIA / Cakeworks）的 bit-for-bit 兼容（切分规则是工程裁决）。
- 未触碰任何根级共享文件：根 `Cargo.toml`、`Cargo.lock`（除依赖解析自动更新外）、`AGENTS.md`、
  `README*`、`.github/**`、`scripts/**`、`docs/DEVELOPMENT_LEDGER.md`、`docs/adr/**`、
  `docs/YEBAN_*.md`、`schemas/**`、其它 `crates/**`、`spikes/**`。

## 10. needs / pending 汇总

| 项 | 类型 | 责任人 |
| :--- | :--- | :--- |
| `.github/workflows/gates-manual.yml` 增加 fuzz 手动 job（命令见第 7 节） | needs | 集成者 |
| `MUST-GATE-011` 千万次 fuzz 尚未执行 | pending | CI 手动档 |
| 「RT 零分配」分配器计数证明 | pending | 集成者裁决（是否引入计数依赖） |
| 第 6 节的 8 项歧义裁决 | needs | 人类 / BDFL |
| `<master>` 四级继承、调制 opcode、`trigger`/`off_*` | pending | 后续切片（ROAD-M2-006 起） |
| `docs/DEVELOPMENT_LEDGER.md` 登记本次测量（上限口径、56 条测试） | needs | 集成者（该文件集成者独占） |
