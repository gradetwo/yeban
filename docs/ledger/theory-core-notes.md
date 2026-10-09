# theory-core 工作线笔记：规则库来源、口径与待决问题

> 工作线：`line/theory-core`　工作树：`.worktrees/theory-core`
> 拥有目录：`crates/yeban-theory/**`　本文档：本工作线唯一新增的非代码文件
> 规范来源：`docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md` §7/§8、
> `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` ROAD-M4-003、
> `docs/YEBAN_INDUSTRY_BENCHMARK_AND_OPEN_SOURCE_STUDY.md` §7（许可矩阵）、
> `AGENTS.md` §2 红线 3/4/8 与 §3 DoD、`docs/DEV_WORKFLOW.md`。

---

## 0. 一句话结论

`crates/yeban-theory` 从 scaffold 变成了**可编译、可测试、纯函数**的乐理内核：
7 个模块、**94 条测试**（71 单元 + 22 属性/集成 + 1 文档测试）全绿，
`clippy -p yeban-theory --all-targets -- -D warnings` 零告警，`cargo fmt --check` 通过。

**流派规则库当前 182 条**（规范正文写的是 "159 种"）。全部条目都是本 crate
依据公有领域乐理与地区性通行实践**自行编码**的规则数据，来源标记逐条可查（见 §3）。

---

## 1. 许可口径（最重要的一节）

`docs/YEBAN_INDUSTRY_BENCHMARK_AND_OPEN_SOURCE_STUDY.md` §7 的许可矩阵里，
"159 种流派规则与和弦走向" 一行登记为：

> **CC0 / Public Domain (公有领域)** — 零风险，传统音乐理论与数学比例属于公有领域，
> 规则编码归属于夜半原创代码。

本工作线严格按这条口径执行，并把它落成三条可检查的纪律：

1. **不搬运任何第三方数据集**。仓库里没有、也不会有来自第三方流派数据集的
   条目、BPM 表或走向表。`GENRES` 表的每一行都是本 crate 作者按公有领域乐理
   （教会调式、功能和声、12 小节布鲁斯这类公共形式、各地区通行节拍）自行撰写的
   规则编码。
2. **不复制受版权保护的文本**。`GenreRule` 的字段只有"音乐事实"级别的内容：
   BPM 区间、拍号、罗马数字级数、音阶名、摇摆比例、音符密度区间。
   `name_zh` / `name_en` 只用通用称法，**没有任何一句来自百科全书、教材或
   第三方数据集的描述性文字**。
3. **逐条登记来源**。每一行都带 `source` 字段，取值只允许下面三个常量之一
   （`GenreLibrary::by_source` 与 `source_histogram` 可直接审计）：

| 来源常量 | 含义 | 条数 |
| :--- | :--- | ---: |
| `traditional-theory/public-domain` | 传统乐理 / 教会调式 / 地区性传统形式（公有领域） | 52 |
| `common-practice/public-domain` | 某地区长期通行的公共曲式与节拍型（无个人著作权） | 31 |
| `20c-commercial-practice-facts` | 20 世纪通行商业实践；**只登记事实**（速度、拍号、级数），不登记他人文本 | 99 |
| **合计** | | **182** |

> `GenreLibrary::source_histogram()` 返回的就是上表；测试
> `source_histogram_covers_every_rule` 断言它覆盖全部条目且与 `by_source` 一致。

**明确排除**（本工作线从未引入，也建议后续工作线不要引入）：

- 任何 **CC-BY-NC**、**无许可**、**专有**的流派/乐理数据集（例如需要购买或
  许可不明的商业和弦走向库、示例工程、MIDI 素材包）；
- 各类"AI 编曲训练集"的标签表（`RSK-12` 点名的风险源）；
- 任何从受版权保护的书籍/网站**摘抄**的描述性文字或和弦例子。

---

## 2. 实现口径（写进代码的约定）

这些约定同时写在 `src/lib.rs` 与各模块的 crate 级文档里，测试会强制执行。

### 2.1 确定性 [ARCH-DET-001]

- **零随机**。本 crate 内部不存在随机数生成器。需要"在若干选项里挑一个"时，
  统一用 `yeban_theory::derive_index(rng_seed, salt, count)` 或
  `derive_range_i64(...)`，底层是 SplitMix64（纯 64 位整数运算）。
  同种子同输出、跨进程跨平台一致，且没有全局可变状态。
- **浮点只有一个出口**：`pitch::note_to_hz`。它走 `libm::pow` 而不是
  `f64::powf`，对齐架构文档对 L1 确定性"统一启用纯 Rust `libm` 数学库"的要求。
  **所有判定逻辑（音级、音程、级数、五度圈、声部跳进）都只用整数**。
- **无隐藏时间源**：没有 `SystemTime::now` / `Instant::now` / `std::fs` /
  `std::env::var`。测试 `crate_has_no_hidden_nondeterminism_sources`
  逐行扫描全部 8 个源文件的非注释行来做机械检查。
- **无 GUI / 音频依赖** [ARCH-TOP-003]：依赖只有 `thiserror` + `libm`
  （dev: `proptest`）。没有 slint / cpal / symphonia / rayon。

### 2.2 确定性集合 [MODEL-AST-003 / 红线 4]

生产代码里没有 `HashMap` / `HashSet`。流派库的 ID 索引与搜索文本缓存都用
`BTreeMap` + `Vec`，`GenreLibrary::ids()` 因此**恒为字典序升序**，
`search()` 的结果顺序也随之确定。

### 2.3 tick 口径 [MODEL-AST-001 的精神]

- `yeban_theory::PPQ = 960`，与 `yeban-model` 的常量**同值**。
  本 crate **刻意不依赖 `yeban-model`**：那会形成多余的单向 crate 依赖
  （theory → model），而理论层只需要这一个常量。一致性由
  `tests/properties.rs::ppq_matches_the_project_wide_960_tick_grid` 以
  "4/4 一小节 = 4 拍 = 3840 tick" 的口径独立验证。
  → 若集成者认为应当合并为一个共享常量，请开 ADR（见 §6 `needs`）。
- **最小时间分辨率 = `MIN_DURATION_TICKS = PPQ / 4 = 240`（一个 16 分音符）**。
  任何展开出的和弦时值都是 240 的整数倍，因此所有 `start_tick` 都落在
  16 分音符网格上。

### 2.4 走向展开规则（确定性，分支顺序即优先级）

`Progression::expand` 按下列顺序选规则，全部在整数网格上：

1. `degrees.len() <= bars` → **每小节一个和弦，级数循环取用**。
   例：`ii-V-I` 铺 4 小节 → `ii` `V` `I` `ii`，各 1 小节。
   例：`I-V-vi-IV` 铺 4 小节 → 4 个和弦各 1 小节。
2. 否则若"每级至少一拍"放得下 → **按四分音符拍（960 tick）均分**，
   余数从**前面的**级数开始各多一拍。
   例：1 小节（4 拍）+ 3 个级数 → 2 + 1 + 1 拍。
   （这一条取代了早期"1280 tick/级"的实现：1280 不在任何常用网格上。）
3. 否则若"每级至少一个 16 分音符"放得下 → **按 240 tick 均分**。
   例：1 小节 + 8 个级数 → 每个 480 tick。
4. 否则 → **报错** `TheoryError::ProgressionTooDense { degrees, slots }`。
   这里刻意选择显式失败，而不是静默丢弃级数或产出零长度区段：
   MCP 层可据此回 `INVALID_PARAMS`，让调用方自己把 `bars` 调大。

任何情况下都保证：`start_tick` 严格升序、区段首尾相接无空隙无重叠、
`duration_ticks` 恒为正且是 240 的整数倍、总和恒等于 `bars × ticks_per_bar`。

### 2.5 声部连接（voice leading）

- **硬约束（不变量）**：相邻和弦之间，**没有任何单声部移动超过 12 个半音**。
  违反即 `TheoryError::NoFeasibleVoicing`，绝不静默产出跳进过大的声位。
  代码里这个界是 `VoicingConstraints::max_voice_jump`（默认 12）。
- **软目标**：总移动量最小。用固定宽度 `SEARCH_BEAM = 24` 的**束搜索**，
  代价 = `累计移动量 + 每个声位的跨度`；代价相同的候选按声位字典序打破平局，
  因此不依赖任何哈希迭代顺序。
- 每个声部只能落在和弦构成音上（含跨八度的重复），且声部严格由低到高。
- 默认 3 声部音域：`C3–C5 / C4–C6 / C5–C7`；默认 4 声部：`C2–C4 / C3–C5 /
  C5–C7 / C6–C8`（4 声部时三和弦必然重复一个构成音，重复被放在上方八度）。

### 2.6 音名拼写

- 判定永远回到 `PitchClass` 的整数比较；`NoteName`（音级字母 + 变音记号，
  变音记号限定在重升/重降 `-2..=+2`）只用于显示。
- 拼写代价 = `|变音记号|`，平局时按调性偏好打破：`F`/`Bb`/`Eb`/`Ab`/`Db`
  与小调类走降号侧，其余走升号侧。
  因此 F 大调的第四级输出 `Bb` 而不是 `A#`；`D` 大调的第七级是 `C#`。
- 和弦自带 `Tonality`（`SharpMajor` / `FlatMajor` / `Minor`），
  由 `Chord::from_symbol` 从根音写法推断（`Bb…` → 降号侧，`F#…` → 升号侧）。
  `symbol()` → `from_symbol()` 对全部 12 根音 × 21 和弦种类恒等（属性测试覆盖）。

### 2.7 和弦符号语法

```text
<根音><后缀>[/<低音>]
```

- 后缀**大小写敏感**：`m7` 是小七、`M7` 是大七（爵士记谱的既有约定）。
- 额外接受爵士简写与符号写法：`C-7`、`CΔ7`、`Cø7`、`C°7`、`C+`、
  `CmMaj7`、`Gsus`、`G7sus4`（新增 `ChordKind::Dominant7Sus4`）。
- `C6/9` 里的 `/` 属于后缀本身，**必须先在"后缀开头"剥掉 `6/9`** 再处理斜杠低音，
  否则 `C6/9/E` 会被切成 `head="C6"` + `bass="9/E"`。
  （这是属性测试 `inverted_chords_round_trip_through_their_slash_notation`
  抓出来的真实缺陷，已修。）
- 不支持的写法（如 `Cmaj9#11`）返回 `TheoryError::ChordQualityUnknown`，
  绝不猜。

---

## 3. 流派规则库：来源与许可逐条登记

- **当前条数：182**（测试 `library_size_is_pinned_to_the_measured_number`
  把这个数字钉住；改动 `GENRES` 必须同步本文件与下面两张名单）。
- 与规范正文的 "159 种" 的差异，见 §4。

### 3.1 `traditional-theory/public-domain`（52 条）

依据公有领域的传统乐理与地区性传统形式自行编码（调式、功能和声、
五声/教会调式、无个人著作权的传统曲式）：

```text
gregorian_chant, renaissance_polyphony, baroque_chorale, baroque_fugue, baroque_suite,
classical_sonata, classical_minuet, waltz, march, romantic_lied, nocturne, etude,
impressionism, impressionist_piano, hymn, anthem, carol, lullaby, opera_aria, operetta,
ballet, passacaglia, chaconne, toccata, prelude, sonatina, blues, delta_blues, spiritual,
work_song, field_holler, folk, americana, bluegrass, celtic, irish_trad, jig, reel,
scottish_trad, klezmer, balkan, polka, chanson, fado, arabic_maqam, turkish_makam,
persian_dastgah, hindustani, carnatic, gamelan, pentatonic_east_asian, andean
```

许可依据：乐理规则（音阶构造、级数功能）与传统曲式（各地区的舞曲拍号与
常用调式）属于公有领域知识；本表的**具体编码**（字段取值与组合）是夜半原创。

### 3.2 `common-practice/public-domain`（31 条）

依据某地区长期通行、无个人著作权的公共曲式/节拍型：

```text
gospel, samba, bossa_nova_brazil, choro, tango, milonga, bolero, son_cubano, salsa,
merengue, bachata, cumbia, mariachi, ranchera, norteno, tejano, flamenco, sevillanas,
rumba_flamenca, ska, rocksteady, reggae, dub, dancehall, afrobeat, highlife, soukous,
afro_cuban, raita, bollywood, bhangra
```

### 3.3 `20c-commercial-practice-facts`（99 条）

20 世纪以降的通行商业流派。**只登记音乐事实**（BPM 区间、拍号、级数走向、
音阶、摇摆比例、密度提示），不登记任何他人的描述性文本：

```text
orchestral_film_score, epic_trailer, minimalism, jazz_swing, big_band, bebop, hard_bop,
cool_jazz, modal_jazz, free_jazz, bossa_nova, latin_jazz, smooth_jazz, jazz_waltz,
gypsy_jazz, ragtime, stride_piano, boogie_woogie, chicago_blues, jump_blues, blues_rock,
soul, motown, funk, p_funk, disco, boogie, contemporary_rnb, neo_soul, quiet_storm,
doo_wop, rock_and_roll, rockabilly, surf_rock, garage_rock, psychedelic_rock,
progressive_rock, hard_rock, heavy_metal, thrash_metal, death_metal, black_metal,
doom_metal, power_metal, progressive_metal, metalcore, nu_metal, punk_rock, pop_punk,
post_punk, new_wave, shoegaze, grunge, alternative_rock, indie_rock, math_rock, post_rock,
emo, house, deep_house, tech_house, progressive_house, garage_house, techno,
minimal_techno, trance, psytrance, hardstyle, dubstep, drum_and_bass, jungle, breakbeat,
big_beat, trip_hop, downtempo, ambient, drone, new_age, synthwave, vaporwave,
lo_fi_hip_hop, boom_bap, trap, drill, grime, chiptune, video_game_score, pop, dance_pop,
synth_pop, ballad, power_ballad, country, honky_tonk, outlaw_country, cabaret, music_hall,
reggaeton, amapiano
```

**许可注意**：流派**名称**本身是通用词，不受版权保护；"BPM 范围/拍号/常用级数"
是可自由陈述的事实。上表**没有**任何来自第三方数据库的行、没有抄任何一句百科
或教材的句子。若未来有人想把这些条目扩充成"带描述的乐理百科"，**必须先解决许可
问题**（见 §5 的"缺口清单"）。

### 3.4 规则库的结构判据

下列测试保证"数据不是坏的"（对全部 182 条生效）：

| 判据 | 说明 |
| :--- | :--- |
| `library_size_is_pinned_to_the_measured_number` | 条数被钉在 182 |
| `ids_are_unique_and_stable` | ID 唯一且 `ids()` 为字典序升序 |
| `id_is_lowercase_snake_case_ascii` | ID 只含小写字母/数字/下划线 |
| `every_rule_has_a_plausible_bpm_range_meter_and_names` | BPM 有序、拍号分母为 2 的幂、名称非空、密度提示有序、摇摆比例在 50..=80 |
| `idiomatic_data_is_structurally_valid` | 每条走向都能被 `Progression::parse` 吃下，每个音阶名都能被 `ScaleKind::parse` 吃下 |
| `every_rule_can_produce_a_chord_sketch` | 每条规则都能产出 4 小节骨架且区段首尾相接 |
| `every_genre_produces_a_playable_sketch`（集成测试） | 每条规则都能一路走通到 **3 声部连接**，且每个和弦的根音都属于该调音阶 |
| `source_histogram_covers_every_rule` | 来源直方图覆盖全部条目 |

---

## 4. 与规范正文的偏差（显式记录，不静默降级）

| 偏差 | 规范说法 | 实际 | 处置 |
| :--- | :--- | :--- | :--- |
| 流派条数 | "159 种" | **182 条** | 超出而非不足。本 crate **未**把数量写死在任何对外文案里；`crates/yeban-theory/Cargo.toml` 的 `description` 从 "159 种流派规则" 改为"流派规则库"，避免写出一个会过期的数字。**根 `Cargo.toml`、`README.md`、`docs/YEBAN_*.md` 一律未改**（不属本工作线），提请集成者决定是否把规范/文案统一为"流派规则库（当前 N 条）"。 |
| `ChordKind` 数量 | 任务书列举 19 种 | **21 种**（新增 `Dominant7Sus4`、并显式登记 `Minor9`） | 任务书要求 `from_symbol` 能吃 `G7sus4`，而 `Sus4` 的公式里没有七度音；若把它解析成 `Sus4`，`symbol()` 会丢掉 `7`，破坏"往返恒等"这条判据。故新增一种，写在模块文档里。 |
| PPQ 常量来源 | 未明确由谁拥有 | `yeban-theory` 独立声明 `PPQ = 960` | 不新增 `theory → model` 依赖。见 §6 `needs`。 |

---

## 5. 缺口清单（诚实说明：为什么不是"159 条一一对应"）

规范只给了数字 "159"，**没有给出清单**。本工作线因此按"公开、可自行编码、
许可清晰"三条标准自建 182 条，覆盖七大类。以下类别**被有意排除或延后**，
原因是许可或口径不明，宁可少而合法：

1. **需要采样/音频资产才能定义的子流派**（如 specific 厂牌音色、民间
   乐器特有演奏法）：只能靠未登记许可的采样包表达 → 排除。
2. **有明确个人著作权的流行子风格**（特定艺人的个人风格标签）：不属于
   公有领域乐理 → 不编入规则库，留给用户自定义 preset。
3. **需要微分音（非十二平均律）的传统体系**：阿拉伯木卡姆/土耳其马卡姆/
   波斯达斯特加赫/印度拉格目前**只登记了十二平均律的近似骨架**
   （用 `phrygian` / `harmonic_minor` 近似），**没有**实现 24-TET。
   本 crate 的 `PitchClass` 是 0..12，表达不了。→ 见 §6 `pending`。
4. **地区性传统的完整名录**（如中国各省戏曲、印尼各岛甘美兰、非洲各语族
   节奏体系）：本轮只放了 `pentatonic_east_asian` 与 `gamelan` 两个代表条目。
   要做到"一地区一条"需要逐条核对来源，本轮不作为。
5. **摇摆比例的精确性**：`swing` 用 `f32` 百分数表示（50 = 平直，
   66.7 ≈ 三连音摇摆），来源是通行实践的**近似值**，不是某个权威表格的复制。
6. **拍号只用 2/4、3/4、4/4、5/4、6/8、7/8**：更复杂的拍号（如 9/8、11/8、
   复合混合拍）尚未登记。

---

## 6. `pending` / `needs` / `TODO(hoist)`

### TODO(hoist)

**无。** 本 crate 只用了根 `Cargo.toml` 的 `[workspace.dependencies]` 里
**已经登记**的版本（`thiserror`、`libm`、`proptest`），没有写任何显式版本，
因此**不需要**集成者动根清单，也没有版本漂移。

### needs（需要集成者/人类裁决）

1. **`PPQ` 常量归属**：现在 `yeban-model::PPQ` 与 `yeban-theory::PPQ` 各有一份
   同值常量（都是 960）。两条路：(a) 保持现状，靠一条集成测试对账；
   (b) 让 `yeban-theory` 依赖 `yeban-model` 的常量。本工作线选 (a)，因为
   (b) 会引入 `theory → model` 依赖，而理论层只用到这一个整数。
   若集成者要改，请开 ADR（`docs/adr/`）。
2. **规范/文案里的 "159 种"**：见 §4。本工作线无权修改规范与根级文案。
3. **`docs/DEVELOPMENT_LEDGER.md`**：本 crate 的测量数字（182 条规则、
   94 条测试）按纪律应记入根账本，但该文件由集成者独占，故只登记在本文件。

### pending（本工作线明确**没有**做的事）

1. **没有实现非十二平均律**（24-TET / 微分音）。木卡姆类条目只是近似骨架。
2. **没有 `serde` 序列化**：`GenreRule` / `Chord` / `Scale` 目前不能直接
   JSON 序列化。MCP 层若需要，应在本 crate 加 `serde`（已在根清单登记）
   或由 `yeban-mcp` 侧做 DTO 映射。本轮不加，避免为尚不存在的调用方设计 API。
3. **没有实现节奏型/鼓组 pattern**：`GenreRule` 只有密度提示与摇摆比例，
   没有具体的鼓点网格。`yeban_propose_section` 目前只能拿到和弦/声部骨架。
4. **没有实现旋律生成**：本 crate 只产出和声与声部连接，没有 `melody()`。
5. **没有把 `yeban_propose_section` 接起来**：MCP 工具属 `yeban-mcp`，
   不在本工作线。
6. **`swing` 只登记不应用**：没有"按 swing 比例把八分音符对量化"的函数。
7. **没有 benchmark**：本机禁跑基准；`BASELINE-*` 的指标未测。
8. **未做跨架构确定性对账**（L2 级，属 CI）。

> ⚠️ **2026-10-08 更正注（`pending 6` 已关闭；原句保留不改写）**
> 提交 `e191abd`（`feat(yeban-theory): apply GenreRule::swing via an integer swing grid`）落地了**整数摇摆网格** ⇒ 上面第 6 条（"`swing` 只登记不应用"）**已关闭**。
> **新增公开面**（本次逐条复核：`grep -n 'pub fn\|pub struct\|pub const' crates/yeban-theory/src/swing.rs`，签名照抄）：
> - `pub const SWING_PERMILLE_STRAIGHT: u16 = 500;`（`crates/yeban-theory/src/swing.rs:56`）、`pub const SWING_PERMILLE_MAX: u16 = 1000;`（`:59`）；
> - `pub struct SwingPair { pub first: u64, pub second: u64 }`（`:65`）＋ `pub const fn total(self) -> u64`（`:75`）＋ `pub const fn offbeat_offset(self) -> u64`（`:81`）；
> - `pub const fn validate_swing_permille(permille: u16) -> Result<(), TheoryError>`（`:93`）；
> - `pub fn swung_onset_offset(pair_ticks: u64, permille: u16) -> Result<i64, TheoryError>`（`:108`）；
> - `pub fn swung_pair_span(pair_ticks: u64, permille: u16) -> Result<SwingPair, TheoryError>`（`:124`）；
> - `pub fn quantize_onset(onset_ticks: u64, pair_ticks: u64, permille: u16) -> Result<u64, TheoryError>`（`:153`）；
> - `GenreRule::swing_permille(&self) -> Result<Option<u16>, TheoryError>`（`crates/yeban-theory/src/genre.rs:178`）；错误变体 `TheoryError::SwingOutOfRange`（`crates/yeban-theory/src/error.rs:114`）；
> - 再导出：`crates/yeban-theory/src/lib.rs:102` 的 `pub use swing::{SWING_PERMILLE_MAX, SWING_PERMILLE_STRAIGHT, SwingPair, quantize_onset, swung_onset_offset, swung_pair_span};`。
> **口径**（出自 `crates/yeban-theory/src/swing.rs` 的模块文档）：输入是 `u16` **千分比**（`500` = 平直、`1000` = 附点八分）；`GenreRule::swing` 的 `f32` 百分数只在 `GenreRule::swing_permille` **一处**转换，其后**全部是整数运算** ⇒ 与 [ARCH-DET-001] 同口径、结果逐位一致。⛔ 越界比例**不被静默钳制**，返回 `SwingOutOfRange`。
> ⚠️ 本条只关闭"只登记不应用"；`pending` 1–5 与 7–8 **仍然开放**。

> ⚠️ **2026-10-08 更正注（`pending 3` 的"具体鼓点"那一半已补上；原句保留不改写）**
> `pending 3` 的原句是"没有实现节奏型/鼓组 pattern …… 没有具体的鼓点网格"。
> `crate::rhythm`（`0a1c0d2`）已补上**网格**那一半；`crates/yeban-theory/src/drum.rs`
> 本次补上**分派**那一半：把**已经存在的** `MetricGrid` 读成一份可逐件读回的鼓组型。
> **新增公开面**（本次逐条复核：`grep -n 'pub fn\|pub struct\|pub enum\|pub const\|pub type' crates/yeban-theory/src/drum.rs`，签名照抄；行号为本次提交时的读数）：
> - `pub const DRUM_VOICE_COUNT: usize = 4;`（`:101`）；
> - `pub enum DrumVoice { Kick, Snare, HiHat, Ride }`（`:109`）＋ `pub const ALL: [Self; DRUM_VOICE_COUNT]`、`pub const fn ordinal(self) -> usize`（`:129`）、`pub const fn name(self) -> &'static str`（`:140`）、`pub const fn gm_key(self) -> u8`（`:155`）；
> - `pub struct DrumHit { pub voice, pub tick: u64, pub bar: u32, pub cell: u32, pub beat: u8, pub weight: u8, pub accent: bool }`（`:167`）；
> - `pub type Backbeat = u8;`（`:188`）、`pub const fn default_backbeat(meter: Meter) -> Backbeat`（`:196`）、`pub const fn is_meter_group_start(meter: Meter, cell: u32, cells_per_beat: u64) -> bool`（`:224`）；
> - `pub struct DrumPattern`（`:246`）＋ `pub fn hits(&self) -> &[DrumHit]`、`pub fn len(&self)`、`pub fn is_empty(&self)`、`pub fn hit_count(&self, voice: DrumVoice) -> usize`（`:272`）、`pub const fn grid(&self) -> &MetricGrid`、`pub const fn meter/bars/onsets_per_bar/ticks_per_bar/total_ticks`、`pub fn hits_in_bar(&self, bar: u32) -> &[DrumHit]`（`:319`）；
> - `pub fn drum_pattern(meter, bars, onsets_per_bar, grouping: Option<BeatGrouping<'_>>, backbeat: Backbeat) -> Result<Option<DrumPattern>, TheoryError>`（`:349`）、`pub fn swung_drum_pattern(..., permille: Option<u16>, ...)`（`:369`）；
> - `GenreRule::drum_pattern(&self, bars: u32, onsets_per_bar: u32) -> Result<Option<DrumPattern>, TheoryError>`（`crates/yeban-theory/src/genre.rs:469`，读登记拍号与摇摆比例）；
> - 再导出：`crates/yeban-theory/src/lib.rs:155` 的 `pub use drum::{Backbeat, DRUM_VOICE_COUNT, DrumHit, DrumPattern, DrumVoice, default_backbeat, drum_pattern, is_meter_group_start, swung_drum_pattern};`。
> **口径**（出自 `crates/yeban-theory/src/drum.rs` 的模块文档）：击点集合 **=** 网格的 onset 集合（不新增、不移动 tick）；底鼓 = 组起点、军鼓 = 反拍拍的**起点**、踩镲 = 每一格、吊镲 = 强位上的组起点；全部整数运算 ⇒ 与 [ARCH-DET-001] 同口径。⛔ 越界的 `backbeat`（0 或 > 拍数）**不被静默钳制**，返回 `Ok(None)`；本 crate **不新增** `TheoryError` 变体。
> **`pending 3` 仍未关闭的那一半**：**逐流派的鼓点型数据**（哪条流派打什么样的鼓点序列）没有登记进 `GENRES` —— 本次只把拍号、摇摆比例与**调用方传入**的分组/反拍位置变成鼓点，`GenreRule::drum_pattern` 的缺省分组与反拍位置都只读拍号。`pending` 1、2、4–8 的状态不变（其中 4 已由 `melody` 关闭）。

---

## 7. 判据纪律：被故意破坏过的判据（SKILL 规则 2）

按要求，至少两条核心判据**先被改坏、观察到变红、再改回**。操作留痕如下
（破坏只在本工作树内做过，未提交）：

### 破坏 A：和弦后缀表（`from_suffix` 候选集里删掉 `Major7`）

- 改动：`crates/yeban-theory/src/chord.rs` 的 `ChordKind::from_suffix`，
  从 `all` 数组里删掉 `Self::Major7`。
- 变红（3 条）：

  ```text
  chord::tests::required_symbols_parse_to_the_expected_kinds ... FAILED   (chord.rs:639)
  chord::tests::jazz_shorthand_and_unicode_suffixes_parse ... FAILED      (chord.rs:666)
  chord::tests::symbol_round_trips_for_every_kind_and_all_twelve_roots ... FAILED (chord.rs:742)
  test result: FAILED. 68 passed; 3 failed
  ```

- 结论：这条判据真的在检查"`Cmaj7` 能不能解析"，不是穿着测试外衣的注释。
- 恢复后：`test result: ok. 71 passed`。

### 破坏 B：声部连接的跳进硬约束（把剪枝条件取反）

- 改动：`crates/yeban-theory/src/voice_leading.rs` 的 `realize`，
  把"跳进超界即剪枝"改成"跳进**不**超界即剪枝"。
- 变红（单元 3 条 + 集成 3 条）：

  ```text
  voice_leading::tests::the_documented_jump_bound_holds_for_the_canonical_progressions ... FAILED
  voice_leading::tests::four_voices_also_work ... FAILED
  voice_leading::tests::impossible_constraints_fail_loudly_instead_of_silently ... FAILED
  test result: FAILED. 68 passed; 3 failed

  # 集成属性测试（--test properties）
  voice_leading_never_exceeds_the_documented_jump_bound ... FAILED
  four_voice_configuration_also_respects_the_bound ... FAILED
  every_genre_produces_a_playable_sketch ... FAILED
  Test failed: assertion failed: result.max_voice_jump() <= 12 (properties.rs:389, 413)
  test result: FAILED. 19 passed; 3 failed
  ```

- 结论：**跳进上界**（本任务书要求的核心不变量）确实由这些判据守住。
- 恢复后：`test result: ok. 71 passed` / `22 passed` / `1 passed`。

### 属性测试额外抓到的真实缺陷（已修）

`tests/properties.proptest-regressions` 里保留了 4 个收缩后的反例种子，
其中两个是**当前实现真有的 bug**，不是测试写错：

1. `root_value = 0, kind_index = 19, inversion = 1`（`C6/9/E`）：
   斜杠解析把 `6/9` 误当成 `6` + 斜杠低音 `9/E` → `NoteNameUnknown`。
   修法：先在**后缀开头**剥 `6/9`，再处理斜杠低音。
2. `bars = 1, degrees = [I, I, I]`：早期实现给出 1280 tick/级，
   **不在任何常用网格上**。修法：见 §2.4 的三级规则（小节 → 拍 → 16 分音符，
   再不够就报 `ProgressionTooDense`）。

另外两个种子是**测试自身写错**后被修正的（`degree_to_pitch` 的越界安全包线、
音名拼写的八度推进），保留在文件里作为回归记录。

---

## 8. 本次改动清单（绝对路径）

代码（全部在 `crates/yeban-theory/` 下）：

| 文件 | 内容 |
| :--- | :--- |
| `src/lib.rs` | 模块挂载、`pub use`、crate 级文档（边界声明）、`splitmix64` / `derive_index` / `derive_range_i64` |
| `src/error.rs` | `TheoryError`（含 `ProgressionTooDense`、`is_input_error` 分类） |
| `src/pitch.rs` | `PitchClass` / `NoteName` / `SpelledPitch` / `Pitch`（含全部八度常量）/ `Interval` / `note_to_hz` / `hz_to_note` / `parse_pitch_class` |
| `src/scale.rs` | `ScaleKind`(16) / `Scale`：级数映射、`contains`、自然三和弦、`triad_quality`、五度圈邻接与近关系调、调性化拼写 |
| `src/chord.rs` | `ChordKind`(21) / `Tonality` / `Chord`：`from_symbol` / `symbol` / 转位 / 声位 / `from_voicing` |
| `src/progression.rs` | `Meter` / `RomanQuality` / `Degree` / `Progression` / `ChordSpan` / `expand_progression` / `PPQ` / `MIN_DURATION_TICKS` |
| `src/voice_leading.rs` | `VoiceRange` / `VoicingConstraints` / `VoiceLeadingResult` / `realize` / `realize_three_voices`（束搜索） |
| `src/genre.rs` | `GenreRule` / `GenreLibrary` / `GENRES`（182 条）/ 来源常量 |
| `tests/properties.rs` | 22 条属性/集成判据 |
| `tests/properties.proptest-regressions` | 4 个收缩反例种子 |
| `Cargo.toml` | 轻依赖（`thiserror`、`libm`；dev `proptest`），全部走 `*.workspace = true` |

工作线唯一新增的文档：本文件。

**未修改**：根 `Cargo.toml`、`AGENTS.md`、`README.md`、`.github/**`、`scripts/**`、
`docs/DEVELOPMENT_LEDGER.md`、`docs/YEBAN_*.md`、其它 `crates/**`、`spikes/**`、
`schemas/**`、任何法务文件。`Cargo.lock` 因新增三个依赖而被更新（允许并一起提交）。

---

## 9. 复查入口（给下一位接手的人）

```bash
cd .worktrees/theory-core
bash scripts/dev/cargo-local.sh test -p yeban-theory        # 94 条判据
bash scripts/gates/run-gates.sh crate yeban-theory          # fmt + 红线守卫 + clippy + test
bash scripts/dev/ci-verdict.sh line/theory-core             # 只有 CI 的判决算绿
```

先读 `src/lib.rs` 的 crate 级文档（边界与确定性契约），再读本文件 §1（许可）
与 §4（偏差）。
