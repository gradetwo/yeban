# `yeban-theory` 形态 D 判据审计台账（持久镜像）

> **为什么有这个文件（裁决 R101）**：注入扫查、探针读数与残余清单此前只存在于
> `/tmp`（私有目录）—— 而 **`/tmp` 会被系统清理**（第九批实测：整个
> `/tmp/mod-theory` 消失，见裁决 R98）。**私有目录 ≠ 持久目录**，所以把
> **可复核的读数与残余清单**落到仓库里，成为后续批次可消费的台账。
>
> 本文件只记录**读数与结论**；判据本体在 `src/**/mod tests` 与 `tests/*.rs`。

## 1. 非真空断言台账（裁决 R93 / R102 / R106）

**口径**：`for x in <运行期集合> { assert!(性质(x)) }` 在集合为空时**恒真**；
`iter().all(..)` / `iter().any(..)` / `windows(n)` 在元素不足时同样**恒真**。
因此扫描**运行期**集合（库派生、`hits()`、`notes()`、`voicings`、`movements`）
的判据必须自带上界（`len >= N` 或宏隐式相等 `assert_eq!(len, N)`）或**运行期计数器**。

**审计读数（本文件维护）**：

| 批次 | 遍历运行期集合的循环 | 自带下界 | 缺下界 |
|---|---|---|---|
| 第九批初查 | 63（16 处循环 / 14 条判据） | 47 | **14 条判据** |
| 第九批改进扫描器后重查 | 65 | 63 | **1 条**（`genre::every_seeded_sketch_keeps_the_sketch_invariants_and_stays_in_its_own_key`，第九批**漏报**）＋ 1 条扫描器假阳 |
| **第十批处理后** | 65 | **65** | **0** |

**14 条残余清单的处置（全部优先 A：加一行下界）**：

| # | 判据 | 扫什么 | 加的下界 |
|---|---|---|---|
| 1 | `drum::every_onset_carries_a_hihat_and_hits_are_ascending_in_tick_then_voice` | `grid.hits()` / `pattern.hits().windows(2)` | `assert!(!grid.hits().is_empty() && pattern.hits().len() >= 2, …)` |
| 2 | `drum::every_hit_matches_its_grid_onset_bit_for_bit` | `pattern.hits()` / `grid.hits()` | `assert!(!pattern.hits().is_empty() && !grid.hits().is_empty(), …)` |
| 3 | `genre::id_is_lowercase_snake_case_ascii` | `GenreLibrary::all()` | `assert_eq!(GenreLibrary::all().len(), 182, …)` |
| 4 | `genre::get_and_ids_agree` | `GenreLibrary::ids()` | `assert_eq!(GenreLibrary::ids().len(), 182, …)` |
| 5 | `genre::the_seed_selector_can_reach_every_registered_kind` | `all()` | `assert_eq!(…len(), 182, …)` |
| 6 | `genre::registered_progression_and_scale_counts_are_pinned` | `all()` | 同上 |
| 7 | `genre::the_bounded_accessors_answer_every_registered_index_and_nothing_beyond` | `all()` | 同上 |
| 8 | `genre::scale_at_parses_every_registered_name_and_reports_bounds_with_the_existing_error` | `all()` | 同上 |
| 9 | `genre::registered_scale_names_fold_onto_canonical_kinds_except_two_documented_aliases` | `all()` | 同上 |
| 10 | `genre::sketch_at_zero_and_a_zero_picking_seed_match_sketch_bit_for_bit` | `all()` | 同上 |
| 11 | `melody::no_leap_exceeds_the_constraint` | `melody.notes().windows(2)` | `assert!(melody.notes().len() >= 2, …)`（R102） |
| 12 | `melody::strong_beats_take_a_chord_tone_when_the_window_has_one` | `notes()` ＋ `if weight < 1 { continue }` | **运行期计数器** `strong_seen` ＋ `assert!(strong_seen >= 32, …)`（R106） |
| 13 | `melody::genre_melody_for_matches_the_unseeded_api_when_the_seed_picks_the_first_entries` | `all()` | `assert_eq!(…len(), 182, …)` |
| 14 | `properties::every_genre_can_change_its_section_with_the_seed` | `all()` | `assert_eq!(…len(), 182, …)` |
| 15 | `genre::every_seeded_sketch_keeps_the_sketch_invariants_and_stays_in_its_own_key` | `all()` | `assert_eq!(…len(), 182, …)`（**第九批漏报，第十批补**） |
| 16 | `drum::voices_are_a_total_mapping_with_distinct_ordinals_names_and_keys` | `DrumVoice::ALL.windows(2)`（编译期长度） | `assert!(DrumVoice::ALL.len() >= 2, …)`（R102，常量长度也钉） |

**集中式兜底**：`tests/determinism_digests.rs::scan_domains_reach_their_lower_bounds`
把全部运行期扫描域的下界集中断言一次（`all()` 182 / `ids()` 182 / `by_source` 52 /
`by_drum_style` 13 / 网格 `hits()` 12 / 旋律 `notes()` == 网格 onset 数 /
声部 `voicings` 2 与 `movements` 1 / 鼓组非空且 ≥ onset 数）。**集中式 ≠ 每条自证**
（裁决 R97），两者**都要**有；本表是逐条那一半。

**登记的扫描器假阳（不修补，登记为工具局限）**：
* `genre::registry_numeric_aggregates_are_pinned_to_the_measured_readings` —— 无 `len()` 断言，
  但**值界**生效（`all()` 为空则四个求和为 0，与字面量 16814/27018/717/756/3159/11882 不等 ⇒ 必红）。
* `scale::every_scale_kind_has_its_documented_chinese_name` —— `NAMES.iter().any(..)` 无 `len()` 断言，
  但**值界**生效（`assert_eq!(distinct, 14)` 与 16 行 `name_zh` 断言）。
* 文本扫描器的**可绕过性**（裁决 R104）：把断言拆成"先绑定到变量、再断言变量"即可绕过；
  扫描器只覆盖字面形态，**不构成完备保证**。

## 1b. 第十一批补录：五种"界"的形态 ＋ 两个扫描器盲区

**⭐ 可观测的"界"有五种形态**（扫描器只认一种就会漏或假阳）：

| 形态 | 例子 | 条数（第十一批读数） |
|---|---|---|
| ① 显式 `len() >= N` | `assert!(pattern.hits().len() >= 2)` | 3（仅此形态） |
| ② **宏隐式相等** `assert_eq!(len, N)` | `assert_eq!(GenreLibrary::all().len(), 182)` | **24**（仅此形态） |
| ③ `!x.is_empty()` | `assert!(!grid.hits().is_empty())` | 计入"其它" |
| ④ **值界**（求和/计数等于非零字面量） | `assert_eq!(bpm_low, 16814)`、`assert_eq!(distinct, 14)` | 1（仅此形态） |
| ⑤ **运行期计数器** | `assert!(strong_seen >= 32)` | 12 条判据含计数器 |

⭐ **R102 复查（"下界两写法"）**：遍历运行期/计算/过滤集合的判据共 **47** 条；
**只认显式写法**的扫描器会把 **37 条**报成"无界"——**全部是假阳**（24 条只有宏隐式、
12 条有计数器、1 条只有值界）。⇒ 第十批新加的下界**两种写法都用了**
（9 条库域用宏隐式 `assert_eq!(…len(), 182)`；`drum`/`melody` 的域用显式 `len() >= 2` ＋ `!is_empty()`）。

⭐ **扫描器盲区二（本轮实测）**：`!x.is_empty()` **不是** `len() >= N`，
第一版正则不认它 ⇒ 把第十批**已经修好**的 `drum::every_hit_matches_its_grid_onset_bit_for_bit`
误报成"无界"。⇒ **"扫描器报缺"必须先人工核对，再动手改判据**。

## 1c. 第十一批新修的 5 条（R93 剩余面：过滤结果与计算集合）

| 判据 | 扫什么 | 加的下界 |
|---|---|---|
| `genre::every_rule_has_a_plausible_bpm_range_meter_and_names` | `all()` | `assert_eq!(…len(), 182)` |
| `genre::idiomatic_data_is_structurally_valid` | `all()` | 同上 |
| `rhythm::onsets_never_leave_their_own_bar` | `grid.hits()` | `assert!(!grid.hits().is_empty())` |
| `properties::every_genre_produces_a_playable_sketch` | `all()`（循环内断言的是 **spans**，不是域） | `assert_eq!(…len(), 182)` |
| `properties::a_narrow_window_never_produces_an_out_of_window_note` | `melody.notes()`（proptest 体内） | `prop_assert!(!melody.notes().is_empty())` |

⭐ **R106 复查（`filter(...)` ＋ 循环内断言 ＋ 无计数/下界）**：候选 **4** 条，
**全部是假阳** —— 过滤结果**当场与字面量向量/计数比较**
（`assert_eq!(starts, vec![0, 2, 4])`、`assert_eq!(filter(...).count(), 4)`）或由 `seen` 数组计数。

## 1d. R100（外部夹具缺失 = 真空第三种形态）核实

本 crate **没有任何判据读运行期外部资源**：
`std::fs` / `CARGO_TARGET_TMPDIR` / `read_dir` / `File::` / `include_bytes!` / `PathBuf` /
`std::env::var` 在 `src/**` 与 `tests/**` 里的命中**只有两处**：
① `lib.rs` 的文档注释（声明"不做 I/O"）；② `lib.rs::crate_has_no_hidden_nondeterminism_sources`
的**禁用针清单**（`"std::fs::"`、`"std::env::var"` 作为**字符串**）。
⇒ **R100 在本 crate 不适用的结论有代码守卫**（该守卫测试本身会因引入 `std::fs::` 而红）。

## 1e. 工具纪律（第十一批新增）

* ⭐⭐ **`cargo fmt` 会重排插入的长断言** ⇒ "同一份编辑是否已落盘"的幂等检查
  ⛔ **必须在 `cargo fmt` 之后再做一次**（本轮实测：fmt 把单行断言折成 5 行，
  前一次回读因此失配，导致同一处下界被**插入两次**；已删除重复并加"每条判据恰好 1 条界"的后置校验）。
* ⭐ **R103 负向实测**：故意弄脏工作区 ⇒ 驱动器 `REFUSED`（exit 3）。
  本轮第一次跑 R105 探针时，也因工作区有未提交改动而被拒 ⇒ 再次证明守卫有效。
* ⭐ **R92（交换类注入的哨兵三步）在本驱动器里不必要**：多编辑注入按**行号一次性**写入
  （先全部算出新内容，再逐文件整体落盘），不存在"读到半交换状态"的窗口 ⇒ 交换是原子的。

## 1f. 第十二批：R111 的**常驻判据** ＋ R113/R114/R115

⭐ **一次性审计 ⛔ 不等于判据**（R115）⇒ 第十/十一批的扫描器已升为常驻判据
`tests/scan_guards.rs::scan_loops_are_bound_and_the_classifier_has_positive_and_negative_controls`。

**判据内置的东西**：
* ⭐ **R113 逐字节等长**：`mask_preserving_len` 把被掩码的字节**各**换成一个空格字节，
  函数内部断言 `masked.len() == raw.len()`（含多字节 UTF-8 样本：`"中文 ♯"`）。
  若按"每字符一个空格"补，多字节 UTF-8 会缩短字节长度 ⇒ 偏移映射会**静默跳过**。
* ⭐ **R114 按断言自己的实参解析**：以**测试函数体**为作用域（⛔ 不用固定字符窗口），
  每条断言用括号深度感知的 `top_level_args` 切出实参；根绑定按**接收者**
  （`grid.hits()` 与 `grid.len()` 是同一接收者 `grid`；`pattern` 与 `grid` 不是）。
* ⭐ **R56/R112 正负对照**：六种形态**每种一条已知绿**（① 显式 `len() >= N`、
  ② 宏隐式 `assert_eq!(len, N)`、③ `!is_empty()`、④ 值界、⑤ 运行期计数器、
  ⑥ `assert_eq!(<接收者作用域表达式>, 整数字面量)`）＋ **一条"无界"已知红**
  （R112 的正对照：它**必须**被判成无界）。
* ⭐ **R93 非真空**：扫描域下界逐条钉住。

**实测读数（15 个文件、测试区）**：运行期集合循环 **76** 个 ⇒ 接收者绑定 **52** ＋
累加器绑定 **9** ＋ **分类器不认 15**（逐文件钉住：`genre.rs` 3、`voice_leading.rs` 1、
`drum.rs` 2、`properties.rs` 9）。

⭐ **残余清单（R97 形态，可由后续批次消费）**：这 15 个循环的界**已人工核对**，
但形态不在上面六种之内 —— 典型是"把结果收进 `Vec` 再与字面量向量比较"、
"`zip` 两个集合后逐对断言"、proptest 体里的 `prop_assert!` 作用在派生值上。
判据**逐文件钉住计数**，因此**新增一个分类器不认的循环会立刻变红**。

## 1g. 第十二批：R100/R116 的常驻判据

`tests/scan_guards.rs::no_criterion_reads_a_runtime_external_resource`：
对 15 个文件（`src/**` 含测试模块 ＋ `tests/**`）扫描 **10 个运行期外部资源禁用针**
（`std::fs::`／`std::env::var`／`std::env::var_os`／`read_dir`／`File::open`／
`CARGO_TARGET_TMPDIR`／`CARGO_MANIFEST_DIR`／`current_dir`／`tempfile`／
`std::process::Command`），**先掩码再扫**，并带 R56 三条对照（真代码已知红、
字符串内已知绿、注释内已知绿）。

**与既有守卫的分工**：`src/lib.rs::crate_has_no_hidden_nondeterminism_sources` 只看**生产代码**
（到 `#[cfg(test)]` 为止，测试代码故意豁免）⇒ 本判据补的是"**判据自身**不读外部资源"这个缺口。

**R116 的可执行查证**（"不存在"的陈述要现场查证）：
```bash
grep -rnE "std::fs|std::env|read_dir|CARGO_TARGET_TMPDIR|File::|include_bytes!|PathBuf|std::process|Command::new" \
  crates/yeban-theory/src crates/yeban-theory/tests | grep -v '"std::' | wc -l
```
读数：**1** —— 唯一命中是 `src/lib.rs` 的**文档注释**（"没有 `std::fs`"）。
其余命中都在 `lib.rs` 那条守卫的**禁用针字符串**里（被 `grep -v '"std::'` 排除）。

## 1h. 驱动器分类器的已知红（R108）

`/tmp/mod-theory/inj/run*proof.py` 是带**分类器**的驱动器（RED／NOT_RED／COMPILE_ERROR／
TIMEOUT／NO_TEST_RAN）。分类器逐条喂过已知红：
| 判定 | 已知红 | 批次 |
|---|---|---|
| `RED` / `NOT_RED` | 12 对注入（含受控实验） | 8–11 |
| `COMPILE_ERROR` | 去掉 `derive`／把 `impl` 插到 `derive` 之间 | 8、9 |
| `NO_TEST_RAN` | 过滤路径写错（`properties::` 前缀） | 10 |
| `TIMEOUT` | `genre_id_salt` 的 `index += 0` 死循环（25s 超时） | 11 |

⭐ **R108：超时路径必须保留部分输出** ⇒ 驱动器在超时后把已读到的输出尾部记进
`partial_tail`（本批补上），否则"超时"与"没有任何输出"无法区分。

## 1i. 第十三批：R118 把"界"按**是否界定集合大小**重新分类

⭐ **判据**：只有"界定**被遍历集合的大小**"的断言才算界（R118）。

| 形态 | 界定集合大小？ | 例 |
|---|---|---|
| ① `x.len() >= N` / `> N` / `== N` | ✅ | `assert!(grid.hits().len() >= 2)` |
| ② 宏隐式相等 `assert_eq!(x.len(), N)` | ✅ | `assert_eq!(GenreLibrary::all().len(), 182)` |
| ③ `!x.is_empty()` | ✅（下界 1） | `assert!(!melody.notes().is_empty())` |
| ⑤ **每轮无条件** `ident += 1` ＋ 断言 | ✅（等价于下界 N） | `strong_seen += 1; … assert!(strong_seen >= 32)` |
| ④ 聚合/值界 `assert_eq!(bpm_low, 16814)` | ⛔ **不界定**（求和可以是任何值） | 第四批的数值多重集判据 |
| ⑤′ 计数器但**条件**自增（写在 `if` 里） | ⛔ **不界定** | —— |
| ⑥ 接收者作用域的**子集计数** `assert_eq!(metric.hit_count(Kick), 2)` | ⛔ **不界定被遍历集合** | 鼓组计数断言 |

**读数变化**：第十二批（旧口径）76 循环 ⇒ 接收者 52 ＋ 累加器 **9** ＋ 不认 15；
**R118 收紧后** ⇒ 接收者 52 ＋ 累加器 **1** ＋ 不认 **23**。

## 1j. 第十三批：23 条残余清单（逐条登记 ＋ 理由）

⭐ 我尝试过"机械地为 23 条各补一条集合大小界"（脚本插入 **33** 处），**被既有判据当场否证**：
`rhythm::tests::grouped_grids_keep_every_structural_invariant` 变红 —— 该判据**故意**遍历
一个**空网格**来验证"空输入不 panic" ⇒ 对它插 `assert!(!grid.hits().is_empty())` 是**错的**。
⇒ **已全部回退**，改走"逐条登记 ＋ 给理由"这条路（R118 允许的第二条路）。

⭐ **发现的合法例外类**：**"遍历可能为空的集合以验证空输入安全"** —— 这类判据的正确形态
**不是**"断言集合非空"，而是"断言遍历的**夹具列表**非空"（例如断言 grids 列表本身 ≥ 2）。
机械规则无法区分"域应该非空"与"空域是待测输入" ⇒ 这一条必须人工判定。

| 文件 | 条数 | 形态与理由 |
|---|---|---|
| `src/genre.rs` | **7** | 6 条 `GenreLibrary::all()` ＋ 1 条 `by_drum_style(Metric)`：**界写在别处**（同一测试体里的求和/多重集判据，属形态 ④）⇒ 分类器按 R118 **正确地**不认，但域由 `all()` 的长度决定，故另由常驻的 `scan_domains_reach_their_lower_bounds`（第九批）钉住 182/52/13。**登记为"已由集中式判据覆盖"**。 |
| `tests/properties.rs` | **12** | proptest 体：域由 `prop_assert!` 作用在派生值上（非长度）；另有 3 条是**故意遍历可能为空的网格/旋律**（空输入安全）。**登记为"集中式 ＋ 属性测试的生成参数共同覆盖"**。 |
| `src/drum.rs` | **2** | `metric.hits().iter().filter(...)`：界是**子集计数**（形态 ⑥，R118 正确地不认）；被遍历的是过滤后的子集，而**总集合**由 `full_grid(...)` 的长度与 `hit_count` 断言共同确定。 |
| `src/melody.rs` | **1** | `GenreLibrary::all()`：同 `genre.rs`，由集中式判据覆盖。 |
| `src/voice_leading.rs` | **1** | `spans.iter().zip(result.voicings.iter())`：`zip` 的界在**两个**集合上；分类器只按接收者绑定单一集合 ⇒ 登记为"分类器形态不足"，其长度关系由 `voicing_count` 断言覆盖。 |

⭐ **判据仍然有牙**：23 条**逐文件钉住计数** ⇒ 任何新增的"分类器不认"循环立刻变红；
任何"本该被认出的界被删掉"也会让 `rooted`/`counter` 计数变化 ⇒ 红。

## 1k. 第十三批：R119 子串匹配的 near-miss 对照

"接收者绑定"用的是**标识符边界**（按非标识符字符切词后做**词相等**比较），
⛔ 不是 `contains`/`find` 子串匹配。对照（已进常驻判据的用例表）：
```rust
// 循环遍历 gridlines.hits()，界却写在 grid.hits() 上 ⇒ **必须**判成无界
"for hit in gridlines.hits() { assert!(grid.hits().len() >= 2); }"  // expect = false
```
若退回子串匹配，`grid` 会命中 `gridlines` ⇒ 误判为有界。

## 1l. 第十三批：R121 冷/热构建读数（证明相计时必须带构建状态）

`touch` 全部源文件后 `cargo check --all-targets` 的**冷**读数为 **real 7.00s**
（`user 0.68 / sys 4.24`）⇒ 第十二批的假 `TIMEOUT`（30s 预算被冷构建吃掉）有了量化依据：
**证明相的每次计时都必须记录"此次是冷还是热"**；本批的对照注入全部在**热**状态下计时。

## 1m. 第十五批：把残余从 **23 收窄到 1**（R97 闭环完成）

⭐ **收窄动作**（三轮，全部先补**真实**集合大小界，再让分类器认出来）：
| 轮 | 动作 | 残余 |
|---|---|---|
| 起始 | R118 收紧后的读数 | **23** |
| 第 1 轮 | 10 处补集合大小界（`all()` ／ `by_drum_style` ／ `grid.hits()` ／ `pattern.hits()` ／ `spans`+`result.voicings`）＋ 分类器支持**多接收者** | 13 |
| 第 2 轮 | 再补 10 处 `all()` 界（`genre.rs` 7 ／ `melody.rs` 1 ／ `properties.rs` 2） | 5 |
| 第 3 轮 | 补 3 处（`drum.rs` 过滤循环 ×2 ／ `properties.rs` zip）＋ 分类器**剥掉回调** | **1** |

⭐ **分类器两处一般化**（都进了常驻判据）：
1. **多接收者绑定**：`for x in a.iter().zip(b.iter())` 要求 `a` 与 `b` **都**被集合大小界覆盖
   （``assert!(!spans.is_empty() && !result.voicings.is_empty())``）。
2. **剥掉迭代链上的回调**：`.filter(|hit| …)` 里的 `hit` 与路径 `DrumVoice` **不是**集合接收者；
   不剥掉就会要求"每个接收者都有界"，那对过滤循环**不可能满足**（第十五批实测：`drum.rs` 两条）。

⭐ **唯一残余（1 条，且分类器是对的）**：
`tests/properties.rs::every_drum_voice_lands_exactly_where_the_documented_rule_says` 的 `for hit in grid.hits()`。
该判据**故意**遍历可能为**空**的网格（空输入安全）⇒ ⛔ **不能**断言非空。
第十五批的机械插入**又一次被它当场否证**（`properties.rs:1319` 变红）⇒ 已回退并在原位写下注释：
```rust
// ⚠ 本判据**故意**遍历可能为空的 grid（空输入安全）⇒ 这里⛔ 不能断言非空。
```
⇒ **"未认 1 条"是正确结论**，不是缺口。这是第十三批登记的那个**合法例外类**的**具体实例**。

⭐ **机械插入两次被同一类判据否证**（第十三批 33 处、第十五批 1 处）⇒ 规则：
**"遍历可能为空的集合"必须人工判定**；机械规则无法区分"域意外为空"与"空是待测输入"。

## 1n. 第十五批：形态注册表模板（供其它判据复用）

`FORM_REGISTRY` 的做法可复用到任何"**有一组形态/类别 ＋ 每个类别要有见证**"的判据：
1. 形态登记成 `const REGISTRY: [(flag, &str); N]`，每条带**稳定 flag**（位标志）与说明；
2. 每个对照声明它**期望命中**的形态掩码；
3. 断言三条：`registry.count_ones() == N`、`covered == registry`（R160①）、
   `declared == registry`（R160②）；
4. 对照里放**一条已知红**（期望判定与"默认结论"相反），并把**无效样本**（编译错误型红）排除在分母外。

**候选复用点（登记，⛔ 本批不改）**：`tests/golden_tables.rs::every_public_type_keeps_its_trait_surface`
里的"11 个 trait × 32 个公开类型"也是一张**类别注册表**（`Debug`/`Eq`/`Send`/`Sync`/`Copy`/`Ord`/`Hash`/
`Display`/`FromStr`/`Error`/`Default`）—— 它有同样的双向风险（"注册了没人测"／"测了没注册"）。
**爆炸半径**：`crates/yeban-theory/tests/golden_tables.rs`（本 crate 内，属我可写面）；**未在本批动手**。

## 2. R70② 自比台账（"两次运行相同"不是契约）

机械扫出 **13** 条 `assert_eq!(f(x), f(x))`（两侧**源码文本相同**）：
`src/lib.rs` 1（`splitmix64(42,0)`）、`src/genre.rs` 11（`ids`/`search`/`by_scale`/
`by_source`/`source_histogram`/`by_drum_style`/`drum_style_histogram`/`ids`/`search`/
`sketch_for`/自反 `base == base`）、`tests/properties.rs` 1（`realize_three_voices` 两次）。

**取代它们的 7 条字面摘要**（`(长度, FNV-1a 64)`，纯整数路径）：

| 读数 | 长度 | 摘要 |
|---|---|---|
| 声部连接 `I-V-vi-IV-ii-V-I`（3 声部） | 71 | `0xA7DA79AB20EFDB4A` |
| 旋律 C 大调 `I-V-vi-IV`、5 onset/小节、种子 99 | 240 | `0xB0D6E74394B6540E` |
| 节奏网格 7/8、5 小节、6 onset、摇摆 660 | 341 | `0x5128F2EE20ECA6A3` |
| `GenreLibrary::ids()` | 1831 | `0x3594A110B44A0F26` |
| `GenreLibrary::search("jazz")` | 86 | `0xB3F581EFECEBEA05` |
| `GenreLibrary::by_source(traditional…)` | 540 | `0x1AC5762A8471C0DF` |
| `SplitMix64` 前 64 个（种子 0..64、盐 7） | 1305 | `0xDC742D0B957D91AC` |

## 3. 静态扫描器掩码台账（裁决 R94 / R96 / R99 / R104）

同一份源码、三种扫法：

| 扫法 | `assert_eq!` 两侧同文 | `assert_ne!` 两侧同文 |
|---|---|---|
| 不掩码（朴素） | 17 | 3 |
| 只掩**字符串** | 13 | 0 |
| 掩**字符串 ＋ 行注释 ＋ 块注释** | **13** | **0** |

差集逐处定位（`tests/determinism_digests.rs`）：**注释** 第 5 / 45 / 298 行；
**字符串** 第 244 / 246 / 260 / 307 行。

**两条反例教训（都踩过）**：
1. 只掩字符串 ⇒ **注释**里的 `assert_eq!(f(x), f(x))` 被当成真断言（**假阳**）；
2. 抹掉字符串**内容**（而不是"跳过字符串里的宏名"）⇒
   `assert_ne!(from_symbol("Cm7"), from_symbol("CM7"))` 两侧被抹成同文（**假红**，裁决 R99）。

**掩码原则**：只记字节**区间**、绝不改文本、保留换行以维持行号。

## 4. 硬断言的超越函数台账（裁决 R77）

本 crate 生产代码只有 **3 处**超越调用；`sqrt` 未使用；乘除/比较/取整属 IEEE 精确类。

| 站点 | 函数 | 超越调用 | 依赖它的硬断言 | 判定 |
|---|---|---|---|---|
| `pitch.rs` `note_to_hz` | 频率输出 | `libm::pow` | `a4_is_440_hz…`（440）、`c4`（261.625565…）、`220`、`880` | **安全**：指数为 `0`/`±1` ⇒ 精确 |
| `pitch.rs` `hz_to_note` | 频率反查 | `libm::log2` ＋ `libm::round` | 往返/夹取判据 | 只做往返与夹取，**不钉**超越值 |
| `genre.rs` `swing_permille` | 登记百分数 → 千分比 | `libm::roundf` | `swing_permille_rounds…` | **安全**（取整与 `* 10.0` 属精确类） |

## 5. 公开面普查台账（第四～七批）

* **公开 `fn` 零判据**：普查前 **22**（其中 **12** 条连全工作区调用点都没有）⇒ 补判据后 **2**（`PitchClass::spell_with_letters`、`swing::validate_swing_permille`，两者都有生产调用点、由间接路径覆盖），**零调用点 0 条**。
* **手写 trait impl**：全 crate **13** 条（`Display`×9 / `FromStr`×2 / `PartialEq` / `Eq`）；**无**手写 `Debug`/`Hash`/`Ord`/`Default`。第八批起 `Display` 由一张 9 行黄金表 ＋ 长度断言覆盖，`Eq` 由 trait 约束在**编译期**钉住。
* **公开常量**：16 个取值逐条钉住（数组比字面量数组），另钉派生关系。
* **登记表**：四层防线 —— 求和（第四批）→ 多重集（第五批）→ 5 字段逐条（第六批）→ **11 字段逐条全表**（第七批，含 `source` 字面量、两个列表的全部元素、`swing` 的 f32 位型）。第四层补上后，第三层漏掉的 11/13 条注入全部收口。

## 6. 驱动器纪律（每次形态 D 批次）

1. **单实例锁**（`flock`）；2. **R40 开工基线当场重做**（逐文件 sha256，⛔ 不用 `git show HEAD:`）；
3. **还原三件套**（`git restore` ＋ `git clean` ＋ 断言 `git status` 为空 ＋ 基线比对）；
4. **R103：开工时 `git status --porcelain` 非空即 `REFUSED`**（防"未提交的判据被还原抹掉"）；
5. **240s 硬超时**（注入可能写出死循环 —— 裁决 R105）；6. `--no-fail-fast` 放在 `--` **之前**，
且 `--lib` 与 `--tests` **都给**（只给 `--lib` 时集成目标里的判据永远"不红"）；
7. **`NO_TEST_RAN` 判定**（过滤没匹配到任何测试时⛔不得记为 `NOT_RED`）；
8. **生产区唯一匹配重锚**（注入按行锚定生产代码，黄金表里的重复文本不会被误伤 —— R44①）。

## 7. 未解项 / 需裁决

* `Tonality::infer_from_root_text` 对根音**字母 `B`** 的判定已按裁决 **R42** 修好（见「R42」提交），
  本文件第 1 节第 15/16 行是它邻域的常驻判据。
* 文本扫描器的**可绕过性**（第 1 节末）未解，仅登记为工具局限。
* 登记表逐条表的**互换**类改动由第四层覆盖；**纯粹互换两条**（值相同）是空操作。
