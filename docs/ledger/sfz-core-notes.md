# sfz-core 工作线笔记（口径 / 来源 / 缺口）

> ⚠ **本文件为历史快照**（落款 2026-10-09）：§5「已实现 vs 明确未实现」等清单是**当时**的读数，**之后**的变化见 [`phase-status.md`](phase-status.md)、⛔ 本文件不再逐条追改。

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

**已实现**：`<control>`/`<global>`/`<master>`/`<group>`/`<region>` 段头（含「段头后同行跟 opcode」）、
四级作用域继承（region → group → master → global）、`//` 行注释（引号内不截断）、`opcode=value`
（一行多个）、取值去引号、`#define $VAR`（定义时展开一层 + 使用处单遍替换）、`#include`
（相对路径 + glob + 沙箱 + 环/深度/数量上限）、`key`/`lokey`/`hikey`/`pitch_keycenter`/
`lovel`/`hivel`/`lochan`/`hichan`/`loop_start`/`loop_end`/`loop_mode`/`tune`/`transpose`/
`volume`(=`gain`)/`pan`/`seq_position`/`seq_length`/`group`(=`polyphony_group`)/`off_by`(=`offby`)、
`sw_last`/`sw_lokey`/`sw_hikey`/`sw_down`/`sw_up`、`loccN`/`hiccN` CC 门控、
`offset`/`end`/`direction`（进入可渲染采样区间）、`trigger`（事件门控，含
`release`/`release_key` 强制 `loop_mode=one_shot` 的规范覆盖）、
`default_path` 拼接，以及固定容量声部池 + 确定性窃取 + 3ms 指数淡出。

**明确未实现**（都记在这里，不静默降级）：

1. **未建模的段头**：`<curve>`、`<effect>`、`<midi>`、`<sample>`。识别到即产生
   `Warning::IgnoredHeader` 并**丢弃其 opcode**，绝不当作 region。
   （`<master>` 原在本条；现已按「region → group → master → global」四级链实现。）
   按第 11 节的语料普查，这四种段头在登记语料里分别出现于 27 / 19 / 0 / 0 个 `.sfz` 文件，
   且本 crate 暂无消费这些段头的渲染路径。
2. **合成/调制类 opcode**：`ampeg_*`/`fileg_*`/`pitcheg_*`、滤波（`cutoff`/`resonance`）、
   LFO、`egN_*` 等一律忽略（引擎侧后续切片）。按第 11 节的语料普查，这是**使用面最广**的
   未实现类（`ampeg_release` 出现在 348 / 1398 个文件，`ampeg_attack` 310 个）。
3. **`off_mode` / `off_time` / `loop_count` / `count`** 未建模。
   按第 11 节的语料普查，`loop_count` 与 `count` 在登记语料里出现 **0 次**（不是当前的高价值缺口）；
   `off_mode` 出现在 89 个文件（`normal` 占 818 / 909 次）。
   特别地：规范里 `off_time` 默认 **0.006 s**，而 `ARCH-RT-004` 规定窃取淡出 **3 ms** ——
   本实现按 Normative 的 3 ms，且不读 per-region `off_time`（见第 6 节冲突项）。
4. **`trigger` 的 release 家族只做事件门控**：`release` / `release_key` 的
   「是否存在对应的 attack region」「`rt_decay` 音量缩放」未实现。规范自己写明这一族行为
   「varies considerably between SFZ players」（ARIA / DropZone / rgc 三套语义各不相同），
   因此**不发明**判定：本 crate 只按事件类型选择 region。
5. **`trigger=first` / `legato` 需要调用方给状态**：两者要求「触发时有没有**其它**音符按着」。
   该状态由 `RegionQuery::held_notes` 传入；严格策略下**不提供就不匹配**
   （与 keyswitch / CC 门控同一条口径）。引擎侧必须提供该状态，否则这两类 region 不发声。
6. **随机轮替** `lorand`/`hirand`：**故意不实现**，因为 `ARCH-DET-001` 禁止随机数。
7. **参数化宏** `#define $F(a) …`：未实现（规范正文未定义，记为缺口）。
8. **递归宏展开**：只做「定义时一层 + 使用处单遍」，不做递归 —— 这是为了杜绝
   `#define $A $A` 类的爆炸与死循环。
9. **`loop_mode` 缺省**：规范说缺省取决于采样文件是否带 loop 元数据；本 crate 不解码音频，
   因此**一律缺省 `LoopMode::NoLoop`**。
10. **`note_offset`/`octave_offset`/`set_ccN`/`sw_note_offset`** 未实现（CC 门控只读 `loccN`/`hiccN`）。
11. **段头只能出现在行首**（其后可紧跟 opcode）。`…<region>…` 出现在取值中间时会被当成取值的一部分。
12. **音名八度**固定 IPN（`C4 = 60`）；不提供 Yamaha/`C3 = 60` 兼容开关（见第 6 节）。

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
7. **`<master>` 支持**：**已裁决并实现** —— 现为 `global → master → group → region`
   四级继承（`Header::Master`；新 `<master>` 清空 `<group>`，理由写在
   `parser.rs` 的 `switch_scope` 注释里）。本条保留为历史问题，不再需要裁决。
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
| 9 | `region_inherits_group_values_even_when_a_later_group_header_intervenes`、`region_inherits_default_path_across_a_later_control_header` | **不是注入，是真实缺陷**：`finalize_region` 原本只在下一个 `<region>` 段头触发，中途的 `<group>`/`<control>` 会清空继承表 → 先写的 region 被整段丢弃（`len() == 1` 而非 2） | **红**（先写判据 → 红 → 修 `switch_scope` 让归约先于任何作用域切换 → 绿） |
| 10 | `a_release_region_is_not_selected_by_a_note_on`、`release_and_release_key_differ_on_the_sustain_pedal`、`trigger_event_matching_table_is_total`、`playback_spec_carries_the_trigger_and_forces_one_shot_for_releases` | `Trigger::responds_to` 的 `Release` 分支改成恒 `true`（还原 origin/main 的「完全不看 trigger」语义） | **红** 4 条（字面：`a trigger=release region must not be selected by a note-on`、`release waits for the pedal to come up`、`assertion left: true / right: false: Release vs NoteOn`、`assertion failed: instrument.playback_for(...).is_none()`） |
| 11 | `release_regions_force_one_shot_loop_mode`、`playback_spec_carries_the_trigger_and_forces_one_shot_for_releases` | `Region::effective_loop_mode` 直接 `self.loop_mode`（丢掉 `trigger=release`⇒`one_shot` 的规范覆盖） | **红** 2 条（字面：`assertion left == right failed: overridden / left: LoopContinuous`、`... forced by trigger / left: LoopContinuous`） |
| 12 | `first_and_legato_require_the_held_note_count` | `Trigger::held_notes_ok` 恒 `true` | **红** 1 条（字面：`state not provided`） |

绿色基线（还原后，`cargo test -p yeban-sfz`）：**27 lib + 16 include_sandbox + 12 malformed_inputs
+ 3 doc-tests = 58 全绿**；`clippy -p yeban-sfz --all-targets -- -D warnings` 零告警。

`trigger` 切片后的绿色基线（`cargo test -p yeban-sfz --no-default-features`）：**61 lib
+ 16 include_sandbox + 12 malformed_inputs + 4 doc-tests = 93 全绿**（lib 由 50 增至 61，
新增 11 条判据）。上面三条新注入（10–12）全部还原：`cmp` 与 `shasum -a 256` 与注入前一致。

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
- (f) 附加：作用域归约顺序正确性（真实缺陷回归）→ 注入 9 的两条
  `region_inherits_*` 测试

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

- 不证明能正确播放任何真实商业 SFZ 音色库（未做端到端试听；调制 opcode 与
  `off_mode`/`loop_count`/`count` 未实现；`trigger` 只做事件门控）。
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
| 调制 opcode（`ampeg_*` 等）、`off_mode`/`off_time`/`loop_count`/`count`、`<curve>`/`<effect>` 段头 | pending | 后续切片（ROAD-M2-006 起；使用面排序见第 11 节） |
| `trigger=first`/`legato` 的「其它音符按住数」必须由引擎侧提供（`RegionQuery::held_notes`），否则这两类 region 不发声 | needs | 引擎接入时（`yeban-engine` 的 `SynthEngine::trigger`） |
| `docs/DEVELOPMENT_LEDGER.md` 登记本次测量（上限口径、58 条测试） | needs | 集成者（该文件集成者独占） |
| **门禁缺口**：`scripts/gates/run-gates.sh crate <name>` **不跑** `license_inventory.py --check` 与 `validate_schemas.py`，只有 CI 的 `checks` job 跑。本次加 `thiserror` 依赖就因此先红了一次（见下）。建议集成者把 `license_inventory.py --check` 并入 `run-gates.sh light`。 | needs | 集成者（`scripts/**` 集成者独占） |
| `docs/ledger/dependency-licenses.md` 是**生成物**：加依赖后必须重跑 `python3 scripts/gates/license_inventory.py`。本次已重跑并随提交更新（`thiserror` 直接依赖者增加 `yeban-sfz` + `Cargo.lock` 哈希）。多条工作线同时加依赖时此文件必冲突，建议集成者按「合并后统一重生成」处理。 | needs | 集成者 |
| `.gitignore` **没有忽略 `/.cache/`**：`scripts/dev/ci-verdict.sh` 在 `~/.cache` 不可写时会把 `XDG_CACHE_HOME` 指到工作区内的 `.cache/`，于是 `gh` 会把 run 日志 zip（约 0.5 MB/个）落在这里；一旦 `git add -A` 就会误入库。本次已删掉误入的 zip（amend 修正）。建议集成者在根 `.gitignore` 增加 `/.cache/`。 | needs | 集成者 |

---

## 11. 登记语料的 opcode 普查（测量 2026-10-06）

**指标**：`git ls-files` 里登记的 `.sfz` 文件中，**至少含一处** `name=` 赋值（先剥掉 `//`
行注释，引号内不截断）的文件数；母体 N = 1398 个文件。单位是「文件数」。这是「该 opcode
在登记语料里有多常用」的**代理指标**，不是「有多少 region 用到它」。

命令：`git ls-files -z | ...` 逐文件剥注释后正则 `(?:^|\s)([A-Za-z_][A-Za-z_0-9]*)\s*=`，
用 `python3 -B` 统计（脚本为一次性探针，未入库）。**注意**：`git ls-files` 对含空格的路径
会加引号，必须用 `-z` 取 NUL 分隔的原始路径，否则文件名会被空白切成假路径。

| opcode / 段头 | 文件数 | 占 1398 | 出现次数 |
| :--- | ---: | ---: | ---: |
| `ampeg_release` | 348 | 24.9% | 2 280 |
| `ampeg_attack` | 310 | 22.2% | 835 |
| `trigger` | 110 | 7.9% | 4 267 |
| `off_mode` | 89 | 6.4% | 909 |
| `amp_veltrack` | 79 | 5.7% | 612 |
| `sw_last` | 58 | 4.1% | 3 453 |
| `lorand` / `hirand` | 44 / 44 | 3.1% / 3.1% | 11 589 / 13 239 |
| `note_polyphony` | 32 | 2.3% | 87 |
| `cutoff` / `resonance` | 31 / 21 | 2.2% / 1.5% | 36 / 21 |
| `<curve>` / `<effect>` 段头 | 27 / 19 | 1.9% / 1.4% | 77 / 19 |
| `off_time` | 15 | 1.1% | 16 |
| `loop_count` / `count` / `direction` | **0** | 0% | **0** |
| `<midi>` / `<sample>` 段头 | **0** | 0% | **0** |

`trigger` 的取值分布（出现次数 / 文件数）：`first` 2 273 / 43、`legato` 1 829 / 48、
`release` 117 / 29、`attack` 28 / 19、`release_key` 20 / 10 —— 五值之和 4 267 **恰好等于**
`trigger=` 的出现总数，说明该语料里的 `trigger` 取值**全部**落在规范表的五个选项内。
`off_mode`：`normal` 818 / 64、`fast` 87 / 21、`time` 4 / 4。

**由该普查得出的取舍**：`loop_count` / `count` / `direction` 在登记语料里出现 0 次，
因此不是当前的高价值缺口；`ampeg_*` 使用面最广但已被第 5 节第 2 条登记为
「引擎侧后续切片」（需要 envelope 的设计裁决）；`off_mode` 的默认值 `fast` 与现行为等价，
建模它只增加字段不改变行为。**本轮选择 `trigger`**：它是第 5 节第 3 条里使用面最广的
（110 个文件），且它决定「region 由哪一类事件触发」——错判会把 release 采样当成 attack 采样、
或让 `legato` 采样在每个 note-on 都发声。

### 11.1 `trigger` 切片的端到端读数（在 1398 个登记 `.sfz` 上）

探针（临时集成测试，已删除）对每个文件用 `IncludeResolver`（基准目录 = 文件所在目录）
展开 `#include` 后 `parse_sources`，再对 **128 个音符 × 3 个力度（64/100/127）** 各做一次
note-on `region_for`，按被选中 region 的 `trigger` 分类计数。

| 读数 | 值 |
| :--- | ---: |
| 登记的 `.sfz` 文件 | 1 398 |
| 解析成功 / include-沙箱或上限错误 / 解析错误 | 1 267 / 107 / 24 |
| region 总数 | 172 350 |
| note-on 被选中的次数（合计） | 281 484（**全部**是 `attack`，985 个文件） |
| 其中 `release` / `release_key` / `first` / `legato` | 0 / 0 / 0 / 0 |
| note-off（踏板松开）能选中一个 region 的次数 / 文件数 | 785 / 21 |

**改动前（origin/main 语义）的重建读数**：把 `responds_to` 与 `held_notes_ok` 都改成恒 `true`
（= 完全不看 `trigger`，即 origin/main 的判定），同一探针得：`first` 被 note-on 选中 **258 次 /
2 个文件**，`legato` **0 次**，`release` **0 次**，`release_key` **0 次**。

**诚实结论**：`region_for` 只返回**第一个**匹配 region，而 release region 在语料里总是排在
attack region 之后，所以「release 采样被 note-on 选中」这个缺陷在本语料上**观察不到**
（0/1398 个文件）。本切片可观察到的行为变化是 `first`/`legato` 的事件门控
（258 次选中 / 2 个文件在无 `held_notes` 时不再匹配）。`trigger` 建模的主要价值在于
**事件路由的正确性**（note-off 路径现能选中 21 个文件里的 release region），而不是修正
本语料上的现存错音。

## 12. 摊平表（`yeban-engine::sfz_instrument::PreparedSfz`）的口径修正与局限（2026-10-15，任务 `#65`）

**驱动的真实缺陷（两处口径不一致）**：

1. `PreparedSfz::prepare` 原先用 `Region::matches_key && matches_velocity` 自拼谓词，
   **绕过**了 `Instrument::region_for_with` 内部的 `region_matches`
   （`trigger` / keyswitch / channel / CC 门控）—— `crates/yeban-sfz/src/instrument.rs` 的
   私有用例在语料里有 **21 个文件**带 release region ⇒ 直接接线会把 release 采样在**按下**时打出来。
2. 离线 `render_note_into` 只取 `find` 的**第一条**匹配，摊平 `render_block_into` **叠加所有**条
   ⇒ 同一份 SFZ 在离线导出与实时两条路上发不同的声音。

**本片统一后的三条口径**：

1. **区域选择**：`yeban-sfz` 公开 `Region::matches_query(&self, &RegionQuery<'_>) -> bool`
   —— `region_matches` 的**公开薄包装**（就是 `region_for_with` 用的那个谓词）。
   引擎的 `regions_matching` / `render_note_into` / `PreparedSfz::prepare` 全部改走它，
   ⛔ 不再自拼谓词。门控不过的区域在摊平表里**两张掩码都清 0** ⇒ 永不进 note-on 命中集。
   ⚠ 严格策略的后果如实登记：`sw_last`（无 `sw_default`）/ `sw_down` / `sw_up` / CC 门控 /
   `trigger=first`/`legato` 的区域，因回调侧**没有**这些外部状态，**不匹配、不发声**
   （与离线 `region_for` 一致；这是 yeban-sfz 既有的严格口径，不是本片发明的）。
2. **多匹配语义 = `all`（叠加）**：两侧都渲染并相加**所有**匹配区域（SFZ 里每个 `region`
   是独立声部）。返回值＝各区域写入帧数的**最大值**（⛔ 不是各区域之和）。
   ⚠ **对既有 app 导出路径的影响**：`crates/yeban-app/src/sfz_load.rs` 的 `SfzTrackSource`
   每条音符只给**一份**样本（`region_sample_for` 取第一条匹配区域的 `sample=`，且它仍自拼
   `matches_key && matches_velocity` —— 第三处旧口径，属 app 面，本片未动）。于是"多匹配 ⇒ 叠加"
   在该路径上表现为**同一份 PCM 按各区域的 rate/gain 叠加 N 次**：当某 `(note, velocity)`
   命中多条 region 时，导出结果与改动前（只取第一条）**不同**。正确的修法是步 3 的
   "每区域各自解码"；本片只统一口径并登记这一影响。
3. **增益**：按 `PlaybackSpec::total_gain()` 的乘积结构摊平成
   `volume × 力度 × 键轴淡化 × 力度轴淡化(in/out 分开)`，回调侧的相乘结合序与离线
   `(volume × 力度) × ((键轴 × 淡入) × 淡出)` **逐位相同** ⇒ `(note, velocity)` 不再冻结在 `(60, 100)`。

**局限登记（本片后仍成立）**：

| # | 局限 | 状态 |
| :--- | :--- | :--- |
| ① | 力度增益 | **本片已修**（每力度一张表，⛔ 不再冻结在力度 100） |
| ② | `trigger` 门控 | **本片已修**（走 `matches_query`；门控不过 ⇒ 掩码全 0） |
| ③ | 无 `offset`/`end` | **本片补了 `SampleSpan`（`start`/`end`）字段**，但渲染**仍未应用**它（应用它必须同时改离线侧，否则两侧口径再次分叉） |
| ④ | 无 `group`/`off_by`/`note_polyphony`/`seq_` | 未做（语义由 `yeban-sfz` 裁、槽位与窃取由 `SynthEngine` 裁 ⇒ 属步 2） |
| ⑤ | 无字节预算、无流式 | 未做（PCM 整段 `clone` 进摊平表；无上限、无流式） |
| ⑥ | 同内容不同路径重复解码 | 未做（每个 region / 每条路径各自解码，⛔ 无去重） |
| ⑦ | CC 轴交叉淡化 | 未做（回调侧无 CC 状态 ⇒ 与离线 `playback_spec` 一致地跳过该轴） |
| ⑧ | `loop_window` / `direction` / `pan` / `off_mode` | 未做（读数在 `PlaybackSpec` 里，摊平表未携带） |

**本片读数**（`cargo test -p yeban-engine`）：

| 判据 | 读数 |
| :--- | :--- |
| 逐样本最大绝对差（**两条都匹配** + 一条不匹配；摊平 3 区域） | **0e0**（`sfz_arm_perf::prepared_matches_offline_sample_by_sample`） |
| 摊平增益 vs `total_gain`（含键轴 ＋ 力度轴 in/out 交叉淡化） | **128×128 网格位型不同点数 = 0** |
| `trigger=release` 的键/力度掩码 | 均 `[0, 0]`；`accepts(60, 100) == false`；离线只剩 release ⇒ 写 **0** 帧 |
| 分配四元组（`rt_zero_alloc` 的 SFZ 回调窗口） | `allocations=0 deallocations=0 lock_blocking=0 io_ops=0`（71/71 场景通过） |
| `PreparedRegion` 结构字节数 | **2672 B**（原 548 B；代价来自 5 张 128×`f32` 表 + `SampleSpan`；⭐ `#91` 加 **+8 B** ＝ `loop_start: u32` ＋ `loop_end: u32` ＋ `loop_mode: LoopMode`(1 B) ＋ 对齐 ⇒ 2624 → 2632；⭐ `#92` 再加 **+24 B** ＝ `ampeg_attack`／`hold`／`decay`／`sustain`／`release` 五字段（5×`f32` ＝ 20 B payload ＋ 4 B 对齐）⇒ 2632 → 2656；⭐ `#94` 再加 **+8 B** ＝ `group: u32` ＋ `off_by: u32`（⭐ `off_mode` 落进对齐填充）⇒ 2656 → 2664；⭐ `#100` 再加 **+8 B** ＝ `note_polyphony: NotePolyphony` ⇒ 2664 → 2672；⭐ ⭐ **`#104` 再加 **+1520 B** ＝ `ampeg_mods: [BakedAmpegCcMod; AMPEG_MOD_CAP=20]` ＋ `ampeg_mod_count: u8`（**曲线点表烘进每一条调制项**：`BakedCurve { index, len, points: [CurvePoint; 8] }`）⇒ 2672 → 4192；⭐ ⭐ **`#106` 再加 **+40 B** ＝ `ampeg_vel2: [Option<f32>; AmpegParam::COUNT=5]`（每参数**至多一条**力度调制量，见 `#106`）⇒ **4192 → 4232**） |


## 13. 引擎侧声部 ＋ 快照形状（2026-10-15，任务 `#66` 的**步 2**）

设计裁决见 `docs/ledger/phase-status.md` 的 R698（裁决 **甲-1**）：`TrackSlot` 增
`sfz: SfzVoicePool`（`[SfzVoice; 16]`、全 `Copy`）＋ 快照侧只读 `PreparedSfz`
＋ `render_track` 加第 3 参 `sfz: Option<&PreparedSfz>`（与既有 `Option<&NoteSchedule>` 对称）。
本片＝**引擎侧发声器**那一半；**绑定来源**（轨↔SFZ 的模型语义）仍未做。

### 交付形状

| 件 | 归属 | 说明 |
| :--- | :--- | :--- |
| `SfzPlan` | 快照（`BTreeMap<EntityId, SfzPlan>`） | 控制侧投影；重数据（区域表 ＋ PCM），与 `insert::ConvolutionPlan` 同族 |
| `SfzVoice` / `SfzVoicePool` | `SynthEngine` 的轨道槽 | 定长 `[SfzVoice; 16]`，全 `Copy`；步 2 是池满**丢弃 ＋ 计数**、⛔ 无窃取 ⇒ ⭐ **任务单 `#70` 改为窃取口径**（见 §13.4） |
| `render_track(track, schedule, sfz, out)` | 实时路径 | 第 3 参是**借用**的快照摊平表（PCM 借用快照，⛔ 不 clone `Vec`） |
| `EngineSnapshot::with_sfz(track, plan)` | **唯一**绑定入口 | `from_project` 不填 ⇒ 产品路径默认空表 ⇒ 逐位不变 |

**分派次序 ＝ 鼓机 ＞ SFZ ＞ 复音**（一条轨至多一件音源）。**终点口径**（无包络时的诚实占位）
＝ `min(end_sample, span 终点, 样本播完)`：⛔ 跳过 `ampeg_*`，⭐ 用样本自然结束；播完即**池内退休**
（`active = false`，⛔ 不碰快照退役队列）。

### 本片读数（`cargo test -p yeban-engine`）

> ⚠ 下表是**步 2** 当时的读数（历史留痕）；⭐ **尺寸**一行已被步 3 改写（见 §13.2），
> `span`/`end=-1` 两行的"摊平表仍出声"也已被步 3 的**同口径**取代。

| 判据 | 读数 |
| :--- | :--- |
| 引擎声部路径 vs `PreparedSfz::render_block_into`（两条 `key=60` 都匹配） | **逐样本最大绝对差 = 0e0**（两侧非零 478 / 478） |
| 样本自然结束 ⇒ 退休 | 播完那一块后活声部 `Some(0)`，下一块**逐位静音**；反向对照：摊平表同一块仍写 479 帧 |
| 显式 `span` 终点（`end=200`） | 步 2：引擎在**第 200 帧**停（span 内与摊平表**逐样本 0**）；摊平表继续到 478（当时的口径分叉）⇒ ⭐ **步 3 已同口径**（两侧都停在第 200 帧，见 §13.2） |
| `end=-1`（`SampleEnd::Silent`） | 步 2：引擎 **0 帧**、0 声部；摊平表仍出声（对照）⇒ ⭐ **步 3 已同口径**（两侧都 0 帧，见 §13.2） |
| ⭐ **反向对照（接线前）** | **不给 `sfz`** 的端到端指纹 == 改动源码前实测值：`0x71dca4dc55b44df5` / `0x6eee378072730521`（`tests/sfz_instrument.rs` 的 S5） |
| 端到端（SFZ 臂） | 非零 1716、峰值 0.21920355、指纹 `0x5587041753dfe375` ≠ 对照臂 `0xd50d599c66c92565` |
| 分派次序 | 鼓机＋SFZ 的输出对"只有鼓机"**逐位相同**；无鼓机时同一份 SFZ 非零 127 |
| 池满（17 条匹配区域） | 步 2：`created = 16`、`dropped = 1`（⛔ 不静默）⇒ ⭐ **任务单 `#70` 已改为窃取口径**：`created = 16`、`stolen = 1`、`dropped = 0`（见 §13.4） |
| 尺寸（步 2） | `SfzVoice = 32 B`、`SfzVoicePool = 520 B`、`SynthEngine = 89936`、`EngineRuntime = 160784 < 196608`（192 KiB）⇒ ⭐ 步 3 改为 **40 / 648 / 91984 / 162832** ⇒ ⭐ **任务单 `#70` 再改为 40 / 1712 / 109152 / 180000 < 196608**（见 §13.4） |
| 零分配（`rt_zero_alloc` ㉙） | 10 000 量子（总能量 2999.164）＋ 1 次换采样率：四元组 `alloc=0 dealloc=0 lock_blocking=0 lock_waits=0 io_requests=0 io_ops=0`；该档 **73 / 73**（⭐ 步 3 后逐字不变） |
| 摊平表内存 | 2 区域 ＋ 480 帧 PCM = **10384 B**（广播路径；⭐ `#106` 后 ＝ 区域表 `4232 × 2 = 8464` ＋ PCM 1920 ＋ 路径 0；⚠ 沿革：`#104` 前 **7168** ⇒ `#104` **10304** ⇒ 现值 **10384**）⇒ ⭐ **任务单 `#84` 起可拆开读**（区域表 ／ PCM ／ 路径文本，见 §13.5） |

### needs（⭐ 下表状态截至**任务单 `#84`**，即 2026-10-16 的"摊平表字节预算读数与判据"；权威表在 `yeban-engine/src/sfz_instrument.rs` 模块头）

| # | 缺口 | 状态 |
| :--- | :--- | :--- |
| sfz-needs-1 | **轨推子/静音未乘到 SFZ 声部** | ⭐ ⭐ **已闭合（任务单 `#68`；裁决 R711）**：**乐器层**拥有乐器内增益（`volume_gain`／`velocity_gain[128]`／`key_crossfade`／`velocity_fade_*`／`amp_veltrack`）、**引擎层**拥有轨增益（推子／静音／独奏）⇒ **两者相乘、⛔ 不重复**。`snapshot::project_schedules` 把**同一个** `track_gain_scale(...)` 结果写进 `NoteSchedule::track_gain`，`SfzVoicePool::trigger_note` 乘在 `region.gain_at(...)` 之外 ⇒ ⭐ 与鼓机**同径**；S17／S18 钉住（读数见 §13.3） |
| sfz-needs-2 | **池满只丢弃 ＋ 计数，无窃取口径** | ⭐ ⭐ **已闭合（任务单 `#70`）**：引擎侧 SFZ 池**采纳引擎既有那一套**（`PolySynth` 的三段键 `(层级, 触发序号, 槽位下标)`）＋ 3 ms 指数淡出（`StealFade` 形状、**同一把** `steal_fade_frames` 舵）；`yeban-sfz::VoicePool::select_victim` 是**它自己那一份**且**不被引擎使用** ⇒ ⛔ 不是"两份实现"。落地：池满走 `steal`（victim 在自己槽里淡出、新声部**挂起在同一槽**）；真丢弃只剩"挂起者被再一次换掉"（计 `dropped`，⛔ 不静默）、窃取计 `stolen`；S8／S19 钉住（读数见 §13.4） |
| sfz-needs-3 | **鼓机 ＋ SFZ 同轨**（引擎＝鼓机优先，SFZ 池被释放） | ⭐ ⭐ **已闭合（信号腿；任务单 `#70`）**：裁决**维持** `鼓机 ＞ SFZ ＞ 复音`（与 `crate::drums` §4.1 一致 ⇒ ⛔ 不改叠加）；落地＝新增**投影读数** `SynthEngine::armed_sources`／`ignored_sources`（三件同轨 ⇒ 武装 `3`、被忽略 `2`）；S20 钉住（读数见 §13.4）。⚠ **模型面**那一半仍挂在 `crate::drums` 的 `needs-3` |
| sfz-needs-4 | **`span` 终点引擎侧已应用、离线/摊平侧未应用** | ⭐ **已闭合（步 3）**：三条路共用 `reduce_span` 一份归约 ⇒ `start` ＋ `end` 都应用；S3/S13 逐样本差 **0e0**（读数见 §13.2） |
| sfz-needs-5 | **轨↔SFZ 的模型绑定不存在**（**"SFZ 如何进 `YebanProjectV1`"未裁**） | ⭐ ⭐ **已闭合（模型面＝任务单 `#76`、引擎面＝任务单 `#78`）**：模型面 = `TrackV3.sampler: Option<SamplerBinding>`（`SamplerBinding { path }`，只留痕路径、⛔ 零运行时对象；裁决见台账 `R731`）；引擎面 = `EngineSnapshot` 携带**待摊平项** `PendingSfz`（`from_project` 从 `track.sampler` 投影、只搬路径字符串）⇒ ⭐ **摊平仍全在控制侧**（引擎⛔ 不读盘、⛔ 不引 `symphonia`/`rubato`）：`sfz_pending()` →（读盘／解析／解码）→ `SfzPlan::prepare_with_samples` → `EngineSnapshot::with_sfz`（后者**取代**前者）。⛔ 不加 `DeviceKind` 档、⛔ 不改 schema；S21～S24 钉住 |
| sfz-needs-6 | **循环 / ADSR / `off_mode` / `group`/`off_by` / `note_polyphony` / `direction`** | ⭐ **三段式（第 264 条）：已落 ／ 仍未落 ／ 登记在案的偏离**。**① 已落**：㈠ **多采样 ／ 每区域独立 PCM**（步 3）；㈡ ⭐ **循环**（任务单 `#91`）——`loop_start`／`loop_end`／`loop_mode` 四值早已**解析且有判据**，`#91` 把它们搬进 `PreparedRegion` 并接进**唯一的逐样本混音循环** `mix_span_into`（三条路同口径；`SfzVoice` **+0 字节**；判据 S28～S33）；㈢ ⭐ **ADSR**（任务单 `#92`）——`ampeg_attack|hold|decay|sustain|release`（秒／百分比，认 `s`/`ms`/`%` 后缀、`ms` 优先）、**四段全分段线性**、输入政策＝**响亮 `Err`** 且**分清"缺失 ≠ 非法"**、`SfzVoice` **+0 字节**（判据 S34～S38 ＋ 解析层 3 条）；㈣ ⭐ ⭐ **`off_mode` ＋ `group` ／ `off_by`**（任务单 `#94`）——两者**也早已解析**（`instrument.rs:1398-1401` 与 `:1531-1532`，别名 `polyphony_group`/`offby`），`#94` **只加播放层**：`off_mode` **只管 `off_by` 关断路径**（规范逐字 "how a region is turned off by an `off_by` opcode"）、`group`/`off_by` 独占组（victim 按自己的 `off_mode` 处理）、**触发者不自掐**、**`one_shot` victim 忽略 `off_by`**、victim 只停 `active` 并**保留同槽 `pending`**（判据 S39～S45；`PreparedRegion` 2656 → **2664**，`SfzVoice` **+0**）；㈤ ⭐ ⭐ **`note_polyphony` ＋ `note_selfmask`**（任务单 `#100`）——两者**也早已解析**（`read_note_polyphony`，`instrument.rs:1987`；归约 `NotePolyphony` `:392-440`），⭐ **而本仓甚至早有一份完整策略** `VoicePool::apply_note_polyphony`（`voice_pool.rs:418`）——但那个池被明文登记为**不被引擎使用**，引擎的 `SfzVoicePool` 从不消费它 ⇒ `#100` **只加播放层**（触发期一次 ≤16 槽扫描；⛔ 无 per-voice 字段）；⭐ **规格有据**（力度资格规则 ＋ "A new note will always play." ＋ "not a strict polyphony limit but more of a **hint**"；唯一规范沉默处＝多个同资格者掐谁 ⇒ **复用仓内既有裁决**）；⭐ **语料 32 文件/87 处**（对照 `direction` **0** 处）；⭐ ⭐ **并逼出一条与 `#94` **相反**的裁决**：`one_shot` 的 victim **照掐**（`#94` 里 `one_shot` 对 `off_by` **免疫**）——依据＝语料里 `note_polyphony` **几乎全用在 `one_shot` 上**（`virtuosity-drums` 每麦克风一组；`cithara` 的 `loop_mode=one_shot`/`note_polyphony=1`/`note_selfmask=off`/`group=1` **四行紧挨**），若沿用外推则该 opcode 在**全部**用到它的文件里变空操作；判据 S49～S53；`PreparedRegion` 2664 → **2672**，`SfzVoice` **+0**）；㈥ ⭐ ⭐ **`ampeg_*` 的 CC 调制**（任务单 `#104`）——`ampeg_*ccN`（SFZ1 旧形）／`ampeg_*_onccN`（SFZ2；规范逐字说它与 `ccN` **"may be aliases"**）／`ampeg_*_curveccN`（ARIA；载**曲线下标**而不是调制量）**此前解析层一个都读不到**（解析走**精确键**，后缀形态永远匹配不上 ⇒ 第 274 条）⇒ `#104` **从 0 到 1 新实现**：名字识别 ＋ 归约（`BTreeMap` 键序 ＝ 参数声明序 × CC 升序，求和顺序唯一）＋ **离线两条路**求值（`base + Σ delta × curve(cc)`；四处独立见证）；⭐ ⭐ **实时通路⛔ 仍未接线**（有意边界、逐字登记在 `yeban-engine/src/sfz_instrument.rs` 模块头；属"架构单"、需人类批准）⇒ ⭐ **本条只闭合一半**；`PreparedRegion` 2672 → **4192**（＋1520 B，摊平期），`SfzVoice` **+0**）；㈦ ⭐ ⭐ **`ampeg_vel2*`（力度调制）**（任务单 `#106`）——规范 `modulations/vel2` 页逐字给出**唯一**公式 `(param) = (eg type)_(param) + ( (eg type)_vel2(param) * velocity / 127 )`（⭐ 五个 `ampeg_vel2*` 表行的 Range 均为 `-100 to 100`、单位随目标）；⭐ **`#106` 之前一个字都没有**（无识别／无归约／无求值）⇒ 本片**新实现**并 ⭐ ⭐ **三条路全部接线**（离线 ／ 摊平表 ／ **实时** —— 实时那条读 `SfzVoice::velocity`，⭐ 该字段**早就在**声部里 ⇒ ⛔ **零新逐声部字段**）；判据 **S59～S62**（含「⭐ 离线 ＝ 摊平 ＝ **实时** 位模式逐位相同」）；`PreparedRegion` 4192 → **4232**（＋40 B ＝ `[Option<f32>; 5]`，摊平期），`SfzVoice` **+0**）。**② 仍未落**：⭐ **`ampeg_*` 的 **CC 支**的实时接线**（`SfzVoicePool` 的触发与渲染今天**不读任何 CC 状态**；⭐ **力度支**已由 `#106` 三条路接全）；`fileg_*`；`pitcheg_*`；`direction`（**反向循环**）；`off_time`（`off_mode=time` 的延迟）。**③ 登记在案的偏离（三条，都有依据、⛔ 不是静默降级）**：㈠ `ampeg_release` **规范缺省 `0.001` 秒、本仓取 `0`**（依据＝保住既有逐位读数；`0.001 s @48 kHz` ＝ 48 帧斜坡会动既有夹具；显式取值一律原样读入）；㈡ `off_mode=time` **按 `normal` 处理**（差别只剩 `off_time` 的延迟，而它不在本片范围）；㈢ `off_by=1 ≡ 同音高`**有意不做**（两页规范全文只有通例、仓内无据 ⇒ ⛔ 不发明）；㈣ `group == off_by` 时**不自掐**（依据＝登记语料 `ork_dialogue.sfz` 正是 `group=1`+`off_by=1` 的"同组独占"写法，自掐会让那 20 处**发不出声**）。|
| sfz-needs-7 | ⭐ **（步 3 新增）缺样本的引擎侧回退是"广播到 `samples[0]`"（内容错）** | ⭐ ⭐ **已闭合（任务单 `#68`；裁决 R711）**：**引擎出静音并计数**（`silent_region_count` ＋ `missing_sample_paths`）、**合成替代内容是控制侧的决定**（想给替代样本 ⇒ 调用方显式放进提供表）。`prepare_with_samples` 遇缺失路径 ⇒ 该区域 `sample_index = NO_SAMPLE` ⇒ 触发时**不建声部**（与 `span()==None` 同径）；⛔ 旧"广播到 `samples[0]`"**已删除**；S15 已**如实地**改写期望（读数见 §13.3） |
| sfz-needs-8 | ⭐ **（步 3 新增）摊平表不含样本字节预算 / 流式** | ⭐ ⭐ **部分闭合（任务单 `#84`）——"可判"那一半已闭合，"流式"那一半仍未做**：①**字节预算读数 ＋ 判据**已落地（`SfzByteBudget` ／ `SfzByteUsage` ／ `SfzBudgetOverrun`、`PreparedSfz::byte_usage` ／ `PreparedSfz::budget_overrun`、控制侧入口 `SfzPlan::byte_usage` ／ `budget_overrun`；判据 S25～S27，读数见 §13.5）；②**超预算口径 ＝ 引擎只报数、控制侧决定**（⛔ 不报错、⛔ 不计数、⛔ 不截断、⛔ 不改摊平结果；三条理由见 `PreparedSfz::budget_overrun`）；③⚠ **仍未闭合**：每个被引用的样本 `pcm` 仍**整段 clone** 进摊平表（去重后 N 份）⇒ **无流式、无 `Arc` 共享**，且**峰值内存不受本预算约束**（调用方手里那份已解码 PCM 本就在场） |

⚠ **换采样率的后果**（实测于 ㉙）：摊平表带着构造期的 `engine_hz` ⇒ 换采样率必须由
**控制侧在窗口外重新摊平**并发布一份新快照；引擎⛔ 不在快照边界重算，**在飞的声部**
保留触发时抄下的 `rate`。

### 13.1 覆盖缺口闭合（2026-10-16，任务 `#66` 补做 2）

⭐ 步 2 的**注入（打红）验证**发现两条判据的**覆盖缺口**（裁定见 `phase-status.md` R702），已补齐：

| 判据 | 判定量 | 补前（注入态） | 补后（还原态） |
| :--- | :--- | :--- | :--- |
| **S10**（换音源／定位必须释放 SFZ 池） | `sfz_active_voices` ＋ 切回 SFZ 那一块的**非零样本数** | 换音源后 `Some(2)`、切回非零 **128**、`seek` 后 `Some(2)` | `Some(0)`、**0**、`Some(0)`（触发后 `Some(2)` 为前置见证） |
| **S1b**（逐块拼接的整音符：8 × 128 帧 vs 摊平表**一次性 1024 帧**） | 逐样本最大绝对差 ＋ **有声块数** | 差 **`7.505936e-1`**、有声块 = **1** | 差 **`0e0`**、有声块 = **4** |

对应的注入（均已逐字还原，`sha256` ＋ `git diff` 0 字节）：

1. **删 `slot.sfz.silence()` 三处**（`synth.rs:1483`/`:1831`/`:1841`）⇒ **只有 S10 变红**：
   `left: Some(2)` / `right: Some(0)`；S1~S9 与 `rt_zero_alloc` ㉙ 仍全绿。
2. **`written < remaining` → `<=`**（`sfz_instrument.rs:830`）⇒ **只有 S1b 变红**（差 `7.505936e-1`、有声块 1）；
   ⚠ 同一条判据的**单块**腿读数**一字不变**（`0e0` / 478 / 478）—— 那正是"单块尺度看不出来"的机械证据。
   （副作用：S10 的**前置见证**也会红：提前退休让"触发后必须有 2 个声部"不成立。）

### 13.2 步 3：多采样 ＋ ⭐ 绑定注入 API（2026-10-16，任务 `#67`）

设计来源：R698 的**步 3**（"多采样 ＋ 绑定注入（控制侧）"）＋ 本线自己登记的 `needs`。
本步**只动控制侧**：⛔ 不加 `DeviceKind` 档、⛔ 不改 schema、⛔ 不碰 `yeban-model`／`yeban-sfz`／`yeban-app`。

#### 交付形状（`crates/yeban-engine/src/sfz_instrument.rs`）

| 件 | 形状 | 说明 |
| :--- | :--- | :--- |
| `PreparedSample` | `path` ＋ `pcm` ＋ `sample_hz` | 摊平表里的**一份**样本；`path` ＝ 去重键（`region.sample_path()` 原样字符串） |
| `PreparedRegion::sample_index` | `u16` | 指到 `samples` 的下标；`NO_SAMPLE`（`u16::MAX`）＝ 不产生采样输出 |
| `PreparedSfz::prepare_with_samples` | 控制侧构造 | **每区域播自己**那份；同一路径**只 materialize 一次**（`BTreeMap` 索引，⛔ 无 `HashMap`） |
| `PreparedSfz::prepare` | 单样本**广播**（⛔ 未删） | 所有 `sample_index = 0`；键/力度映射对、**音色内容错** ⇒ `sample_content=broadcast(1)` |
| `reduce_span` | **唯一**归约 | 离线 `render_note_into`／摊平 `render_block_into`／实时声部三条路共用 ⇒ `start`＋`end` 同口径 |
| `SfzInstrument::referenced_sample_paths` | 控制侧查询 | 排序去重的路径全集（调用方据此解码；解码⛔ 不在引擎） |
| `SfzPlan::prepare_with_samples` | 公开入口 | 经 `EngineSnapshot::with_sfz` 注入 ⇒ **唯一**的轨↔SFZ 绑定路径（模型语义⛔ 不发明） |

#### 本步读数（`cargo test -p yeban-engine`；`sfz_instrument` 16 / 16，全套 296 + 各集成档全绿）

| 判据 | 读数 |
| :--- | :--- |
| **S11 多采样各播自己** | `key=60` 摊平指纹 `0xd8710445ec5a5faf` ＝ 离线(`a.wav`) 指纹；`key=64` `0x29bc219a1b7a0b4b` ＝ 离线(`b.wav`)；⭐ 两条指纹**不同**。反向对照：**对调**提供表 ⇒ `0x28928510f012cb43` ≠ 原值；**广播**在 `key=64` ⇒ `0xb0d8460b4f0cf499` ＝ "用 `a.wav` 播 `key=64`"的离线渲染 ⇒ **音色内容错**的机械证据 |
| **S12 同一样本引两次只解码一次** | 同一路径引两次：`sample_count = 1`、`pcm_bytes = 1920`（480 × 4，**一份**）；反向对照（两条不同路径）：`2` / `3840`。提供表同一路径两份 ⇒ 首个为准（输出 ＝ 第一份的离线参照，⛔ 不静默合并） |
| **S13 `start`/`end` 真正应用** | `offset=10 end=200`：摊平写入 **190** 帧（半开 `[10,201)`）；**离线 vs 摊平差 `0e0`**；**引擎 vs 摊平差 `0e0`**；输出 `[0] = 0.27231953` **逐位等于** `源[10] × gain`；反向对照 `offset=50` ⇒ 写入 **150** 帧、输出 `[0] = 0.12940946`（起点真的进了循环） |
| **S3 `end` 两侧同口径**（改写） | `end=200` ⇒ 引擎与摊平表都停在**第 200 帧**、整块**逐样本最大绝对差 `0e0`**、摊平写入 **200** 帧；反向对照 `end=400` ⇒ **400** 帧 |
| **S4 `end=-1` 两侧同口径**（改写） | 引擎 **0** 帧 / `Some(0)` 声部；⭐ 摊平表**也** 0 帧（步 2 时它仍出声 ⇒ 分叉闭合）；反向对照（无 `end=-1`）⇒ 摊平写入 **479** 帧 |
| **S14 广播重载与步 2 逐位相同** | span-free 夹具 `(60,127)`、512 帧：**FNV-1a `0xa933ddab286bdf5e` ＝ 本线在 HEAD `6ae70ef8`（改动前）实测值**；`is_broadcast()` ✓、`sample_content_label() = "broadcast(1)"`、写入 479 / 非零 478 |
| **S15 缺样本如实回退 ＋ 计数** ⚠ **已被 §13.3 改写** | ⚠ **旧期望固化了错内容**（"缺样本 ⇒ 回退到 `samples[0]`，输出与'用 `a.wav` 播那个键'逐位相同"＝把**内容错**钉成了期望）。步 3 当时的读数：`fallback_region_count = 1`、`missing_sample_paths = ["missing.wav"]`、`sample_content = distinct(1)+fallback(1)`，指纹 `0xb0d8460b4f0cf499`；空提供表 ⇒ `silent = 2`、写入 **0** 帧、`missing = ["a.wav","missing.wav"]`。⭐ **新期望见 §13.3**（静音 ＋ 计数；`+fallback(...)` 这一档已删） |
| **S16 绑定注入 API 经公开路径（多采样）** | `referenced_sample_paths = ["a.wav","b.wav"]` → `SfzPlan::prepare_with_samples`（`sample_count = 2`）→ `EngineSnapshot::with_sfz` → **端到端**非零 1716、峰值 0.21920355、指纹 `0x10e619a422b4b7dd` ≠ 不给 SFZ 的 `0xd50d599c66c92565`；反向对照：**广播** plan 的端到端指纹 `0x5587041753dfe375` ≠ 多采样版 |
| **尺寸（照打）** | `SfzVoice = 40 B`（原 32：`sample_index: u16` ＋ 对齐）、`SfzVoicePool = 648 B`（16 × 40 ＋ 8）、`SfzPlan = 96`、`PreparedSfz = 96`、`SynthEngine = 91984`、`EngineSnapshot = 4480`、⭐ `EngineRuntime = 162832 < 196608`（192 KiB） |
| **零分配（`rt_zero_alloc` ㉙，未改夹具）** | 10 001 量子（总能量 **2999.164**，与步 2 逐字相同）＋ 1 次换采样率：四元组 `alloc=0 dealloc=0 lock_blocking=0 lock_waits=0 io_requests=0 io_ops=0`；该档 **73 / 73** |
| **既有判据未回归** | S1/S1b/S2/S5/S6/S7/S8/S10 读数逐字不变（S5 两条接线前指纹仍是 `0x71dca4dc55b44df5` / `0x6eee378072730521`；S6 端到端仍是 `0x5587041753dfe375`） |

#### ⭐ 注入打红（三处，各自的**红读数** ＋ 还原自证）

还原自证：注入前记录 `sha256(sfz_instrument.rs) = 5fd478c52901937747376c3830db58b5d8590f46eeb8c9a4c984da51ff2ac35b`
与 `git diff`（100111 字节）；每次还原后 `sha256sum -c` **OK**、`diff -q` 两次 `git diff` **逐字节相同**、
文件里 `INJECT` 标记数 **0**。

| # | 注入 | 红读数 | 变红的判据 |
| ---: | :--- | :--- | :--- |
| ① | `flatten_regions` 里 `sample_index` **恒置 0** | `s11`：`"b.wav" ⇒ 第 1 份` `left: 0 / right: 1`；`s15`：`补上内容之后输出必须变` `left: 12743012160029652121 / right: 12743012160029652121`；`s16`：`多采样 vs 广播的端到端指纹必须不同` `left: 6162899113317753717 / right: 6162899113317753717`；**13 passed / 3 failed** | S11、S15、S16 |
| ② | `reduce_span` 里 `end` **恒取 `pcm_len`**（⛔ 不应用 `span.end`） | `s3`：`end=200（含端点）⇒ 半开 [0, 201)` `left: Some(480) / right: Some(201)`；`s13`：`终点＝半开 [10,201)` `left: Some(480) / right: Some(201)`；**14 passed / 2 failed**（S4 与 S14 仍绿 ⇒ 注入作用域只到 `end`，`end=-1` 走 `span == None` 的早退） | S3、S13（终点腿） |
| ③ | `reduce_span` 里起点 **恒取 0.0**（⛔ 不应用 `span.start`） | `s13`：`起点＝offset` `left: Some(0.0) / right: Some(10.0)`；**15 passed / 1 failed**（S3 用 `offset=0` ⇒ 不受影响） | S13（起点腿） |

⇒ ⭐ 三条注入分别**只**打红与它对应的判据 ⇒ 判据各自有牙（不是"一起红"的粗判据）。

#### ⛔ 本步**不做**（如实登记）

`loop_*`／`ampeg_*`／`off_mode`／`group`/`off_by`/`note_polyphony`／模型层绑定（只给 API）／
`PolySynth` 逐位判据／新依赖（⛔ 未加任何依赖；`yeban-app` 的 `sfz_load.rs:238-269` **未动**）。

#### ⭐ 口径分叉：无（本步闭合了 sfz-needs-4）

步 2 的"引擎应用 `span`、离线/摊平不应用"在步 3 被 `reduce_span` 消除 ⇒
S3/S13 的**逐样本最大绝对差 = 0e0** 覆盖**整段**（含显式 `offset`/`end`），
不再是"只覆盖触发量子、夹具必须没有 `offset`/`end`"。

### 13.3 两处"归属"裁决的**落地**（2026-10-16，任务单 `#68`；裁决 R711）

本步**只落地 R711 的两条裁决**（⛔ 不做循环／`ampeg`／`off_mode`／`group`／模型层绑定／池满窃取／
鼓机＋SFZ 同轨／新依赖）：`sfz-needs-1`（轨推子／静音／独奏）＋ `sfz-needs-7`（缺样本 ⇒ 静音）。

#### 甲 · 让 SFZ 声部与鼓机**同径**过轨推子／静音／独奏（`needs-1`）

**勘察（实现前先量"鼓机在哪一步乘轨增益"）**：

| 步 | 位置 | 做什么 |
| :--- | :--- | :--- |
| ① 投影 | `crates/yeban-engine/src/snapshot.rs:914` | `let gain = crate::synth::track_gain_scale(mute, solo, solo_safe, any_solo, volume_db)` ⇒ ⭐ **全仓唯一的轨增益算子**（可闻门 × 推子） |
| ② 折进逐音符 | `crates/yeban-engine/src/snapshot.rs:978` | `velocity_gain(note.velocity) * gain` ⇒ 写进 `ScheduledNote::gain` |
| ③ 交给鼓机 | `crates/yeban-engine/src/synth.rs:1837` | `drums[index].trigger(DrumHit::new(voice, note.start_sample, note.gain))` ⇒ ⭐ **鼓机吃到的就是含轨门的那一份** |
| ③′ 交给复音 | `crates/yeban-engine/src/synth.rs:1866` | `slot.synth.note_on(NoteEvent::new(…, note.gain), tables)` ⇒ 同径 |
| ⛔ SFZ（改前） | `crates/yeban-engine/src/synth.rs`（SFZ 分支） | 只把 `note.velocity` 交给 `trigger_note` ⇒ **轨增益整条没进去**（＝缺陷本身） |

⇒ ⭐ **修法 ＝ 让 SFZ 与鼓机同径**（⛔ **不是**把力度搬到引擎）：把**同一个** `gain` 也写进
`NoteSchedule::track_gain`（`crates/yeban-engine/src/snapshot.rs:991` 的 `with_track_gain(gain)`），
再由 `render_track` 的 SFZ 分支（`crates/yeban-engine/src/synth.rs:1854`）把它交给
`SfzVoicePool::trigger_note(…, track_gain)`，在**触发期**乘在乐器层增益**之外**
（`crates/yeban-engine/src/sfz_instrument.rs:1179` 的 `region.gain_at(note, velocity) * track_gain`）
⇒ ⭐ **两者相乘、⛔ 不重复**（引擎⛔ 不碰力度：SFZ 的 `velocity_gain[128]` 归乐器层）。

⚠ **为什么挂在调度表上而不是槽位上**：`slot.gain_scale` 只服务**环音符**那条路；已调度的
鼓机/复音读的是**同一条调度表**上的 `note.gain` ⇒ 把轨门挂在**同一张表**上，三者
**按构造同径**（也避免"未武装槽位 ⇒ SFZ 静默而鼓机照响"的新不对称）。

#### 乙 · 缺样本 ⇒ 不建声部、去掉"广播到 `samples[0]`"（`needs-7`）

| 位置 | 改法 |
| :--- | :--- |
| `crates/yeban-engine/src/sfz_instrument.rs:682` | 缺 `sample_path` ⇒ `silent_regions += 1` ＋ 路径进 `missing_sample_paths` |
| `crates/yeban-engine/src/sfz_instrument.rs:684` | `sample_index = PreparedRegion::NO_SAMPLE`（⛔ 不再回退到 `samples[0]`） |
| `crates/yeban-engine/src/sfz_instrument.rs:1150` | `trigger_note` 里 `plan.sample(NO_SAMPLE) ⇒ None ⇒ continue` ⇒ 与 `span_bounds() == None` **同径**：⛔ 不建声部 |
| 删 | `fallback_regions` 字段 ＋ `fallback_region_count()` ＋ 读数里的 `+fallback(N)` 一档（**唯一**计数口径＝`silent_region_count()`） |

#### 本步读数（`cargo test -p yeban-engine --locked`；`sfz_instrument` **18 / 18**，lib **296**，㉙ **73 / 73**）

| 判据 | 读数 |
| :--- | :--- |
| ⭐ **S17 推子 ⇒ SFZ 与鼓机同比例** | 峰值：SFZ 臂 `0 dB = 0.429437995` / `−6 dB = 0.215228811` / 静音 `0` / 别轨独奏 `0`；鼓机臂 `0.766429424` / `0.384124607` / `0` / `0`；⭐ 比值 **SFZ `0.501187146` vs 鼓机 `0.501187205`**（期望 `track_gain(−6 dB) = 0.501187205`，相对误差 ≤ `1e-5`）；静音／独奏两档**逐位** `0` |
| ⭐ **S18 引擎⛔ 不乘第二次力度** | `v ∈ {0,32,64,100,127}`：引擎声部路径与**乐器层**一次性渲染**逐位相同**（`0x28c31cf8df2ec325` / `0x35e214ace6e19df0` / `0xbae210721e5956bc` / `0xfd159d8c329d0e7f` / `0xa933ddab286bdf5e`，两侧同值）；反向对照：`velocity_gain(127) == 1.0`（恒等）⇒ S1 的 127 那条腿**抓不住**双重计，中段力度才有牙 |
| ⭐ **S15 缺样本 ⇒ 静音 ＋ 计数**（改写） | 缺 `missing.wav`（提供表 `[a.wav]`）：`silent = 1`、`missing = ["missing.wav"]`、`sample_content = distinct(1)+silent(1)`、`sample_index = NO_SAMPLE`、`key=64` **写入 0 帧**（整块逐位 0）；反向对照①：补上 ⇒ 该键**逐位**＝ `b` 的离线参照；反向对照②：`key=60` 那条区域**写入 479 帧**（静音作用域只到缺样本的区域）；空提供表 ⇒ `silent = 2`、写入 **0** 帧 |
| **尺寸** | `SfzVoice = 40`、`SfzVoicePool = 648`、⭐ `SfzPlan = 88`／`PreparedSfz = 88`（原 **96**：删掉 `fallback_regions: usize`）、`SynthEngine = 91984`、`EngineSnapshot = 4480`、⭐ `EngineRuntime = 162832 < 196608`（**逐字未变**） |
| **零分配（`rt_zero_alloc` ㉙）** | 10 001 量子（总能量 **2999.164**，与步 2／步 3 **逐字相同**）＋ 1 次换采样率：四元组 `alloc=0 dealloc=0 lock_blocking=0 lock_waits=0 io_requests=0 io_ops=0`；该档 **73 / 73** |
| **既有判据** | S1 `0e0`／478／478、S1b `0e0`／有声块 4、S2、S3、S4、S5 `0x71dca4dc55b44df5`／`0x6eee378072730521`、S6 `0x5587041753dfe375`、S7、S8、S10、S11、S12、S13、S14 `0xa933ddab286bdf5e`、S16 `0x10e619a422b4b7dd` —— ⭐ **逐字不变** |

#### ⭐ 注入打红（三处，各自的**红读数** ＋ 还原自证）

还原自证：注入前记录 4 个文件的 `sha256`（`sfz_instrument.rs = c2a36c5e…a8142871` 等）＋
`git diff | wc -c = 51590`；每次还原后 `sha256sum -c` **四行全 OK**、`git diff` 字节数
**回到 51590**、四个文件的 `INJECT` 标记数 **0**。

| # | 注入 | 红读数 | 变红的判据 |
| ---: | :--- | :--- | :--- |
| ①（甲·防双重计）| `trigger_note` 里把增益改成 `region.gain_at(note, velocity) * track_gain * crate::synth::velocity_gain(velocity)` | `s18`：`力度 32：引擎声部路径必须与乐器层一次性渲染逐位相同` ⇒ `left[1] = 976564401 / right[1] = 993248848`（v=127 那条仍绿）；**17 passed / 1 failed** | S18（⛔ **只**打红它） |
| ②（甲·丢轨门）| `trigger_note` 里⛔ 不乘 `track_gain`（`let _ = track_gain;`） | `s17`：`[SFZ 臂] 0 dB = 0.429437995 / −6 dB = 0.429437995 / 静音 = 0.429437995 / 别轨独奏 = 0.429437995`，`比值 SFZ = 1.000000000 vs 鼓机 = 0.501187205`；**17 passed / 1 failed** | S17（两臂比值分叉 ＋ 静音/独奏档不再为 0） |
| ③（乙·广播回来）| `prepare_with_samples` 缺样本时**回退到 `samples[0]`**（旧行为）| `s15`：`assertion left == right failed: ⭐ 缺样本的区域计数` ⇒ `left: 0 / right: 1`；**17 passed / 1 failed** | S15 |

⇒ ⭐ 三处注入各打红**与它对应**的判据（S18／S17／S15 互不代红）⇒ 判据各自有牙。

#### ⚠ 因本单改变／需解释的既有读数（其余逐字不变）

| 项 | 改前 → 改后 | 原因 |
| :--- | :--- | :--- |
| `sfz_instrument` 档 **16 → 18** | 16 passed → 18 passed | ⭐ 新增 S17／S18（本单要求的判据） |
| S15 名称与期望 | `s15_a_missing_sample_falls_back_and_is_counted` → `s15_a_missing_sample_is_silent_and_is_counted` | ⭐ **旧期望固化了错内容**（"回退到 `samples[0]`"），按 R711 裁决如实改写 |
| `S9` 的 `SfzPlan`／`PreparedSfz` | `96` → **`88`** | 删掉 `fallback_regions: usize`（8 B）⇒ ⭐ **变小**；`EngineRuntime = 162832 < 196608` **未变** |
| doctest 行号 | `snapshot.rs - snapshot::SnapshotReader (line 1494)` → **(line 1499)** | `snapshot.rs` 的文档注释 ＋5 行（该 doctest 是 `ignored`，⛔ 不是判据） |
| `[sfz] 两次解析的区域表地址` | 两次相同 → 两次不同 | 只**打印**（无断言）；堆地址随分配器/进程布局而变 |
| `[engine-stats-mirror/J15] 读次数` | `668719` → `669885` | ⭐ **窗口内并发读计数量**：同树连跑两次实测 `669972` / `671614` ⇒ 随线程调度漂移，⛔ 与本单无关 |
| `[MUST-GATE-012]` 的 `drain_calls` / `backlog` / `max_quanta_between_drains` | e.g. `backlog 514 → 513` | ⭐ 同一个 run 里"高频交换"与"高频交换（复跑）"本来就不同（514/513）⇒ 调度相关读数 |
| `[perf]` 墙钟／`[device-clock] elapsed` | 小幅浮动（如 `摊平一次 165 → 37 µs`） | ⭐ 墙钟噪声；**内存 4544 字节**与 **SFZ 总能量 748.967** 逐字不变 |
| `[assertion-bounds]` 行 | 逐行出现位置变（`… entered` 与测试名粘连不同） | ⭐ stdout 交错（计数读数 `R118 4/4`／`R146 5/5` **不变**） |

⛔ **本步不做**（如实登记）：`loop_*`／`ampeg_*`／`off_mode`／`group`/`off_by`/`note_polyphony`／
模型层绑定（`needs-5`）／池满窃取（`needs-2`，需 ADR）／鼓机＋SFZ 同轨（`needs-3`）／
新依赖（⛔ 未加）／推远端。⚠ 边界：`yeban-sfz`／`yeban-app`／`yeban-model`／`schema` **一个字未动**
⇒ 只跑 engine 四门（＋仓级 `light`）。

### 13.4 两条同族缺口：池满**窃取**（`sfz-needs-2`）＋ 三件同轨的**可读信号**（`sfz-needs-3`）（2026-10-16，任务单 `#70`）

本步只做这两条：⛔ 不碰循环／`ampeg`／`off_mode`／`group`/`off_by`／`note_polyphony`／
模型层绑定（`needs-5`）／`needs-8`／新依赖。⚠ **`yeban-sfz` 一个字未动**（见下"裁定"）。

#### 甲 · 勘察：三处窃取口径（逐条带 `文件:行号`）

| # | 口径 | 位置 | victim 键 | 淡出 |
| --: | :--- | :--- | :--- | :--- |
| ① | `yeban_dsp::polysynth::PolySynth`（**引擎已用**） | `crates/yeban-dsp/src/polysynth.rs:804-811`（`steal_priority`）、`:1205-1217`（三级比较）、`:1218`（`voice_steals`）、`:1221-1245`（软窃取） | ⭐ **三段** `(层级, 触发序号, 下标)`：层级 `0` ＝ Release **或** 包络 < `0.001`（−60 dBFS）、`1` ＝ 其余；序号 ＝ `start_sample`（最早触发） | ⭐ **软窃取**：victim 在**自己的槽**里 `start_steal_fade`（3 ms）＋ 新音符 `pending` 挂同一槽，淡出走完就地起音 ⇒ 旧淡出与新起音**不同时发声** |
| ② | `yeban_sfz::voice_pool::VoicePool`（**引擎不用它**） | `crates/yeban-sfz/src/voice_pool.rs:500-518`（`select_victim`）、`:741-750`（`victim_key`）、`:18-25`（自述偏离） | **四段** `(层级, 注入电平 dBFS, 触发序号, 下标)`：层级 `0` ＝ retiring **或** Release、`1` ＝ 电平 < −60 dBFS、`2` ＝ 其余 | ⚠ **立即接管**：`note_on` 超限时把 victim 的槽**直接**给新声部、把 victim 句柄**交还调用方**，由渲染器对残余输出淡出 |
| ③ | 引擎每轨 SFZ 池（**本单改前**） | `crates/yeban-engine/src/sfz_instrument.rs`（旧 `trigger_note` 的 `None => self.dropped += 1`） | ⛔ **没有**（池满 ⇒ 丢弃 ＋ 计数） | ⛔ 没有 |

⇒ ⭐ **① 与 ② 的差别不在"谁最该被窃"**（两边的第一段都是"已在释放/淡出者优先"），而在
**淡出归谁执行**：① 池**内**（槽里同时放淡出者与挂起者）、② 池**外**（句柄交还调用方）。
② 的那个池**不被引擎使用** —— 机械读数：`grep -rn "voice_pool::VoicePool\|yeban_sfz.*select_victim"
crates/yeban-engine/` 在**改动后**命中 **2 条，且两条都是文字**（本节引用的
`sfz_instrument.rs:143`／`:1236` 的说明），**代码引用 0 条**
⇒ ⭐ **引擎里今天只有一份窃取实现（①）**，本单把它作为 SFZ 池的口径。

#### 甲 · 裁定与做法（`sfz-needs-2` 闭合）

**裁定**：引擎每轨 SFZ 池**采纳 ①（PolySynth 三段口径）**——它已经在实时路径上、已有判据
（`tests/steal_fade.rs`）；② 的 `select_victim` 标为**它自己那一份**（那个池不被引擎使用
⇒ ⛔ 不构成"两份实现"）。落地：

| 件 | 位置 | 做法 |
| :--- | :--- | :--- |
| `FadeRamp` | `sfz_instrument.rs:960-1020` | 3 ms 指数淡出的一段斜坡（`FLOOR^(n/N)`，与 `yeban-sfz` 的 `StealFade` **同形状**）；`NONE` 的 `decay == 1.0` ⇒ 非淡出路径**逐位不变** |
| `mix_span_into` | `sfz_instrument.rs:1043-1066` | **唯一**混音循环多收一个 `&mut FadeRamp`：`(sample × gain) × fade.gain`；淡出走完即 `break`（把剩余帧交还调用方） |
| `SfzVoicePool` | `sfz_instrument.rs:1224-1600` | 每槽多：`pending`（挂起声部）＋ `pending_active` ＋ `fade` ＋ `order`；池多 `order_counter` / `steal_fade_frames` / `dropped` / `stolen` |
| `select_victim` | `sfz_instrument.rs:1433` | ⭐ **三段键** `(层级, 触发序号, 槽位下标)`；层级 `0` ＝ 正在淡出（PolySynth 那一档里本池**可判**的那一条：本池⛔ 无包络、也⛔ 无电平注入） |
| `steal` | `sfz_instrument.rs:1400` | 池满 ⇒ 软窃取（victim 淡出 ＋ 新声部挂同槽）；帧数 `0` ⇒ 硬窃取（注入用）；⭐ **挂起者被再一次换掉 ＝ 真丢弃**（计 `dropped`，⛔ 不静默） |
| `render_slot` | `sfz_instrument.rs:1467` | **帧精确**提升：淡出在**本块内**走完 ⇒ 就地接管、用同一段循环继续写剩下的帧 |
| 3 ms 舵 | `synth.rs:1045-1051`、`set_sample_rate` | 与复音**同一把** `steal_fade_frames`（默认 `steal_fade_frames_for(采样率)` ＝ 144 @ 48 kHz）⇒ ⛔ 不是第二份常数 |

⭐ **同时裁掉模块头登记的那处"立即接管 vs 额外 slack 槽"**（`voice_pool.rs:18-25` 的自述偏离）：
**slack 就在槽内**（`pending` 与 victim 共用一个槽，与 `PolySynth` 的 `PendingNote` 同款）
⇒ 定容池不需要第 17 个槽、也不必"立即接管"。⚠ 该裁定**只落在引擎侧**：`yeban-sfz` 的
那个池仍保留它的"立即接管 ＋ 句柄交还"（那是它与**自己的渲染器**的契约）⇒ 文件未动。

#### 乙 · 勘察与裁定（`sfz-needs-3` 闭合）

**勘察**：分派次序是 **鼓机 ＞ SFZ ＞ 复音**（`synth.rs:1894` 触发、`:1898`／`:1907`／
`:1914` 一带渲染）⇒ 三件同轨时鼓机赢、SFZ 池被 `slot.sfz.silence()` **释放**
（既有判据 S7 钉住"输出对只有鼓机**逐位相同**"）。

**裁定**：**维持**该次序（与 `crates/yeban-engine/src/drums.rs` §4.1"一条轨至多一件音源"
的既有裁决一致）⇒ ⛔ **不改成叠加**。真正缺的是"**用户看得出被忽略了**" ⇒ 落地为一个
**投影读数**（⛔ 不改输出）：

| 件 | 位置 | 做法 |
| :--- | :--- | :--- |
| `armed_sources: [u8; 16]` | `synth.rs:960` / `:987` / `:1889` | `render_track` 每量子写一次：复音**恒在场**（≥ 1）＋ 鼓机 `+1` ＋ SFZ `+1` |
| `SynthEngine::armed_sources` | `synth.rs:1062` | `None` ＝ **还没渲染过**（⛔ 不是"0 件"） |
| `SynthEngine::ignored_sources` | `synth.rs:1077` | ＝ `armed − 1` ⇒ 三件同轨 **`2`**、鼓机＋复音 `1`、只有复音 `0` |

#### 本步读数（`cargo test -p yeban-engine --locked`；`sfz_instrument` **20 / 20**、全套 **501**、㉙ **73 / 73**）

| 判据 | 读数 |
| :--- | :--- |
| ⭐ **S8 池满 ⇒ 窃取**（改写） | 17 条：`建出声部 = 16；活声部 = 16；窃取 = 1；真丢弃 = 0；挂起 = 1`；33 条：`建出声部 = 16；窃取 = 17；真丢弃 = 16；挂起 = 1`；反向对照 15 条 ⇒ `窃取 = 0`／`丢弃 = 0` |
| ⭐ **S19 窃取最旧 ＋ 3 ms 淡出**（新增） | 池满触发：`新建 = 0；窃取 = 1；丢弃 = 0；挂起 = 1；槽 0 淡出剩余 = 144；槽 0 在场音高 = 60`；淡出：`out[0] = 0.620001256`、`out[143] = 0.000650469`（**逐位** ＝ `out[0] × FLOOR^(143/144)`）、比值 `0.001049142`；走完 144 帧 ⇒ 槽 0 音高 `60 → 64`（接管者**真的出声**）；硬窃取对照臂：槽 0 **立刻** `64`、`out[143]` **逐位** ＝ `out[0]`、指纹 `0x0309113d6b8cdef3`（软）vs `0xf3e3c6f4c00a9825`（硬）≠ |
| ⭐ **S20 三件同轨的可读信号**（新增） | 三件同轨：`武装 = Some(3)；被忽略 = Some(2)`（输出对只有鼓机**逐位相同**）；反向对照：鼓机＋复音 `Some(2)／Some(1)`、只有复音 `Some(1)／Some(0)`、**没渲染过** `None` |
| **尺寸（重打）** | `SfzVoice = 40 B`（未变）、`SfzVoicePool = 1712`（原 648）、`SfzPlan = 88`／`PreparedSfz = 88`（未变）、`SynthEngine = 109152`（原 91984）、`EngineSnapshot = 4480`（未变）、⭐ `EngineRuntime = 180000 < 196608`（余量 **16608** 字节） |
| **零分配（`rt_zero_alloc` ㉙）** | 10 001 量子：四元组 `alloc=0 dealloc=0 lock_blocking=0 lock_waits=0 io_requests=0 io_ops=0`；⭐ 总能量 **2999.164**（与步 2／步 3／`#68` **逐字相同**）；该档 **73 / 73** |

#### ⭐ 注入打红（两条，各**只**打红本单的判据）

还原自证：注入前记录 `sha256`（`sfz_instrument.rs = 63156722…`、`synth.rs = 980a7759…`、
test 档 `sfz_instrument.rs = 0d5bb69c…`；⭐ 均为 `cargo fmt -p yeban-engine` **之后**的读数）
＋ `git diff -- <三个文件> | wc -c = 61172`；每次还原后 `sha256sum -c` **三行全 OK**、
该字节数**回到 61172**、三个文件里 `INJECT` 标记数 **0**。

| # | 注入 | 红读数 | 变红的判据 |
| --: | :--- | :--- | :--- |
| ①（甲）| `trigger_note` 的池满分支**改回丢弃**（`None => dropped += 1`） | `s8`：`⭐ 第 17 条区域必须**窃取**一个槽` ⇒ `left: 0 / right: 1`（打印 `窃取 = 0；真丢弃 = 1；挂起 = 0`）；`s19`：`left: 0 / right: 1`（打印 `槽 0 淡出剩余 = 0；槽 0 在场音高 = 60`）；**18 passed / 2 failed** | S8、S19（⭐ 只这两条） |
| ②（乙）| `render_track` 里 `armed_sources[index] = 1`（取走信号） | `s20`：`⭐ 鼓机 ＋ SFZ ＋ 复音 ⇒ 武装 3 件` ⇒ `left: Some(1) / right: Some(3)`；**19 passed / 1 failed** | S20（⭐ 只这一条） |

#### ⚠ 因本单改变／需解释的既有读数（其余逐字不变）

`cargo test -p yeban-engine --locked -- --nocapture` 改动前后各跑一次 ⇒ **排序归一化 diff**
（`sort` ＋ 折叠 `finished in …s`、`0x…` 字面量）共 **107 行**（`<` 31 / `>` 38；其中**内容行**
**69**，其余 **38** 行是 diff 的 hunk 头/分隔线），逐条归类：

| 类 | 行 | 处置 |
| :--- | :--- | :--- |
| ⭐ **本单应改** | **20 行**（`<` 7 / `>` 13）：`[engine-sfz/S8]`（1 行 → 2 行）、`[engine-sfz/S9]`（648 → 1712／91984 → 109152／162832 → 180000）、`[engine-7/J17]` 与 `[engine-osc] O2` 的 `EngineRuntime`（162832 → 180000）、`running 18 tests → 20`、`result 18 → 20 passed`、S8 测试名、新增 S19／S20 两行 | ⭐ 池满了多 16 槽的挂起/淡出/序号状态 ＋ 两条新判据 ⇒ 尺寸与条数**应当**变；已在 S9 里逐字钉住新值 |
| 噪声·调度 | `[MUST-GATE-012]` 的 `quanta`／`drain_calls`／`drained/pruned`／`backlog`／`wall_ms`／`audio_s`、`[engine-stats-mirror/J15] 读次数 671043 → 672417` | ⭐ 窗口内并发读数（与 §13.3 记的 668719 → 669885 **同一类**）；与 SFZ 池无关 |
| 噪声·墙钟 | `[perf]` 四行（如 `SFZ 有排程 8 秒：12.780 → 13.171 ms`、`渲染 8 秒音频：1.703 → 3.358 ms`）、`[device-clock] elapsed`、`all doctests ran in 0.48s/0.49s` | ⭐ 微基准/墙钟；⚠ **独立复跑 `sfz_arm_perf` 三次**得 `5.108 / ~13 / ~13 ms` 的散布 ⇒ 那 2× 摆动是**当时后台编译的负载**，不是回归；⭐ 能量 `748.967`、内存 `4544` **逐字不变** |
| 噪声·地址 | `[sfz] 两次解析的区域表地址` | 堆地址随分配器布局变 |
| 噪声·stdout 交错 | `[assertion-bounds]`／`[mixer] 淡化帧数` 等行与 `test … ok` 粘连（`entered`／`] entered`）、开头两个空行 | ⭐ 并行测试的 stdout 交错；内容本身未变（`淡化帧数 48k=1440 96k=2880 44.1k=1323` 逐字仍在） |

⇒ ⭐ **没有"无法归类"的行**：`<`／`>` 的每一条都落在上表的**本单应改**或**已归类的噪声**里。

⛔ **本步不做**（如实登记）：循环／`ampeg_*`／`off_mode`／`group`/`off_by`/`note_polyphony`／
模型层绑定（`needs-5`）／字节预算与流式（`needs-8`）／新依赖／推远端。
⚠ 边界：`yeban-dsp`／`yeban-sfz`／`yeban-model`／`yeban-app`／`schemas` **一个字未动**
⇒ 只跑 engine 四门（＋仓级 `light`）。

### 13.5 摊平表的**字节预算读数 ＋ 判据**（`sfz-needs-8` 的"可判"那一半）（2026-10-16，任务单 `#84`）

⭐ **本片要的不是流式，而是"让它可判"**：一条**读数**（拆成区域表／PCM／路径文本）
＋ 一条**判据**（`bytes ≤ N`，`N` 的来源可复算）＋ 一条**超预算口径**（引擎只报数、控制侧决定）。
⚠ **流式仍未做** ⇒ `needs-8` 只**部分闭合**（见 needs 表）。

#### 一 · 现状读数（对象 ＝ 一份 `PreparedSfz`；单位 ＝ **字节**）

拆开的三条换算（**唯一**一份求和，`SfzByteUsage::total_bytes` ＝ 既有 `memory_bytes()`）：

| 项 | 换算 | 广播夹具（S6：2 区域） | 预算夹具（S25：3 区域、2 路径） |
| :--- | :--- | ---: | ---: |
| 区域表 | `size_of::<PreparedRegion>() × 区域数` | `4232 × 2` ＝ **8 464** | `4232 × 3` ＝ **12 696** |
| PCM | `4 × 去重后总帧数` | `4 × 480` ＝ **1 920** | `4 × (480 + 1920)` ＝ **9 600** |
| 路径文本 | `Σ path.len()`（广播路径为空串 ⇒ 0） | **0** | `5 + 5` ＝ **10** |
| **合计** | ＝ `memory_bytes()` | **10 384** | **22 306** |

⚠ **本表在 `#104` 前有两处陈旧**（`#104` 一并修）：① 区域表那一列在 `#91`～`#100` 后应逐级为 `2632／2656／2664／2672`，而 `#100` 只改了"预算夹具"那一格 ⇒ `#104` 起两列**同时**给现值；② **合计**列停在 `#92` 的 `17 482`（＝ 3×2624 ＋ 9600 ＋ 10 的**步 3 前**读数）⇒ 现值是 **22 306**（⭐ `#106` 再把它推到 `4232 / 12696 / 22306`；沿革见 §13.5 的 S25 行）。

⭐ **"今天有没有任何判据/读数在盯这个数？"—— 读数有，判据（上界）没有**（本单前的清点，逐处）：

| 点 | 形状 | 是不是上界判据 |
| :--- | :--- | :--- |
| `PreparedSfz::memory_bytes` ／ `SfzPlan::memory_bytes` | 读数（`usize` 字节） | ⛔ 不是（无比较） |
| `tests/sfz_instrument.rs` S6 | **打印**"区域 2、内存 10384 字节"（⭐ `#106` 实测值；沿革 `#104` 前 7168 ⇒ `#104` 10304） | ⛔ 不是 |
| `tests/sfz_arm_perf.rs` 两处 | **打印**"内存 14616／6152 字节"（⭐ `#106` 实测值：3 区域那条 ／ 1 区域那条；沿革 `#104` 前 9792／4544 ⇒ `#104` 14496／6112） | ⛔ 不是 |
| `crates/yeban-app/tests/sfz_ui_selector.rs` | `memory_bytes > 0` | ⛔ **地板**，不是上界 |
| `crates/yeban-app/src/sfz_load.rs`（`plan_bytes`） | 控制侧读数（供 UI／诊断） | ⛔ 不是 |
| `tests/sfz_instrument.rs` S12 | `pcm_bytes() == 1 × 480 × 4` | 去重口径的**精确等式**，与"整库上限"无关 |

⇒ ⭐ 结论：**改动前，一条 1 GB PCM 的音色库会被整段读进内存并 clone 进摊平表，而没有任何
判据会红**；唯一的"读数"是 `memory_bytes()`（只是在几个测试里打印，在 app 侧透出）。

#### 二 · 预算口径与 `N` 的来源

* `SfzByteBudget`：三条**独立**上界（区域表 ／ PCM ／ 路径文本）—— 一个总数会让"区域表悄悄
  长大"与"PCM 爆炸"在读数上不可区分；
* `SfzByteBudget::for_layout(regions, pcm_frames, path_bytes)`：由**声明的形状**按
  `region_bytes` ／ `pcm_bytes` ／ `path_bytes` 那**同一份**换算反推 ⇒ `N` **恰好**等于该形状的
  精确字节数（⭐ **判定是闭区间**：恰好等于上界**通过**，与 `yeban-decode::PcmBudget` 同口径）。
  于是判据钉住的是"**读数里没有隐藏的第四份开销**"（`Vec` 容量余量、或有人往
  `PreparedRegion` 里加字段却没进读数），⛔ 不是拍出来的阈值；
* `SfzByteBudget::new(...)`：控制侧直接给上界。⭐ **本 crate 不自带默认上限**：要用产品要求
  推导的上限（如 `PcmBudget::max_pcm_bytes`），把那个数**传进来**，⛔ 不复制那份常数。

#### 三 · 超预算怎么办 —— 裁决：**引擎只报数，控制侧决定**

`PreparedSfz::budget_overrun(&SfzByteBudget) -> SfzBudgetOverrun`：⛔ 不报错、⛔ 不计数、
⛔ 不截断、⛔ 不改摊平结果，只给逐项超出字节数（`is_within()` ＝ `bytes ≤ N`）。三条理由：

1. **在摊平里报错来得太晚**：`prepare_with_samples` 拿到的是**已经解码**的
   `InMemorySample`（峰值内存**在调用之前**就已花掉一次，本函数只是 clone 一份）
   ⇒ 在那里报错挡不住峰值，只把"载入成功但偏重"变成"载入失败"；
2. **归属先例**：`R711`（`needs-7`）已裁决"引擎出事实＋计数、产品决定交控制侧"——
   字节预算是同一条分界线（引擎有"占多少字节"的事实，控制侧有"值不值得载入"的判断）；
3. **⛔ 不动既有判据**：强制预算必须改 `prepare*` 的返回形状或加参数 ⇒ 既有调用点
   （`SfzPlan::prepare*`、`sfz_load.rs`、S11～S16）全部要动，本单明确⛔ 不改既有类型/签名。

#### 四 · 本片读数（`cargo test -p yeban-engine`，新增三条）

| 判据 | 读数 |
| :--- | :--- |
| ⭐ **S25 读数拆开** | 区域表 `size_of::<PreparedRegion>() 4232 B × 3 = 12696`；PCM `4 B × 2400 = 9600`；路径 `10`；**合计 22306 ＝ `memory_bytes()`**（沿革：`#91` 之前 2624/7872/17482 ⇒ `#91` 2632/7896/17506 ⇒ `#92` 2656/**7968**/**17578** ⇒ ⭐ `#94` 2664/**7992**/**17602** ⇒ ⭐ `#100` 2672/**8016**/**17626** ⇒ ⭐ ⭐ `#104` 4192/12576/22186 ⇒ ⭐ ⭐ `#106` **4232**/**12696**/**22306**） |
| ⭐ **S25 预算判据** | `N = for_layout(3, 2400, 10)` ＝ `{区域表 ≤ 12696，PCM ≤ 9600，路径 ≤ 10}`；实际 `{12696, 9600, 10}` ⇒ 超出 `{0, 0, 0}`、`is_within = true` ⚠ 区域表那一维**自动跟着 `size_of` 走**（判据是现读，⛔ 不是字面） |
| ⭐ **S25 内置反向腿** | 三个维度各 −1 B ⇒ `{区域表 1,0,0}`／`{0,1,0}`／`{0,0,1}`，`is_within = false`（⛔ 绿不是空的） |
| ⭐ **S26 超预算口径** | `N = for_layout(3, 480, 10)` ⇒ 超出 `{区域表 0，PCM 7680，路径 0}`、合计 **7680** 字节、`is_within = false`；⭐ `prepare_with_samples` 仍 **`Ok`**、样本数 2、PCM 9600、区域 3 |
| ⭐ **S26 零预算** | `new(0, 0, 0)` ⇒ 超出 `{12696, 9600, 10}`、合计 **22306**（⭐ 只报数：表一字未改；⭐ `#106` 实测值，沿革 `#84` `{7872, 9600, 10}`/`17482` ⇒ `#104` `{12576, 9600, 10}`/`22186`） |
| ⭐ **S26 摊平结果逐位不变** | `byte_usage {12696, 9600, 10} → {12696, 9600, 10}`；`sample_count 2 → 2`；`pcm_bytes 9600 → 9600`；`region_count 3 → 3`；渲染 `(写入 512, 非零 511) → (写入 512, 非零 511)` |
| ⭐ **S27 1 GiB 级（⛔ 不真的分配）** | 读数 `{区域表 335872，PCM 1073741824，路径 1536}` vs 预算 `{335872, 536870912, 1536}` ⇒ 超出 `{0, 536870912, 0}`、`is_within = false`；⭐ 上界给**恰好 1 GiB** ⇒ **通过**（闭区间） |

#### 五 · 注入打红（三条，逐条还原自证）

还原自证：注入前 `sha256`（`src/sfz_instrument.rs = 2df79766…`、test 档 `sfz_instrument.rs = 7b50dc3a…`）
＋ `git diff` 字节数 ＝ **45309**；每次还原后 `shasum -a 256 -c` **三行全 OK**、该字节数**回到 45309**、
两个文件里 `INJECT` 标记数 **0**。

| # | 注入 | 红读数 | 变红的判据 |
| --: | :--- | :--- | :--- |
| ①（测试侧）| S25 的 `N` 的 PCM 那一维调到**夹具之下**（`for_layout(3, 2400→480, 10)`） | `预算 N … {区域表 ≤ 7872，PCM ≤ 1920，路径 ≤ 10}；实际 {7872, 9600, 10} ⇒ 超出 {0, 7680, 0}（is_within = false）`；`assertion left == right failed: N 的 PCM 那一维必须恰好等于夹具的实际 PCM 字节数` ⇒ `left: 1920 / right: 9600`；**0 passed / 1 failed** | **S25**（⭐ 这条同时是"应绿的反向腿"的注入验证：原本绿的那条腿被打红）|
| ②（库侧）| `SfzByteUsage::overrun` 的 `saturating_sub` 改成**恒 0**（判据失去牙齿） | S25：`超出 {0,0,0}（is_within = true）`、反向腿同样 `is_within = true` ⇒ 红；S26：`超出 {0,0,0}、合计 0 字节、is_within = true` ⇒ 红；S27：`超出 {0,0,0}、is_within = true` ⇒ 红；**24 passed / 3 failed** | **S25／S26／S27**（⭐ 只这三条）|
| ③（测试侧）| S26 的超预算臂把预算**放宽到恰好装下**（`for_layout(3, 480→2400, 10)`） | `N = for_layout(3, 480, 10) ⇒ 超出 {0,0,0}、is_within = true`；`panicked … ⭐ 这份夹具对「只给 a.wav」的预算必须**超**（否则本判据无话可说）`；**0 passed / 1 failed** | **S26** |

#### 六 · 状态差集（任务单第 241 条）

`cargo test -p yeban-engine --locked`（默认捕获模式 ⇒ 结果行不被 stdout 交错污染），改动前／后各一次：

| 量 | 改动前 | 改动后 |
| :--- | ---: | ---: |
| `(二进制, 判据, 状态)` 三元组 | **509**（26 个二进制） | **512**（26 个二进制） |
| 消失 | — | **0** |
| 出现 | — | **+3**：`tests/sfz_instrument.rs` 的 `s25_…`／`s26_…`／`s27_…`（均 `ok`） |
| 状态翻转 | — | **0** |

⇒ ⭐ **差集恰好等于本单应新增的三条**（无一条既有判据消失或翻转）。
⭐ 注入 ② 的差集同样是**恰好 3 条翻转**（`s25`／`s26`／`s27`：`ok → FAILED`，0 消失／0 出现）。

#### 七 · 尺寸与零分配（⛔ 本单一个字段都没加）

| 判据 | 读数 |
| :--- | :--- |
| 尺寸照打（S9） | `SfzVoice 40` ／ `SfzVoicePool 1712` ／ `SfzPlan 88` ／ `PreparedSfz 88` ／ `SynthEngine 109152` ／ `EngineSnapshot 4496` ／ ⭐ **`EngineRuntime 180000 < 196608`**（余量 16608）—— 与 `#70` 逐字相同 |
| `rt_zero_alloc` 全部场景 | **73 / 73**；⭐ ㉙（每轨 SFZ 音源 10 001 量子＋1 次换采样率）`四元组[alloc=0 dealloc=0 lock_blocking=0 lock_waits=0 io_requests=0 io_ops=0]`、⭐ 总能量 **2999.164**（逐字不变）；`[sfz] 回调窗口：allocations=0 deallocations=0 lock_blocking=0 io_ops=0` |
| 既有读数未扰动 | S6 摊平表 **7168 B** ／ SFZ 臂指纹 `0x5587041753dfe375` ／ `[perf] 摊平一次 4544 B`、`逐样本差 0e0（内存 9792 B）`（⚠ 本行是**任务单 `#84` 当时**的读数：那三个字节数后来随 `PreparedRegion` 长大而**移动**，⛔ **不是未扰动**；现值（⭐ `#106` 实测）＝ `S6 **10384 B**`／`[perf] 摊平一次 **6152 B**`／`逐样本差 0e0（内存 **14616 B**）`（沿革：`#104` 为 10304／6112／14496），**指纹 `0x5587041753dfe375` 逐字不变**） |

#### 八 · needs 表更新

* `yeban-engine/src/sfz_instrument.rs` 模块头的 `sfz-needs-8` 行 ⇒ **部分闭合**（"可判"闭合、"流式"未做），
  并把 `SfzByteBudget` 一族加进"件"表；
* 本文档的 needs 表 `sfz-needs-8` 行 ⇒ 同步**部分闭合**（同口径）。

⛔ **本片不做**（如实登记）：**真流式**（`Arc` 共享／分页读盘／流式重采样）／
**控制侧在解码之前**设闸门（那是 app 侧，本单⛔ 不动）／模型侧／`ampeg_*`／`off_mode`／
`group`/`off_by`／新依赖／推远端。
⚠ 边界：`yeban-dsp`／`yeban-sfz`／`yeban-model`／`yeban-app`／`schemas`／`scripts` **一个字未动**
⇒ 只跑 engine 四门（＋仓级 `light`）。
