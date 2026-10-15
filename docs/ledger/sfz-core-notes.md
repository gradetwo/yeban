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
| `PreparedRegion` 结构字节数 | **2624 B**（原 548 B；代价来自 5 张 128×`f32` 表 + `SampleSpan`） |


## 13. 引擎侧声部 ＋ 快照形状（2026-10-15，任务 `#66` 的**步 2**）

设计裁决见 `docs/ledger/phase-status.md` 的 R698（裁决 **甲-1**）：`TrackSlot` 增
`sfz: SfzVoicePool`（`[SfzVoice; 16]`、全 `Copy`）＋ 快照侧只读 `PreparedSfz`
＋ `render_track` 加第 3 参 `sfz: Option<&PreparedSfz>`（与既有 `Option<&NoteSchedule>` 对称）。
本片＝**引擎侧发声器**那一半；**绑定来源**（轨↔SFZ 的模型语义）仍未做。

### 交付形状

| 件 | 归属 | 说明 |
| :--- | :--- | :--- |
| `SfzPlan` | 快照（`BTreeMap<EntityId, SfzPlan>`） | 控制侧投影；重数据（区域表 ＋ PCM），与 `insert::ConvolutionPlan` 同族 |
| `SfzVoice` / `SfzVoicePool` | `SynthEngine` 的轨道槽 | 定长 `[SfzVoice; 16]`，全 `Copy`；池满**丢弃 ＋ 计数**，⛔ 无窃取 |
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
| 池满（17 条匹配区域） | `created = 16`、`dropped = 1`（⛔ 不静默） |
| 尺寸（步 2） | `SfzVoice = 32 B`、`SfzVoicePool = 520 B`、`SynthEngine = 89936`、`EngineRuntime = 160784 < 196608`（192 KiB）⇒ ⭐ 步 3 改为 **40 / 648 / 91984 / 162832** |
| 零分配（`rt_zero_alloc` ㉙） | 10 000 量子（总能量 2999.164）＋ 1 次换采样率：四元组 `alloc=0 dealloc=0 lock_blocking=0 lock_waits=0 io_requests=0 io_ops=0`；该档 **73 / 73**（⭐ 步 3 后逐字不变） |
| 摊平表内存 | 2 区域 ＋ 480 帧 PCM = **7168 B**（广播路径；步 3 后逐字不变） |

### needs（⭐ 下表状态截至**任务单 `#68`**，即 2026-10-16 的 R711 落地；权威表在 `yeban-engine/src/sfz_instrument.rs` 模块头）

| # | 缺口 | 状态 |
| :--- | :--- | :--- |
| sfz-needs-1 | **轨推子/静音未乘到 SFZ 声部** | ⭐ ⭐ **已闭合（任务单 `#68`；裁决 R711）**：**乐器层**拥有乐器内增益（`volume_gain`／`velocity_gain[128]`／`key_crossfade`／`velocity_fade_*`／`amp_veltrack`）、**引擎层**拥有轨增益（推子／静音／独奏）⇒ **两者相乘、⛔ 不重复**。`snapshot::project_schedules` 把**同一个** `track_gain_scale(...)` 结果写进 `NoteSchedule::track_gain`，`SfzVoicePool::trigger_note` 乘在 `region.gain_at(...)` 之外 ⇒ ⭐ 与鼓机**同径**；S17／S18 钉住（读数见 §13.3） |
| sfz-needs-2 | **池满只丢弃 ＋ 计数，无窃取口径** | **未闭合**：`yeban-sfz` 的 `VoicePool` 是全局 512 声部、victim 选择是另一份口径 ⇒ 要一条 ADR |
| sfz-needs-3 | **鼓机 ＋ SFZ 同轨**（引擎＝鼓机优先，SFZ 池被释放） | **未闭合**：与 `crate::drums` 的 needs-3 同族（模型若禁止这种组合，本条删除） |
| sfz-needs-4 | **`span` 终点引擎侧已应用、离线/摊平侧未应用** | ⭐ **已闭合（步 3）**：三条路共用 `reduce_span` 一份归约 ⇒ `start` ＋ `end` 都应用；S3/S13 逐样本差 **0e0**（读数见 §13.2） |
| sfz-needs-5 | **轨↔SFZ 的模型绑定不存在**（**"SFZ 如何进 `YebanProjectV1`"未裁**） | **未闭合（模型面）**：本步只把它做成**可经公开路径调用的 API**（`SfzPlan::prepare_with_samples` → `EngineSnapshot::with_sfz`，S16 端到端）；⛔ 不加 `DeviceKind` 档、⛔ 不改 schema |
| sfz-needs-6 | **循环 / ADSR / `off_mode` / `group`/`off_by`** | **部分闭合**：步 3 只闭合了它的"多采样／每区域独立 PCM"那一半；其余仍完全未做 |
| sfz-needs-7 | ⭐ **（步 3 新增）缺样本的引擎侧回退是"广播到 `samples[0]`"（内容错）** | ⭐ ⭐ **已闭合（任务单 `#68`；裁决 R711）**：**引擎出静音并计数**（`silent_region_count` ＋ `missing_sample_paths`）、**合成替代内容是控制侧的决定**（想给替代样本 ⇒ 调用方显式放进提供表）。`prepare_with_samples` 遇缺失路径 ⇒ 该区域 `sample_index = NO_SAMPLE` ⇒ 触发时**不建声部**（与 `span()==None` 同径）；⛔ 旧"广播到 `samples[0]`"**已删除**；S15 已**如实地**改写期望（读数见 §13.3） |
| sfz-needs-8 | ⭐ **（步 3 新增）摊平表不含样本字节预算 / 流式** | **未闭合**：每个被引用的样本 `pcm` **整段 clone** 进摊平表（去重后 N 份）；无上限、无流式、无 `Arc` 共享口径 |

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
