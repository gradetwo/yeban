# `line/theory-wiring` 台账：把 `yeban_propose_section` 接上 `yeban-theory`（关闭 needs-6）

> 工作树 `yeban/.worktrees/theory-wiring`，分支 `line/theory-wiring`，起点 `main e3b6bfc`。
> 裁决依据：`ADR-0001` **D49**（逐字见下）+ `docs/ledger/feature-alignment.md` 错位 6。
>
> > `yeban-theory`（**7 040 行**）此前**零依赖边**；`yeban-mcp` 的 `section.rs:45` 自造了 4 行 `STYLE_PRESETS`。
> > ⇒ **接线**：由 MCP 侧（或引擎侧）**按需**消费 `yeban-theory` 的既有能力，并**关闭**
> > `docs/ledger/tools-domain-notes.md` 的 needs-6。接线时**不许**把 theory 的逻辑复制一份到 mcp。

**一句话结果**：`yeban_propose_section` 的**风格合法集合**、**调式语义（音级集合）**、**声部数**现在都来自
`yeban-theory` 的公开 API；本地 `STYLE_PRESETS`（4 行）与 `MODES`（12 行）**已删除**；
`D dorian` 与 `D minor` 的输出**不同**且差异**恰好**是第六级；未知风格仍然是 `STYLE_NOT_FOUND`；
`yeban-theory` 的源码**一个字节都没改**；本线**没有**复制 theory 的任何逻辑。

---

## 1. 事实基线（动手前先读源码，不照抄任务描述）

| 一手事实 | 证据 |
| :--- | :--- |
| `yeban-theory` 此前**零消费者** | `crates/yeban-mcp/Cargo.toml` 的依赖表里没有它；根 `Cargo.toml` 的 `[workspace.dependencies]` 里**早已登记** `yeban-theory = { path = "crates/yeban-theory" }`（根清单第 85 行） |
| MCP 侧的风格/调式表是本地常量 | 改动前 `crates/yeban-mcp/src/domain/section_build.rs` 的 `STYLE_PRESETS`（4 行）+ `MODES`（12 行）；`scale` 只用到 `tonic_pc` ⇒ `D dorian` ≡ `D minor` |
| `GenreRule` 到底给了什么 | `crates/yeban-theory/src/genre.rs:49` 的字段：`id` / `name_zh` / `name_en` / `default_bpm_range` / `meter` / `typical_progressions` / `typical_scales` / `swing` / `note_density_hint` / `source`。**没有**任何乐器、声部或配器字段 |
| `GenreLibrary` 里**没有**"风格 → 声部"规则 | `grep -rn 'instrument\|Instrument\|voice_name\|part_name' crates/yeban-theory/src/` ⇒ **0 命中** |
| theory 的流派 ID 就是给 MCP `stylePreset` 用的 | `crates/yeban-theory/src/genre.rs:51` 的字段注释逐字写着"稳定 ID（小写蛇形，`GenreLibrary::get` 的键，**对应 MCP `stylePreset`**）" |
| theory 全库规模与形态（实测） | 182 条流派；每个流派首个典型和弦**都是三和弦**；全部 182 条里只有 `funk` 的走向含七和弦（`I7-IV7`）；`realize(_, FOUR_VOICES)` 对 **182/182** 都可行（因为 `realize` 把和弦音铺在 C2..C7 六个八度里，三和弦也能撑起 4 声部） ⇒ **"四声部可行性"不能用来定声部数** |
| theory 的调式别名行为（实测） | `ScaleKind::parse("ionian") == ScaleKind::Major`、`parse("aeolian") == ScaleKind::NaturalMinor` ⇒ `Ionian`/`Aeolian` 是**解析别名**，不是独立的公开调式 |

> 上面最后两条是**必须实测才知道**的：它们直接否掉了两个"看起来很自然"的接线方案
> （"声部数 = 四声部可行性"、"声部数 = 首个和弦构成音数"）。实测脚本见 §5.4。

---

## 2. 接了哪些 API（逐个 `文件:行` + 签名）

调用点全部在 `crates/yeban-mcp/src/domain/section_build.rs`（本线**未改** `crates/yeban-theory/**`）。

| # | theory API（文件:行） | 签名 | 本线怎么用（MCP 侧行号） |
| :--- | :--- | :--- | :--- |
| 1 | `crates/yeban-theory/src/genre.rs:2474` | `pub fn get(id: &str) -> Result<&'static GenreRule, TheoryError>` | `preset_parts` 判"风格是否存在"：`section_build.rs:441`；`GenreNotFound` → 契约码 `STYLE_NOT_FOUND`（**语义未放松**） |
| 2 | `crates/yeban-theory/src/genre.rs:2490` | `pub fn ids() -> Vec<&'static str>` | `available_presets()`（`section_build.rs:353`）= 错误响应 `data.availablePresets` 的**唯一来源** |
| 3 | `crates/yeban-theory/src/genre.rs:2458` | `pub const fn len() -> usize` | 错误响应 `data.presetCount`（182） |
| 4 | `crates/yeban-theory/src/genre.rs:2452` | `pub const fn all() -> &'static [GenreRule]` | 判据 ② 的全库遍历（MCP 生产路径不需要它；它让判据能独立复算） |
| 5 | `crates/yeban-theory/src/genre.rs:113` | `pub fn primary_scale(&self, tonic: PitchClass) -> Result<Scale, TheoryError>` | `genre_voice_count`（`section_build.rs:374`）用固定参考主音 `PitchClass::C`（`REFERENCE_TONIC`）取该流派的音阶，用来把它的典型走向展开成真实和弦 |
| 6 | `crates/yeban-theory/src/progression.rs:413` | `pub fn parse(text: &str) -> Result<Self, TheoryError>` | 同上：把 `GenreRule.typical_progressions` 的罗马数字文本解析成走向 |
| 7 | `crates/yeban-theory/src/progression.rs:557` | `pub fn chords(&self, key: &Scale) -> Vec<Chord>` | 同上：走向 → 真实和弦序列 |
| 8 | `crates/yeban-theory/src/chord.rs:457` | `pub fn pitch_classes(&self) -> Vec<PitchClass>` | 同上：和弦 → 构成音数（三和弦 3 / 七和弦 4）⇒ **声部数** |
| 9 | `crates/yeban-theory/src/scale.rs:209` | `pub fn parse(text: &str) -> Result<Self /*ScaleKind*/, TheoryError>` | `parse_scale`（`section_build.rs:483`）判调式：接受面（`minor` / `natural_minor` / `自然小调` / `aeolian` / …）与语义**都由 theory 决定** |
| 10 | `crates/yeban-theory/src/scale.rs:159` | `pub const fn name(self) -> &'static str` | 公开调式名（`mode_names()`，`section_build.rs:195`）与 `Scale::canonical()` 的调式文本 |
| 11 | `crates/yeban-theory/src/scale.rs:288` | `pub const fn new(tonic: PitchClass, kind: ScaleKind) -> Self` | `Scale::theory_scale()`（`section_build.rs:337`）：MCP 的 `Scale` → theory 的音阶视图 |
| 12 | `crates/yeban-theory/src/scale.rs:361` | `pub fn contains(&self, pc: PitchClass) -> bool` | `snap_into_scale`（`section_build.rs:532`）：音阶内容（有哪些音级）的**唯一判据** |
| 13 | `crates/yeban-theory/src/pitch.rs:251` | `pub const fn new(value: u8) -> Result<PitchClass, TheoryError>` | 音级构造（音名表 → `PitchClass`，以及收拢候选音级） |
| 14 | `crates/yeban-theory/src/pitch.rs:267` | `pub const fn semitones(self) -> u8` | 移调量 `delta = (tonic.semitones() + 12 - material.root_pc) % 12` |
| 15 | `crates/yeban-theory/src/pitch.rs:418` | `pub const C: Self` | `REFERENCE_TONIC`（只用于"和弦构成音数"——该量与主音无关，见 §5.4） |
| 16 | `crates/yeban-theory/src/error.rs:110` | `enum TheoryError { …, GenreNotFound, … }` | `preset_parts` 的映射依据（`GenreNotFound` ↔ `STYLE_NOT_FOUND`） |
| 17 | `crates/yeban-theory/src/scale.rs:96` | `pub const fn intervals(self) -> &'static [u8]` | **不直接调用**，但它是 #9/#12 的语义来源（音阶间隔表在 theory 里，MCP 不复制） |
| 18 | `crates/yeban-theory/src/genre.rs:126` / `:145` | `pub fn sketch(&self, tonic: PitchClass, bars: u32) -> Result<Vec<ChordSpan>, TheoryError>` / `pub fn chords(&self, tonic) -> Result<Vec<Chord>, TheoryError>` | **未调用**（生产路径与判据都不用；`sketch` 只取**第一条**典型走向，会丢掉 `funk` 的七和弦 ⇒ 本线改用"全部走向 + `Progression::chords`"，见 §3.2。它只出现在设计探针与一条注释里） |

**"没有复制逻辑"的机械证据**（本机真跑）：

```text
$ grep -nE 'MAJOR_INTERVALS|DORIAN_INTERVALS|\[0, *2, *4, *5, *7|RomanQuality|semitone_distance_to|candidate_voicings|SEARCH_BEAM' \
      crates/yeban-mcp/src/domain/section_build.rs
（0 命中 —— 文件里没有任何音阶间隔数组、和弦公式数组、罗马数字解析或声部连接搜索）
```

所有"音乐知识"都落在 theory 的调用上；`section_build.rs` 里只多了"什么时候问 theory、
把答案怎么读"的政策代码（收拢、命名、错误映射）。

---

## 3. 哪些**没**接，及原因

### 3.1 风格 → 声部**名字**：theory 里没有这个能力（本线登记为 needs，不假装）

`GenreRule` 没有乐器/声部字段，`grep instrument` 在 `crates/yeban-theory/src/` 是 **0 命中**。
因此：

- 声部**数**来自 theory（§2 #5–#8）：该流派全部典型走向里"和弦构成音数"的最大值；
- 声部**名**保留本层的**角色词表**（`section_build.rs:122,125`）：
  3 声部 `Bass / Alto / Soprano`，4 声部 `Bass / Tenor / Alto / Soprano`（低 → 高，与 theory 的声部序一致）。

"和弦有几个构成音就分几个声部"是**本层的配器政策（读法）**，但它的**数据一个字节都不来自本地表**：
`GenreRule::primary_scale` → `Progression::parse` → `Progression::chords` → `Chord::pitch_classes().len()`。
theory 自己的 `VoicingConstraints::FOUR_VOICES` 文档也写"4 声部留给需要加九音/七音的进行"，与本读法同口径。
没有对应命名规则时（构成音数 ∉ {3,4}）本层**如实报 `CONFLICT`**（`part_names`，`section_build.rs:412`），
不猜名字、不兜底。

### 3.2 被否掉的两个方案（实测数据说话）

| 方案 | 为什么否掉 |
| :--- | :--- |
| 声部数 = `realize(&spans, FOUR_VOICES).is_ok() ? 4 : 3` | 实测 182/182 都可行（`realize` 把和弦音铺在 6 个八度里 ⇒ 三和弦也能撑 4 声部）⇒ 恒定 4，零信息 |
| 声部数 = 首个典型和弦的构成音数（`GenreRule::sketch` / `chords` 只取**第一条**走向） | 实测 182/182 首个和弦都是三和弦 ⇒ 恒定 3，零信息；改用"**全部**典型走向的最大构成音数"才拿到 3/4 的区分（`funk` = 4） |

### 3.3 `voice_leading`（声部连接）**未接**（能力在，但接线会改本工具的产品语义）

`voice_leading::{realize, realize_three_voices}` 可以真的算出三声部音高，用它替换"材料移调"就等于让
`yeban_propose_section` 自己写旋律 —— 那会推翻 `line/propose-section` 已经落地的"内容取自工程里真实材料"
语义（`CLIP_NOT_FOUND`、材料逐字段保留、轮转分配等判据），属于**产品决策**而不是接线。
`section_build.rs` 模块头也早已写明"真正的写旋律在 `line/theory-core` 一侧"。本线只**探测**了它（§5.4），未接线。

### 3.4 `chord::ChordKind::has_seventh`（`chord.rs:225`）未接

它与"构成音数"等价（4 音 ⇒ 有七音），但用 `pitch_classes().len()` 可以**不写死 3/4 这两个数字**，
声部数的值本身来自 theory 的数据形状，因此选了后者。

### 3.5 `GenreRule` 其余字段（`meter` / `default_bpm_range` / `swing` / `note_density_hint` / `source`）未接

它们对**本工具**的既有输出没有承重语义（段落时长来自工程拍号、无 tempo/swing 字段），
硬接只会制造"看起来接了"的假象。留给需要它们的线（例如未来的 drum/swing 生成）。

---

## 4. `D dorian` vs `D minor` 的**真实输出差异**（逐音）

判据用的材料（`project_with_material`，`section_build.rs` 测试夹具）：
`D4(62) / Bb4(70) / B4(71) / F4(65) / D5(74)`，最低音音级 = 2 ⇒ 目标主音 `D` 时移调量 = 0，
所以下面看到的差异**只**能来自音阶（收拢），不可能来自移调。

| 输入 | 逐音输出（排序后） | 说明 |
| :--- | :--- | :--- |
| `scale = "D dorian"` | `[62, 65, 69, 71, 74]` | `B`(71) 在音阶里 ⇒ 保留；`Bb`(70) 不在 ⇒ 收拢到最近的 `A`(69)（下行平局） |
| `scale = "D minor"` | `[62, 65, 70, 70, 74]` | `Bb`(70) 在音阶里 ⇒ 保留；`B`(71) 不在 ⇒ 收拢到 `Bb`(70) |
| 差异集合 | dorian 独有 `{69, 71}`；minor 独有 `{70, 70}` | 全部落在**第六级**音级 `9=A / 10=Bb / 11=B`；两音阶的交集音 `[62, 65, 74]`（D/F/A）逐音不变 |

规范样本工程的材料是 `C E G C`（60/64/67/72，最低音音级 0）：

| 输入 | 逐音输出 | 口径 |
| :--- | :--- | :--- |
| `scale = "C minor"` | `[60, 63, 67, 72]` | `E`(64) 不在 C 自然小调 ⇒ 等距下行为 `Eb`(63) |
| `scale = "D minor"` | `[62, 65, 69, 74]` | 整体 +2 后再收拢：`F#`(66) ⇒ `F`(65) |

**收拢规则**（本层的确定性政策，theory 只提供音级集合）：不在音阶里的音高移到**最近的音阶音**；
距离相同时**优先下行**（♭ 侧）；越界做八度折叠（`shift_bytes`，`section_build.rs:560`），
音级不丢、音域始终 `0..=127`。`D chromatic` 时所有音高原样保留（判据里有反例读数）。

**接口副作用（如实登记）**：`scale` 的规范写法从 `"C minor"` 变成 `"C natural_minor"`
（模式名现在由 `ScaleKind::name()` 给出），因此它进 `section_id` 的种子文本变了 —— 这是**确定性的**变
（同一输入仍逐字节同一批 op），不是随机漂移。

---

## 5. 判据清单与注入记录

### 5.1 本机**真跑**的命令（`yeban-mcp` 传递含 rayon/hound/midly/symphonia/rubato ⇒ `crate` 档 SKIP）

```text
bash scripts/dev/cargo-local.sh build -p yeban-model          # 4.2s
bash scripts/dev/cargo-local.sh build -p yeban-theory         # 4.19s（新增依赖边；theory 只依赖 thiserror+libm）
DEPS=target/debug/deps
rustc --edition 2024 --test -D warnings crates/yeban-mcp/verify/section_pure.rs \
  --extern yeban_model=$(ls $DEPS/libyeban_model-*.rlib | head -1) \
  --extern serde_json=$(ls $DEPS/libserde_json-*.rlib | head -1) \
  --extern yeban_theory=$(ls $DEPS/libyeban_theory-*.rlib | head -1) \
  -L dependency=$DEPS -o /tmp/tw/section_pure
/tmp/tw/section_pure            # ⇒ test result: ok. 32 passed; 0 failed; 0 ignored
```

`32 = 23 (section_build) + 3 (ids) + 6 (脚手架独立判据)`。
另跑 `clippy-driver --edition 2024 --test -D warnings -D clippy::all …` ⇒ **0 告警**
（`yeban-mcp` 整 crate 的 clippy **只有 CI 能跑**，见 §7）。

### 5.2 判据 → 需求映射（硬要求 ①–⑩）

| 硬要求 | 判据（判据名） | 本机/CI |
| :--- | :--- | :--- |
| ① `D dorian` ≠ `D minor` 且差异**恰好**是音阶语义，逐音 | `section_build::tests::dorian_and_minor_differ_exactly_by_the_sixth_degree`（写死 `[62,65,69,71,74]` vs `[62,65,70,70,74]`、差异集合、交集音、`scale=None` 两次逐字节相同）＋脚手架 `dorian_and_minor_differ_exactly_by_the_sixth` | **本机真跑** |
| ② 风格 → 声部映射来自 theory（注入变红） | `section_build::tests::style_to_voices_comes_from_the_theory_genre_rule`（遍历 **182** 条流派，用 theory 原始调用独立复算声部数）＋脚手架 `every_theory_genre_maps_to_a_voice_count_from_its_own_chords`（并要求取值集合恰为 `{3,4}`） | **本机真跑** |
| ③ 未知风格仍 ⇒ `STYLE_NOT_FOUND`（错误语义未放松） | `section_build::tests::unknown_preset_is_style_not_found`（`availablePresets` **等于** `GenreLibrary::ids()`；反向：`polka`/`synthwave`/`lo_fi_hip_hop`/`funk` 必须**可用**）＋ `section.rs::tests::unknown_preset_is_style_not_found`（契约码 `STYLE_NOT_FOUND`）＋ e2e `propose_section_unknown_style_is_rejected_with_the_theory_catalogue`（候选清单 > 100 条、含 `synthwave`/`lo_fi_hip_hop`、失败不改工程/不留提案；反向 `polka` 被接受） | 第一条本机真跑；后两条**CI** |
| ④ 骨架 + 连接真的写进工程 / 逆操作逐字节回退仍全绿 | `skeleton_and_voice_routing_really_exist_after_applying`、`batch_inverse_restores_the_project_byte_for_byte`、`routing_edges_are_never_cyclic`、`skeleton_ids_are_never_nil_and_are_distinct`、`op_summary_matches_the_real_delta`（**未因换数据源而放松**：声部数由 4 改 3 的断言同步更新，语义不变） | **本机真跑** |
| ⑤ 确定性（同一输入两次 ⇒ 工程字节相同） | `plan_is_deterministic_and_applies_cleanly`（`assert_eq!(first, second)`）＋ `dorian_and_minor_…` 的 `no_scale_a == no_scale_b`＋ e2e `propose_section_creates_a_real_section_and_is_deterministic`（`op_bodies` 逐字节比较） | 前两条本机；e2e **CI** |
| ⑥ 依赖边真的存在 | 编译级（本机真跑）：去掉 `--extern yeban_theory` 后裸 `rustc` 报 **`error[E0433]: cannot find module or crate \`yeban_theory\`` → `section_build.rs:95: use yeban_theory::genre::{GenreLibrary, GenreRule};`**；运行级：`style_to_voices_…` 断言 `available_presets() == GenreLibrary::ids()`、`GenreLibrary::len()` 参与比对 | **本机真跑** |
| ⑦ 零新增**其它**依赖 | `git diff Cargo.lock` ⇒ **只有一行新增** `"yeban-theory"`（`yeban-mcp` 的依赖列表）；`git diff Cargo.toml`（根）⇒ **空**（根清单早有 `yeban-theory` 登记行） | 本机机械核对（§8） |
| ⑧ `dryRun` 预览与真实执行一致（既有语义不退化） | `op_summary_matches_the_real_delta`（摘要数 vs 真实增量）＋ e2e `propose_section_dry_run_previews_without_touching_bytes`、`propose_section_creates_a_real_section_and_is_deterministic`（`willCreate.opCount == proposal.opCount`） | 前者本机；e2e **CI** |
| ⑨ 提案线（propose → merge/reject）既有判据全绿 | e2e `propose_section_*` 全套（骨架/声部连接/逐字节逆操作/幂等/环路拒绝/负样本/`funk` 四声部）+ `contract.rs`；**负样本里 `polka` 必须换成 `yeban_unknown_style`**（`polka` 现在是 theory 的流派 ID，不再产生 `STYLE_NOT_FOUND`，否则 `exercised_error_codes` 会少一个码） | **CI**（本机跑不了；改动是机械换名 `lofi-beats → lo_fi_hip_hop` 等 + 声部数 4 → 3 + 声部名断言 + 1 条新判据） |
| ⑩ 门禁 | `bash scripts/gates/run-gates.sh light` ⇒ **`门禁通过 (mode=light)`，exit 0**；`crate yeban-mcp` 本机 SKIP（`heavy-deps.py yeban-mcp` ⇒ exit 0 "传递含重依赖 hound, midly, rayon, rubato, symphonia"），重活交 CI（§7） | 本机 light 真跑 |

### 5.3 判据全表（本机 32 条真跑）

`section_build::tests`（23）：`note_names_and_pitch_classes_are_aligned`、`scale_semantics_come_from_theory`、
`material_notes_snap_to_the_nearest_scale_tone`、`dorian_and_minor_differ_exactly_by_the_sixth_degree`、
`style_to_voices_comes_from_the_theory_genre_rule`、`unknown_preset_is_style_not_found`、
`bars_and_scale_are_validated`、`plan_is_deterministic_and_applies_cleanly`、
`skeleton_and_voice_routing_really_exist_after_applying`、`batch_inverse_restores_the_project_byte_for_byte`、
`planning_never_mutates_the_project`、`missing_material_is_a_clear_clip_not_found`、
`missing_master_bus_is_a_clear_track_not_found`、`section_name_style_bars_and_scale_really_change_the_output`、
`unwired_is_derived_from_the_real_ops`、`a_second_identical_batch_cannot_be_applied_twice`、
`op_summary_matches_the_real_delta`、`cycle_detection_finds_and_reports_cycles`、
`a_cyclic_project_is_refused_before_arranging`、`a_master_bus_missing_from_the_node_table_is_added_once`、
`an_existing_master_bus_node_is_not_added_twice`、`parts_round_robin_over_the_available_materials`、
`material_content_is_preserved_except_pitch_and_identity`；
`ids`（3，既有）；脚手架独立口径（6）：`routing_edges_are_never_cyclic`、
`skeleton_ids_are_never_nil_and_are_distinct`、`missing_material_reports_the_typed_fault`、
`dorian_and_minor_differ_exactly_by_the_sixth`、
`every_theory_genre_maps_to_a_voice_count_from_its_own_chords`、`the_preset_catalogue_is_the_theory_library`。

### 5.4 探针（先量再设计；不是判据，但决定了设计）

`/tmp/tw/probe*.rs`（裸 `rustc` + `--extern yeban_theory`）逐条打印了：
182 条流派的音阶长度分布、首个和弦构成音数分布、全部走向里和弦构成音数的直方图（`{3: 181, 4: 1}`）、
`realize(THREE/FOUR)` 的可行性与 `max_voice_jump` 分布、`GenreLibrary::get` 对旧预设名的判定
（`cinematic-orchestral` / `lofi-beats` / `acoustic-folk` 全部 **不存在**；`polka` / `synthwave` **存在**）。

### 5.5 注入 → 变红 → 还原（**每条都真做过，退出码/判据名为准**）

方法：`cp` 快照 → Python 定点替换（每次 `assert t.count(old) == 1`）→ 重编译重跑脚手架 →
记红点 → 从快照还原 → `shasum -a 256` 比对**逐字节**一致 → 再跑一遍全绿。
被注入文件：`crates/yeban-mcp/src/domain/section_build.rs`，注入前/还原后 sha256 均为
**`a47dc7d4c80d4f3b5d68985bcdc2668c26e71655ff286e06712fc8ef0504b001`**（= 本线提交的那一份）。

| # | 注入（把 theory 的查询换成硬编码常量） | 变红的判据（实测） | 还原 |
| :--- | :--- | :--- | :--- |
| **I1** | 音阶查询 → 常量：`theory_scale = Some(TheoryScale::new(C, Major))` | `28 passed; 4 failed` —— 脚手架 `dorian_and_minor_differ_exactly_by_the_sixth`；lib `dorian_and_minor_differ_exactly_by_the_sixth_degree`、`material_notes_snap_to_the_nearest_scale_tone`、`section_name_style_bars_and_scale_really_change_the_output` | ✅ sha256 一致 |
| **I2** | 声部数 → 常量：`let count = 3;`（保留 `genre_voice_count` 调用以免 dead_code） | `29 passed; 3 failed` —— 脚手架 `every_theory_genre_maps_to_a_voice_count_from_its_own_chords`；lib `style_to_voices_comes_from_the_theory_genre_rule`、`section_name_style_bars_and_scale_really_change_the_output`（`funk` 的 4 声部） | ✅ sha256 一致 |
| **I3** | 风格清单退回 D49 之前的本地 4 行表：`available_presets()` 返回 `cinematic-orchestral/lofi-beats/synthwave/acoustic-folk` | `29 passed; 3 failed` —— 脚手架 `the_preset_catalogue_is_the_theory_library`；lib `unknown_preset_is_style_not_found`（候选清单 ≠ theory）、`style_to_voices_comes_from_the_theory_genre_rule` | ✅ sha256 一致 |

还原后复核：`/tmp/tw/section_pure` ⇒ `32 passed; 0 failed`；`grep -rn 'INJECT' crates/yeban-mcp/` ⇒ **0 命中**（L24）。

---

## 6. 未接项 / needs（本线**没有**改 theory 的源码）

| # | needs | 性质 | 建议 |
| :--- | :--- | :--- | :--- |
| **needs-shape-1** | `yeban-theory` **没有乐器/声部命名**概念 | theory 能力缺口（`grep instrument` = 0 命中） | `yeban_propose_section` 的声部名仍是本层角色词表（`Bass/Alto/Soprano`、`Bass/Tenor/Alto/Soprano`）。若要有"流派 → 配器（Strings/Brass/Percussion…）"的真实规则，需要在 **theory**（或 `yeban-services`）里登记"配器预设"数据，MCP 再按 D49 的方式消费它。在此之前本层不假装它存在（构成音数 ∉ {3,4} 时如实 `CONFLICT`） |
| **needs-shape-2** | theory 没有"把**外部材料**映射进音阶"的 API | theory 能力缺口 | 本层的 `snap_into_scale`（最近音 + 平局下行 + 八度折叠）是**政策**；theory 只提供 `contains` / `pitch_classes` / `degree_of`。若别的线也需要同一口径，应把它做成 theory 的公开函数（一个"音阶内收拢"原语），否则第二个调用方会各写一份 |
| **needs-shape-3** | 旧预设词 `cinematic-orchestral` / `lofi-beats` / `acoustic-folk` 已**不再存在** | 契约影响（规范 §7.2 只写 `stylePreset: String`，`schemas/mcp-tools.schema.json` **没有**枚举约束） | `stylePreset` 的合法值现在是 `<GenreLibrary 的流派 ID>`（182 条）。替代关系建议：`cinematic-orchestral → orchestral_film_score`、`lofi-beats → lo_fi_hip_hop`、`synthwave → synthwave`（同名）、`acoustic-folk → folk`。**其它台账/文档里若引用旧名，以本条为准** |
| **needs-shape-4** | `docs/ledger/dependency-licenses.md` 的 `Cargo.lock` 指纹行 | 生成物（脚本写） | 本线跑了 `python3 scripts/gates/license_inventory.py` 重新生成，diff **只有 1 行**（`Cargo.lock` SHA-256 前 16 位）；外部依赖包数仍是 **618**（没有新第三方包）。集成者合并多个改 `Cargo.lock` 的分支后需**重跑**该脚本 |
| **needs-shape-5** | `docs/ledger/tools-domain-notes.md` 的 **needs-9** 行 | 台账一致性 | needs-9 记的缩水（"`scale` 只用主音音级 ⇒ `D dorian` ≡ `D minor`"）**已由本线关闭**。按本线授权（"只改 needs-6 相关行"）该行**未改**，建议集成者合并时把 needs-9 标为已关闭并指向本文件 |

仍未接的（**不是** needs，是本线明确的范围外）：`voice_leading::*`（§3.3）、
`GenreRule::{meter, default_bpm_range, swing, note_density_hint, source}`（§3.5）、
`GenreRule::{sketch, chords}`（**完全未调用**：`sketch`/`chords` 只看第一条典型走向，会丢掉 `funk` 的七和弦，见 §3.2）。

---

## 7. 本机真跑 vs CI（**严格区分**）

| 项 | 本机（Apple M2，受限沙箱） | CI |
| :--- | :--- | :--- |
| `cargo` 全量 / `clippy -p yeban-mcp` / e2e 判据 | **跑不了**：`yeban-mcp` 传递含 rayon/hound/midly/symphonia/rubato ⇒ `run-gates.sh crate yeban-mcp` **SKIP**（`heavy-deps.py yeban-mcp` exit 0） | `rust (workspace 全量)` 腿（本轮 plan 选了全量腿）：`cargo clippy --workspace --all-targets --locked -- -D warnings` + `cargo test --workspace --all-targets --locked` ⇒ **已由 run 37254805472 关闭** |
| `src/domain/section_build.rs` 的逻辑与判据 | **真跑**（裸 `rustc --edition 2024 --test -D warnings` + `--extern yeban_theory`）：**32 passed; 0 failed**；`clippy-driver -D clippy::all` 0 告警 | 同一份源文件（CI 也编它） |
| 本线改到的 lib 层文件（`section.rs` / `samples.rs` / `domain/mod.rs`） | **只做了类型/文本级核对**，未编译 | 上面的 `clippy --workspace --all-targets` + `test --workspace --all-targets` 覆盖 |
| `run-gates.sh light` | **真跑**：`门禁通过 (mode=light)`，exit 0 | `checks` 腿跑同一脚本的相应部分 |
| 判决 | 本机绿**不是**判决 | 判决 = `bash scripts/dev/ci-verdict.sh --watch line/theory-wiring` 读回的 run —— 见 §7.1 |

### 7.1 代码那一轮的判决（**这一轮才算判决**，L23）

```text
$ bash scripts/dev/ci-verdict.sh --watch line/theory-wiring     # 退出码 0 ⇒ 判决属于本线 tip
✓ line/theory-wiring CI · 37254805472   (triggered via push; tip = dba2b19)
✓ lockfile (确定性 Cargo.lock) 16s        ✓ deny (cargo-deny 开源合规) 57s
✓ plan (受影响集合) 5s                    ✓ checks (fmt / 红线守卫 / schema) 43s
✓ windows (yeban-mcp / yeban-model 的平台分支) 1m50s
✓ rust (workspace 全量) 4m13s             - rust (${{ matrix.crate }}) 0s（plan 选了全量腿）
```

判决 = **`conclusion: success`**，且它覆盖了本线的**全部**判据：

- `plan` 判定"全工作区受影响"（本线动了根 `Cargo.lock` ⇒ 命中 ROOT_TRIGGERS），
  因此跑的是 `.github/workflows/ci.yml` 的 **`rust (workspace 全量)`** 腿，它的两步是
  `cargo clippy --workspace --all-targets --locked -- -D warnings` 与
  `cargo test --workspace --all-targets --locked`（见该 workflow 第 262–271 行）
  ⇒ **`yeban-mcp` 的 lib 判据 + `tests/tools_e2e.rs` + `section.rs` 判据都真的在 CI 上跑过并全绿**，
  本机跑不了的那部分（§7 上表"本机跑不了"三行）由此关闭；
- `windows` 腿也绿（`yeban-mcp` / `yeban-model` 的平台分支）；
- `checks` 腿跑的是与本机同一条 `run-gates.sh` 相应步骤（fmt / 红线守卫 / schema）。

**文档提交的节奏（L23/L26）**：本节与 `tools-domain-notes.md` 的 needs-6 回填是**代码提交之后**的
第二个 docs-only 提交，以免 `ci.yml` 的 `concurrency.cancel-in-progress` 把代码那一轮的 run 吃掉。
⇒ **代码判决一律以 tip `dba2b19` 的 run `37254805472` 为准**；docs-only 那一轮的 run 即使绿，
也只证明文档不破坏 fmt/守卫/schema（它会按受影响集合跳过 rust 腿），**不构成**对代码的判决。


---

## 8. 依赖边证据与 `Cargo.lock` diff 范围（硬要求 ⑥⑦）

```text
$ git diff Cargo.toml                    # 根清单
（空 —— `yeban-theory` 早已登记在 [workspace.dependencies]，本线**没有**动根清单）

$ git diff Cargo.lock
@@ -6227,6 +6227,7 @@ dependencies = [
  "yeban-decode",
  "yeban-model",
  "yeban-render",
+ "yeban-theory",
 ]
（**只此一行**：成员之间的依赖边；包集合不变、无第三方包增删）

$ git diff --stat Cargo.lock Cargo.toml
 Cargo.lock | 1 +
 1 file changed, 1 insertion(+)
```

`crates/yeban-mcp/Cargo.toml` 只新增了 `yeban-theory.workspace = true` 一行（+ 说明注释），
版本仍由根清单唯一决定；`cargo metadata` / `heavy-deps.py` 在本机都能跑通
（`yeban-theory` 的 `heavy-deps.py` exit 1 = 轻依赖，因此裸 rustc 脚手架在本机可跑）。

---

## 9. 修改文件与净行数

| 文件 | 变化 | 说明 |
| :--- | :--- | :--- |
| `crates/yeban-mcp/src/domain/section_build.rs` | `+827 / -…` | 删本地表、接 theory、加判据与夹具；新增 `genre_voice_count` / `part_names` / `snap_into_scale` / `shift_bytes` / `mode_names` / `canonical_kind` / `available_presets` / `theory_conflict` |
| `crates/yeban-mcp/Cargo.toml` | +21 | 依赖边声明 + 说明（**只**加 `yeban-theory.workspace = true`） |
| `crates/yeban-mcp/verify/section_pure.rs` | `+114 / -…` | 编译命令加 `--extern yeban_theory`；新增 3 条独立口径判据；旧预设名换新 |
| `crates/yeban-mcp/tests/tools_e2e.rs` | `+119 / -…` | 预设名换新 + 声部数 4→3 + `funk` 的四声部判据 + 声部名断言 + 1 条新判据（未知风格 ⇒ `STYLE_NOT_FOUND`）+ 负样本 `polka` → `yeban_unknown_style` |
| `crates/yeban-mcp/src/domain/section.rs` | `+30 / -…` | `pub use` 清单（`MODES`/`STYLE_PRESETS` → `mode_names`/`available_presets`）+ 判据 |
| `crates/yeban-mcp/src/domain/mod.rs`、`src/samples.rs` | 各 1 行 | 夹具/样本里的预设名换新 |
| `Cargo.lock` | +1 | 依赖边（**只**此一行） |
| `docs/ledger/tools-domain-notes.md` | 1 行 | **needs-6 → 已接线**（就地改写，保留"当时"表述） |
| `docs/ledger/dependency-licenses.md` | 1 行 | 生成物（`Cargo.lock` 指纹） |
| `docs/ledger/theory-wiring-notes.md` | 新增（324 行，含 §7.1 回填） | 本文件 |

净行数：

- **代码提交** `dba2b19`（`git show --stat`）：**11 files changed, 1243 insertions(+), 175 deletions(-)**
  —— 含本文件的首版（298 行）；
- 其中的**机械改动**（不含本文件）：**10 files changed, 945 insertions(+), 175 deletions(-)**
  （`git diff --stat` 提交前实测）；
- **第二个 docs-only 提交**（本文件 §7.1 与 needs-6 的 run id 回填）：只改
  `docs/ledger/theory-wiring-notes.md` 与 `docs/ledger/tools-domain-notes.md` 两行。

**提交纪律**（L23/L26/L28）：`git add -A` 前先看 `git diff --cached --stat`（11 个文件，全部在本线地盘内，
无 `__pycache__` / `*.log` / `*.bak`）；代码 + 判据 + 文档在**第一批**推送（`dba2b19`）并等判决，
读回 run `37254805472` 后才做**第二个 docs-only 提交**回填 run id，
以免 `concurrency.cancel-in-progress` 吃掉代码那一轮的 run。
