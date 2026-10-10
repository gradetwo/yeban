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
