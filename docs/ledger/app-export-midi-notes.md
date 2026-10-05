# app-export-midi 工作线台账（`--export-midi`：MIDI 导出的**第一个出口**）

> **一句话**：`crates/yeban-render/src/midi.rs`（SMF 0/1 导出与严格回读）此前**零消费者**
> （`grep -rn "yeban_render::midi\|render::midi" crates/*/src crates/*/tests` 命中 0），
> 本线给它接上**第一个出口** = app CLI `--export-midi <path>`，即
> ADR-0001 **D47** 的裁决原文：**MIDI 导出的唯一出口 = app CLI `--export-midi`**
> （**不**扩 `yeban_render_master` 的 `format` 参数）。它同时是 `.als` 导出
> （`ROAD-M4-007`）的**唯一前置能力** —— 前置能力以前是"实现了但没有任何出口"。
>
> **本线没有**再写一份 SMF 编码器：字节全部出自 `yeban_render::midi`。

---

## 0. 交付映射（规范 ID → 落点 → 判据）

| 规范 / 裁决 | 落点 | 判据（见 §4） |
| :--- | :--- | :--- |
| `[ARCH-FMT-001 §5.5]`（SMF 0/1 导出） | [`crates/yeban-render/src/midi.rs`](../../crates/yeban-render/src/midi.rs) 的**既有**编码器（本线**未改**该文件） | ② ③ ④ + 该文件自带的 12 条（本机真跑） |
| `[MODEL-AST-001]`（960 PPQ 整数时钟） | `MThd` 的时间分度 = `yeban_model::PPQ`，并与 `yeban_render::midi::DEFAULT_PPQ` 对账 | ③（含漂移 ⇒ 拒绝导出） |
| `[MODEL-AST-003]`（`BTreeMap` 确定性 / 红线 4） | 轨道 / 摆放 / 音符一律按身份升序展开 | ④（逐字节确定性） |
| `[ARCH-SEC-004]`（原子落盘） | 复用 `save::write_file_atomically`（**没有**第二份原子写入） | ⑥ |
| `ADR-0001 D47`（唯一出口） | `crates/yeban-app/src/cli.rs` 的 `--export-midi` | ⑦ + §1 |
| `ADR-0001 D43`（无兼容包袱） | 非容器输入复用 `--open` 的**退出码 3**；不发明新码（D25） | ⑤ |
| `ADR-0001 D28`（唯一实现） | 映射只有一份（[`export_midi.rs`](../../crates/yeban-app/src/export_midi.rs)），报告只有一份（`cli::emit`） | ①②③④ |
| `ADR-0001 D51`（**集成者裁决**：允许工作区内依赖边） | `yeban-app → yeban-render` 一条内部边（见 §6） | 实测 diff = 1 行 |

修改的文件与净行数（`git diff --stat` 口径，新文件按全文计）：

| 文件 | 性质 | 净变化 |
| :--- | :--- | ---: |
| [`crates/yeban-app/src/export_midi.rs`](../../crates/yeban-app/src/export_midi.rs) | **新增**（映射层 + 13 条判据） | +828 |
| [`crates/yeban-app/src/cli.rs`](../../crates/yeban-app/src/cli.rs) | 开关 / 用法 / 退出码 / 报告 + 5 条判据 | +449 / −14 |
| [`crates/yeban-app/tests/cli_contract.rs`](../../crates/yeban-app/tests/cli_contract.rs) | 真二进制判据 B12 / B13 | +155 |
| [`crates/yeban-app/src/lib.rs`](../../crates/yeban-app/src/lib.rs) | 模块挂载 + 文档 | +7 |
| [`crates/yeban-app/src/main.rs`](../../crates/yeban-app/src/main.rs) | 命令表加一行 | +1 |
| [`crates/yeban-app/Cargo.toml`](../../crates/yeban-app/Cargo.toml) | 内部依赖边（D51，含理由注释） | +16 |
| `Cargo.lock` | **机械重生成**（`cargo metadata`） | **+1 行**（见 §6） |
| [`docs/ledger/dependency-licenses.md`](dependency-licenses.md) | **机械重生成**（`license_inventory.py`） | +1 / −1（锁哈希） |
| 本文件 | **新增** | — |

---

## 1. 命令形状（`--help` 原文，实测）

真二进制（本机探针，与 `main.rs` 的无窗口分发逐行同构）`--help` 里与本次相关的原文：

```text
  --export-midi <path>     把当前工程导出成**标准 MIDI 文件** (SMF 1): conductor 轨
                           (tick 0 的 tempo + 拍号) + 每条含 MIDI 的轨道一条 MTrk
                           (通道按导出顺序 0,1,2,…); **PPQ 与工程一致 (960)**;
                           字节由 yeban-render 的**唯一** SMF 编码器产出 (ADR-0001 D47),
                           与 --export-elements / --save-as 共用同一份**原子**落盘实现
```

```text
  --export-elements 与 --export-midi 与 --save-as 任意组合
                               顺序固定: **先**导出元素, **再**导出 MIDI,
                               **最后**保存工程; 任一导出失败 ⇒ 不写工程 (退出码 5)
  --export-midi 不给 --open    导出的是演示工程, 输出 `exported-midi: ... from=sample=...` 明说
  --open / --save-as / --export-elements / --export-midi
                               各只能给一次; 重复给 = 用法错误 (退出码 2)
```

```text
  5 --export-elements 失败 / --export-midi 失败 (I/O;
      或工程里没有可导出的 MIDI 音符 / 拍号分母不是 2 的幂 /
      工程 PPQ 与编码器默认 PPQ 不一致 / 编码器拒绝越界的音高或力度)
```

```text
  yeban-app --open song.yeban --export-midi song.mid
```

**报告行**（`key=value`，与 `saved:` 同族的稳定格式）：

```text
exported-midi: path=song.mid bytes=133 ppq=960 format=parallel tracks=1 notes=6 tempos=1 \
  temp=song.mid.tmp-01M44X63ZEN67B0GZ7XFK44KVB from=demo.yeban (format=yeban-container)
```

**退出码**（**不发明新码**，ADR-0001 D25 的口径）：

| 情形 | 退出码 | 出处 |
| :--- | ---: | :--- |
| `--export-midi` 成功 | `0` | `EXIT_OK` |
| `--open` 失败（非容器 / 截断 / 缺件 / 读不到 …） | **`3`** | 复用 `EXIT_OPEN`（与 `--open` 同一语义） |
| 用法错误（缺取值 / 重复 / 未知开关） | `2` | `EXIT_USAGE` |
| 导出失败（无 MIDI 内容 / 悬空片段 / 拍号不可表达 / PPQ 漂移 / 编码被拒 / 落盘 I/O） | **`5`** | 复用 `EXIT_EXPORT`（与 `--export-elements` 同档） |

---

## 2. 导出映射表（模型字段 → MIDI 事件，**逐项**）

### 2.1 工程 → 文件

| 工程侧 | SMF 侧 | 判据 |
| :--- | :--- | :--- |
| — | 格式恒为 **SMF 1**（`MThd` 格式号 `0x0001`） | ① ③ |
| `yeban_model::PPQ`（960） | `MThd` 时间分度 = `0x03C0`，**不换算、不取整** | ③ |
| `project.bpm` | conductor 轨 `FF 51 03` Tempo（`mpqn = round(60_000_000 / bpm)`，由编码器算） | ⑤ |
| `project.time_signature.numerator` / `.denominator` | conductor 轨 `FF 58 04 nn dd 18 08`（`dd = log2(denominator)`；`cc=24` / `bb=8` 是编码器写死的惯例值） | ⑨ |
| 没有别的时间点 | tempo map **只有 tick 0 一条**（工程只有一个顶层 `bpm`） | ⑤ |

### 2.2 轨道 → `MTrk` / 通道

| 工程侧 | SMF 侧 |
| :--- | :--- |
| （conductor） | `MTrk#0`，名 `Yeban Conductor`（编码器写死） |
| `project.tracks` 的 `BTreeMap` **身份升序**遍历，跳过 `master_bus_track_id` | 每条**含至少一颗音符**的轨道 → 一条 `MTrk`；空轨**不产生**（不写空 `MTrk`） |
| 第 `i` 条被导出的轨道（`i` 从 0 计） | MIDI 通道 `i % 16`；`i ≥ 16` 时**复用**通道（如实记录在 `tracks=` 里，不静默丢弃） |
| `TrackV3::name` | `FF 03` TrackName（空名不写 meta —— 编码器的既有语义） |
| `track.clips` 的 `BTreeMap` 身份升序；`placement.muted == true` **跳过** | 摆放按身份升序展开 |
| `ClipContent::Audio` | **不导出**（音频→MIDI 不在本切片） |
| 主总线轨道 | **不导出**（声学出口 ≠ 内容轨；与 `bridge` 的投影口径一致） |

### 2.3 `MidiNote` 字段 → 事件

| `MidiNote` 字段 | 去向 |
| :--- | :--- |
| `start_tick` | `NoteOn` 的**绝对** tick = `placement.start_tick + note.start_tick`（`saturating_add`，不 wrap） |
| `duration_ticks` | `NoteOff` 的绝对 tick = 起始 + 时值 |
| `pitch` | `NoteOn` / `NoteOff` 的 `key` |
| `velocity` | `NoteOn` 的 `vel`（`NoteOff` 的力度恒 0 —— 编码器语义） |
| `micro_timing_ticks` | **并入起始 tick**（由 `yeban_render::midi::effective_start` 做：`start + micro`），与实时引擎的 `placement_start + note.start_tick + micro` 同口径 |
| `id` | 不进 SMF（只用于错误上报） |
| `probability` / `ratchet` / `slide` / `pitch_bend_curve` / `syllable` / `phonemes` | **不导出**（`midi.rs` 已登记的边界） |

**同 tick 的全序**由编码器的 `tie_break`（NoteOff → NoteOn → Tempo → TimeSignature）保证，
因此"同一工程两次导出逐字节相同"是内容决定的，不是输入顺序决定的 → 判据 ④。

### 2.4 与"播放路径"的两处**有意的**差异（都在 §7 登记）

| 主题 | `yeban-engine` 播放（[`snapshot.rs`](../../crates/yeban-engine/src/snapshot.rs)） | 本导出 | 理由 |
| :--- | :--- | :--- | :--- |
| 摆放窗口 | 把音符 **clamp** 到 `placement` 的 `[start, start+duration]` | **不裁剪**，如实写作者写下的 tick | 与 `yeban-mcp` 的 `yeban_render_master` 同口径；导出裁剪会**静默**改变作者内容。这是**待裁决**项（§8 needs-4） |
| 触发概率 / 连击 | 按 `rng_seed` 判定 `probability`，按 `ratchet` 展开 | 原样导出，不判定、不展开 | `midi.rs` 的已登记边界；判定需要 RNG，会让"同一工程两次导出"依赖种子 |

---

## 3. 实测输出（本机真跑，原始读数）

### 3.1 成功路径

```console
$ /tmp/app-export-midi-harness/probe --open demo.yeban --export-midi song.mid --headless
headless ok
opened: path=demo.yeban bytes=7683 format=yeban-container
project: id=01J8Z5Q0R7K3M9X2V4B6N8P0P0 bpm=120.00 ts=4/4 title="夜半 Yeban"
project-counts: tracks-all=7 master-track=1 scenes=4 sections=4 clips-pool=2 midi-notes=6 …
view-counts: tracks=6 master=1 clips=3 notes=6 sections=4 scenes=4 elements=214 dynamic-regions=14
headless: 未构造 MainWindow, 未初始化 Slint 后端, 未渲染任何像素 —— …
exported-midi: path=song.mid bytes=133 ppq=960 format=parallel tracks=1 notes=6 tempos=1 \
  temp=song.mid.tmp-01M44X63ZEN67B0GZ7XFK44KVB from=demo.yeban (format=yeban-container)
exit=0
```

`MThd` 头 16 字节（`xxd`）：`4d54 6864 0000 0006 0001 0002 03c0`
= `MThd` + 长度 6 + 格式 **1** + 轨道数 **2** + 时间分度 **0x03C0 = 960**。

### 3.2 **第三方独立**核对（手写 SMF 解析，不依赖 `midly`，也不依赖仓库的 `vlq.rs`）

```text
MThd len=6 format=1 ntrks=2 division=960 (=0x03c0)
-- MTrk#0 len=38
   t=     0 meta 0x03 596562616e20436f6e647563746f72      # "Yeban Conductor"
   t=     0 meta 0x51 07a120                              # tempo 500000 µs = 120 BPM
   t=     0 meta 0x58 04021808                            # 4/4, cc=24, bb=8
   t=     0 meta 0x2f
-- MTrk#1 len=65
   t=     0 meta 0x03 e9bc93                              # "鼓"
   t=     0 ch0 NoteOn  key=60 vel=100
   t=   480 ch0 NoteOff key=60 vel=0
   t=   480 ch0 NoteOn  key=64 vel=100
   t=   960 ch0 NoteOff key=64 vel=0
   t=   960 ch0 NoteOn  key=67 vel=100
   t=  1440 ch0 NoteOff key=67 vel=0
   t=  1440 ch0 NoteOn  key=72 vel=100
   t=  1920 ch0 NoteOff key=72 vel=0
   t=  1920 ch0 NoteOn  key=74 vel=100
   t=  2400 ch0 NoteOff key=74 vel=0
   t=  2400 ch0 NoteOn  key=76 vel=100
   t=  2880 ch0 NoteOff key=76 vel=0
   t=  2880 meta 0x2f
total bytes: 133
```

（演示夹具的六颗音符是 `start = 480×i`、`pitch = [60,64,67,72,74,76]`、时值 480、力度 100 —— 逐项对得上。）

### 3.3 失败路径

```console
$ probe --open junk.bin --export-midi never.mid
yeban-app: 打开 `junk.bin` 失败: `junk.bin` 不是 `.yeban` 容器 (容器裁决: end-of-central-directory record not found)
exit=3          # 复用 --open 的语义; never.mid 不存在

$ probe --open demo.yeban --export-midi missing/out.mid
yeban-app: 导出 MIDI 到 `missing/out.mid` 失败: 写临时文件 `missing/out.mid.tmp-01M44X6406VN4NBDM6F56WJGXJ` 失败: No such file or directory (os error 2)
exit=5          # 精确到"哪一步、哪个路径"; 不留半个文件, 不留临时文件
```

---

## 4. 判据清单

### 4.1 本机**真跑**（探针，非复制品）

探针在 `/tmp/app-export-midi-harness/`（**仓库之外**，不入库），用 `#[path]` 指向仓库原件。
`midly` 0.5.3 是**零依赖**上游，单独用 `rustc` 编 rlib（0.3 s，`--cap-lints allow`），
因此 `yeban-render` 的 `vlq.rs` + `midi.rs` **原件**可以在本机真跑，
而**不需要**编译 `rayon` / `hound` / `sha2`（它们的其余模块才需要）。

| 步骤 | 命令（工作树内） | 实测结果 |
| :--- | :--- | :--- |
| ① `yeban-render` 的 `vlq.rs` + `midi.rs` 自带判据（仓库原件） | `bash /tmp/app-export-midi-harness/run.sh` | ✅ **18 passed; 0 failed** |
| ② `yeban-app` 零 Slint 半边（`cli` / `export_midi` / `bridge` / `open` / `save` / … 仓库原件） | 同上 | ✅ **122 passed; 0 failed** |
| ③ 真进程判据 `tests/cli_contract.rs`（探针二进制当被测进程） | `bash /tmp/app-export-midi-harness/verify.sh` | ✅ **14 passed; 0 failed**（B1–B13） |
| ④ clippy（两个探针 crate，`-D warnings -D clippy::all`，探针根**不再** allow `clippy::all`） | `bash /tmp/app-export-midi-harness/clippy.sh` | ✅ **零告警**（§5-7：这条一度是空转的，已修） |
| ⑤ `cargo fmt --all --check` | `bash scripts/dev/cargo-local.sh fmt --all --check` | ✅ 通过 |
| ⑥ 门禁 light（**无管道**，直接读退出码） | `bash scripts/gates/run-gates.sh light` | ✅ **门禁通过 (mode=light)**，`exit=0` |

**本线新增 / 修改的判据**（编号沿用 `cli.rs` 的顺序，接在 41 之后）：

| # | 判据 | 断言 |
| :--- | :--- | :--- |
| ① `export_midi::tests::exported_bytes_round_trip_through_the_smf_reader` | 导出字节能被 SMF 读取面读回 | `track_chunks` = `MThd`(len 6) + conductor + N 条 `MTrk`；每条以 `FF 2F 00` 收尾；`parse_smf` 得到 SMF 1 与 6 颗音符 |
| ② `export_midi::tests::exported_notes_match_the_demo_fixture_note_by_note` | 逐音符一致 | `(通道, 音高, 力度, 起始 tick, 时值)` 六元组**逐一相等**；期望值是**独立**写下的常量（不是从实现导出） |
| ②b `export_midi::tests::placement_start_is_added_to_the_note_tick` | 绝对 tick 口径 | 把摆放起点改成 960 ⇒ 每颗音符整体后移 960（与 engine 的 `placement_start + note.start_tick` 同口径） |
| ③ `export_midi::tests::ppq_header_is_the_project_ppq_and_the_encoder_default_agrees` | **PPQ 头字段 == 960** | `bytes[12..14] == [0x03, 0xC0]`；`u16::from_be_bytes == 960`；且 `DEFAULT_PPQ == PPQ == 960` 两个事实源对账 |
| ③b `export_midi::tests::ppq_drift_between_model_and_encoder_is_rejected_by_construction` | 漂移即拒绝 | 不一致时 `MidiExportError::PpqMismatch` 的文案逐字断言（"拒绝导出 (不许偷偷换算)"） |
| ④ `export_midi::tests::two_exports_of_the_same_project_are_byte_identical` | 逐字节确定性 | 演示夹具与 `filled` 样本各导出两次 ⇒ `==` |
| ⑤ `cli::tests::export_midi_on_a_non_container_exits_three_and_writes_nothing` | 非容器 ⇒ **3** | `EXIT_OPEN == 3`；stderr 含"不是 `.yeban` 容器"；目标文件**不存在**；空工程 ⇒ 导出档（5）且无文件 |
| ⑥ `cli::tests::export_midi_into_an_unwritable_path_exits_five_without_a_half_file` + `export_midi::tests::a_failed_write_leaves_no_half_file` + `a_read_only_directory_never_touches_the_existing_file`（unix） | 不可写 ⇒ 5 + 原子性 | 精确到"写临时文件"；目标不存在；目录里无 `.tmp-` 残留；只读目录下**已有文件一字未改** |
| ⑦ `cli::tests::help_lists_every_switch_and_short_circuits` + `tests/cli_contract.rs` B1 | `--help` 里有 `--export-midi` | 开关列表 + 退出码 5 的文案 + 示例都在用法里 |
| ⑤/②/③/④ 的**进程级**复证 | `tests/cli_contract.rs` B12 | 真二进制的 `exported-midi:` 行字段、文件字节、`MThd` PPQ、逐音符、两次导出 `==`、无 `.tmp-` 残留、无 panic |
| ⑤/⑥ 的**进程级**复证 | `tests/cli_contract.rs` B13 | 真进程退出码 **3** / **5**、stderr 原文、目标与目录都不被创建 |
| ⑤ `export_midi::tests::filled_sample_maps_its_micro_timing_into_the_start_tick` | 规范样本映射 | `micro_timing_ticks = -12` ⇒ 起点 1908；tempo = 468750（128 BPM） |
| ⑥b `export_midi::tests::each_midi_track_gets_its_own_chunk_and_channel` | 多轨 → 多 `MTrk` / 多通道 | 两条含 MIDI 的轨道 ⇒ 通道 `0,1`；`MThd + 3` 个 chunk；每通道 4 颗音符 |
| ⑦ `export_midi::tests::muted_placements_are_skipped_and_empty_projects_are_refused` / `a_dangling_clip_reference_is_a_precise_error` / `time_signature_denominator_maps_to_its_power_of_two` / `cli::tests::export_midi_runs_before_save_as_and_blocks_a_failed_save` / `export_midi_without_open_exports_the_demo_project_and_says_so` / `combination_semantics_are_exactly_the_documented_table` | 边界与组合语义 | 静音摆放跳过；空工程拒绝；悬空片段点名（不静默跳过）；分母非 2 的幂拒绝；顺序 `exported:` → `exported-midi:` → `saved:`；导出失败 ⇒ 不写工程；无 `--open` 时 `from=sample=…` 明说 |

### 4.2 注入 → 变红 → 还原（**2 条，全部真做过**）

方法：源文件先备份到 `/tmp/app-export-midi-backup/`，注入后用两个探针重跑，
记录红点，再从备份还原并 `cmp` + `md5` 逐文件确认 identical。

| # | 注入（任务指定的形态） | 改法 | 实测红点 |
| :-- | :--- | :--- | :--- |
| A | **把 PPQ 写成 480**（应让 ③ 红） | `crates/yeban-render/src/midi.rs` 的 `pub const DEFAULT_PPQ: u16 = 960` → `480` | `export_midi::tests::ppq_header_is_the_project_ppq_and_the_encoder_default_agrees`: `left: 480, right: 960`（"编码器默认 PPQ 必须等于工程 PPQ (960)"）；连带 `ppq_drift_between_model_and_encoder_is_rejected_by_construction`: `left: 480, right: 480`。**总读数 105 passed / 17 failed**（导出被 `PpqMismatch` 拒绝 ⇒ 所有走导出路径的判据一起红，这正是"拒绝而不是偷偷换算"的形态）。真进程：`export_midi_writes_a_parseable_deterministic_smf_from_the_real_binary` **FAILED**（13 passed / 1 failed）。**`midi.rs` 自带的 18 条仍绿** —— 它们用符号常量而非字面量，这是"只有 app 侧那条断言钉住 960"的诚实读数 |
| B | **把 tick 换算改成"beat 取整"**（应让 ② 红） | `export_midi.rs` 的 `start_tick: placement.start_tick + note.start_tick` → `raw / PPQ * PPQ`（960 tick = 一拍） | `export_midi::tests::exported_notes_match_the_demo_fixture_note_by_note`: 逐音符 diff 原文 `left: [… (0,64,100,0,480), (0,72,100,960,480), (0,76,100,1920,480) …]` vs `right: [… (0,64,100,480,480), (0,72,100,1440,480), (0,76,100,2400,480) …]`（480→0 / 1440→960 / 2400→1920）；`placement_start_is_added_to_the_note_tick` 与 `cli::tests::export_midi_writes_a_parseable_smf_whose_notes_match_the_project` 同红。**总读数 119 passed / 3 failed**。真进程：B12 **FAILED**（13 / 1） |

还原复核：`md5` 与注入前逐字节一致
（注入当时的哈希：`midi.rs` = `fbdd36bf355a61c8187c6df18fec5386`，
`export_midi.rs` = `5c9dba3a5933511a792c42e84371e73b`；⚠ `export_midi.rs` 此后因
`clippy::filter_next` 又改了一处**测试**代码，见 §5-7 ⇒ 现在它的哈希已不同），
`cmp` identical，`grep -rn "INJECT" crates/` = **0 命中**；还原后复跑：

```text
① yeban-render: 18 passed; 0 failed
② yeban-app   : 122 passed; 0 failed
③ 真进程      : 14 passed; 0 failed
```

---

## 5. 本机真跑 vs CI：**严格区分**

### 本机**真的证明了**（读数见 §4.1）

- 映射逻辑（`export_midi.rs`）与命令行逻辑（`cli.rs`）的**单元判据**：在 `rustc --edition 2024 --test -D warnings` 下真跑，122 + 18 条。
- **完整的导出路径真的被走过**：`midly` 单独编 rlib 之后，`yeban-render` 的 `vlq.rs` + `midi.rs` **原件**在本机参与编译与执行 —— 字节级的 ①②③④ 不是"等 CI"。
- **真进程**语义：探针二进制（与 `main.rs` 无窗口分发同构）跑 `cli_contract.rs` 的 14 条 ⇒ argv / stdout / stderr / 退出码 / 落盘后果都是真读数（含 `--export-midi` 的 3 / 5）。
- 格式、文档门禁、依赖许可清单对账（`run-gates.sh light` 全绿）。

### 本机**没有**证明（只有 CI 能证，逐条说清）

1. **链接后的真二进制**：`cargo build -p yeban-app` 的产物（含 Slint）本机**从未编译**。
   探针与它的差别是"Slint 链接 + GUI 子命令"；本文件与 `cli_contract.rs` 从不走 GUI，
   但"cargo 目标里的类型检查 / 链接顺序 / `build.rs`"只有 CI 能证。
2. **`yeban-app` 新增的 `yeban-render` 依赖边在 Cargo 图里**：本机探针是 `rustc` 手编，
   不经过 Cargo 的 feature / `--extern` 解析 ⇒ "`--locked` 能用这条边解析整个工作区"由 CI 的
   `lockfile` job 与 `rust (yeban-app)` job 判。
3. `cargo clippy --workspace --all-targets --locked -- -D warnings`（含 Slint 目标与
   `.slint` 生成代码）与 `cargo fmt --all --check` 的 **CI 版**。
4. `cargo deny check`（MUST-GATE-004）：本机不装 cargo-deny。
5. `yeban-render` **整 crate** 的 clippy/test（`rayon` / `hound` / `sha2` 本机不编）；
   本机只真跑它的 `vlq.rs` + `midi.rs`。
6. 跨平台：Windows job（`.yeban.lock` 的路径分支）与 Linux 的 GUI 工具链依赖。
7. ⚠ **CI 真的抓到了本机没看见的一条**（run **37254414896**，本线第一次推送）：
   `clippy --workspace` 红在 [`export_midi.rs`](../../crates/yeban-app/src/export_midi.rs) 的
   测试代码 `filter(..).next_back()`（`clippy::filter_next`，`-D clippy::all` ⇒ 错误）。
   **根因是探针自己的**：探针 crate 根当时带着 `#![allow(rust_2018_idioms, clippy::all)]`
   （从 app-cli 线的 harness 抄来的），而**源文件里的 crate 级 `allow` 会盖过命令行的
   `-D clippy::all`** ⇒ 那个"clippy 零告警"是**空转**的，一条 clippy lint 都没跑。
   修法两步：① 探针三个 crate 根去掉 `clippy::all` 的 allow（只留 `rust_2018_idioms`）；
   ② 代码改成 `.rfind(..)`。**验证**（不是声称）：把 `filter(..).next_back()` 故意写回去后
   本机 `clippy.sh` **退出 1** 并打印出与 CI 逐字相同的那条 `clippy::filter_next`；
   改回 `.rfind(..)` 后零告警。这正是"本机绿不是绿、CI 判决才算数"的**具体形态**。

> **纪律复核**：本机跑的是"同构探针"，能证明**逻辑与字节**，**不能**证明"发布的那个产物"。
> 因此 §9 把"CI 判决"记成 pending，判决回来后由集成者按 `docs/CI_CD.md` 的口径填。

---

## 6. 越界项：**工作区内**依赖边（D51 已批准）+ `Cargo.lock` 的**实际 diff**

`--export-midi` 必须调用 `yeban_render::midi`，而 `yeban-app` 此前**没有**这条依赖边。
三条实测（本线在动手前先测的）：

1. 加边之后 `cargo metadata --offline --locked` **直接失败**：
   `error: cannot update the lock file … because --locked was passed`；
2. CI 的 `lockfile` job（`.github/workflows/ci.yml:311`）**无条件**跑 `cargo metadata --locked`；
   所有 `rust (…)` job 也用 `--locked` ⇒ 不动锁就**必红**；
3. `scripts/dev/changed-crates.py:35-43` 把 `Cargo.lock` 列为 `ROOT_TRIGGERS`
   ⇒ 碰它会拉**全量 workspace 门禁**（≈5 分钟 + windows job）—— 这一点如实登记，不是意外。

集成者的裁决（记作 **D51**，由集成者登记进 `docs/adr/ADR-0001-*.md`）：**允许工作区内依赖边 +
机械重生成根清单**（不许引入新外部 crate；生成物必须由工具产出；同时只有一条线能碰根清单）。

`Cargo.lock` 的**实际 diff**（`git diff Cargo.lock` 原文，**只有一行**，无新 `[[package]]` 段）：

```diff
@@ -6186,6 +6186,7 @@ dependencies = [
  "slint-build",
  "yeban-engine",
  "yeban-model",
+ "yeban-render",
  "yeban-ui-mcp",
  "yeban-ui-test-port",
 ]
```

生成方式（**工具产出，绝不手改**）：

```console
$ cargo metadata --offline --format-version 1 >/dev/null      # 只多上面那一行
$ python3 scripts/gates/license_inventory.py                  # 锁哈希列随之更新（1 行）
```

外部依赖包数**不变**（618），没有任何新 package 段落 ⇒ 没有新的许可义务、`deny.toml` 无需改动。
**并发事实**：`theory-wiring` 线同时被授权加 `yeban-mcp → yeban-theory` —— 两条边都是纯增量，
谁后合并谁用上面同一组命令重生成即可（**不要**回退对方的边，也不要用
`git checkout --ours/--theirs` 解决锁冲突 —— 新教训 **L33**）。

---

## 7. 未实现项（如实登记，**不是**静默降级）

1. **`.als` 导出（`ROAD-M4-007`）仍未实现**：本线只把它的**前置能力**接上了出口 ——
   `midi.rs` 不再是零消费者；`.als` 本体（`flate2` Gzip XML + 映射损失表 + 烘音频兜底）
   仍 PENDING，且需要特性门（红线 6）。
2. **控制器 / SysEx / 通道压力 / 弯音 / Program Change 全部不导出**：`midi.rs` 的公开面里
   只有 NoteOn / NoteOff / Tempo / TimeSignature 四类事件；映射层没有第二份编码器，
   因此也不存在"偷偷加一个 CC"的地方。
3. **多轨编排到多 MIDI 轨的策略**：本版 = SMF 1 + 每条含音符的工程轨道一条 `MTrk` +
   通道 `i % 16`。**未实现**：让用户选"单轨合并 / 按通道合并 / 按组导出"；也**未实现**
   "把 >16 轨映射到端口 / bank"。>16 轨时复用通道是**如实**的（`tracks=` 里报总数）。
4. **`probability` / `ratchet` 不导出**（§2.4）：与播放路径的差异已登记；
   若人类要求"导出=听到的"，需要先裁决 RNG 口径（种子进 SMF 吗？）。
5. **不做 `placement` 窗口裁剪**（§2.4）：与 `yeban-engine` 播放路径的 clamp 不同口径；
   与 `yeban-mcp` 的 `yeban_render_master` 同口径。**待裁决**（§8 needs-4）。
6. **`loop_config` 不导出**：循环重复是播放期展开，SMF 里没有对应物（重复段只能靠复制事件表达）。
7. **不导出**：段落 / 场景 / 自动化（含 tempo 自动化）/ 路由 / 混音参数（音量、声相、
   mute/solo）/ `sections[].start_tick`（没有 marker 元事件）/ 调号 / 拍号变更链 /
   `SceneV3::tempo`（工程只有一个顶层 `bpm`）。
8. **`TimeSignature` 元事件的 `cc=24` / `bb=8` 是编码器写死的惯例值**（每四分音符 24 个
   MIDI clock、每四分音符 8 个 32 分音符），不来自工程。
9. **SMPTE 时间码与 SMF 2（Sequential）不支持**：`midi.rs` 的读取面明确拒绝
   （`UnsupportedTimecode` / `UnsupportedFormat(2)`）；写侧恒为 Metrical + SMF 1。
10. **GUI 里没有导出控件**，**十个 MCP 工具里没有 MIDI 导出位** —— 这是 D47 的**有意**结果
    （唯一出口 = app CLI；**不**扩 `yeban_render_master` 的 `format`）。
11. **没有"导出选中轨道 / 时间范围 / 场景"**：只有"当前工程全量"。
12. **本机不编 `yeban-app` 的 bin 目标**（Slint）⇒ 新模块在 Cargo 目标下的编译证据只有 CI（§5）。

---

## 8. needs

1. **三方对齐矩阵需要集成者更新**（`docs/ledger/feature-alignment.md` 本线**禁改**）：
   第 151 行的 MIDI 行现在应为"系统已实现 + UI 无控件 + MCP 无出口，**出口 = app CLI
   `--export-midi`**（D47）"；`phase-status.md` 的 `ROAD-M4-007` 行里"前置能力已在"
   现在应补一句"**出口已接**（app CLI），`.als` 本体仍 PENDING"。
2. **D47 / D51 需要落进 ADR 正文**（`docs/adr/**` 本线**禁改**）：D47 是本次交付的依据，
   D51 是 §6 那条越界边的批准记录，两者目前只在本文件与提交信息里。
3. **Quick Start / README 需要集成者加一行**（`README*.md` 本线禁改）：
   `yeban-app --open song.yeban --export-midi song.mid`（无显示器可跑，退出码 0/3/5）。
4. **待裁决：导出是否应施加 `placement` 窗口裁剪**（§2.4 / §7-5）。两个既有口径不一致
   （engine 播放 clamp / mcp 渲染不 clamp），本线选了后者并如实登记；若要改，须同时改
   两条路径或在文档里写清"导出 ≠ 播放"。
5. **待裁决：`probability` / `ratchet` 是否进 SMF**（§7-4）。若要进，需要先定"种子怎么进文件"。
6. **CI 成本**：本线碰到 `Cargo.lock` ⇒ 触发**全量** workspace 档（含 windows job）。
   这是 D51 第 4 条的已知代价，登记在此以免下次误判为"某处写错了"。
7. **`.als` 线（`ROAD-M4-007`）**：需要 `flate2` 特性门 + 依赖许可裁决（红线 2/6），
   且现在可以直接复用本线的"工程 → `MidiExport` → 原子落盘"骨架里**与格式无关**的那一段
   （映射层 + 报告 + 退出码），不必再发明第二套 CLI 出口。

---

## 9. pending

| # | 事项 | 为什么还没关 |
| :-- | :--- | :--- |
| 1 | **CI 判决**（run id + 逐 job 读数） | 本机绿不是绿（`docs/CI_CD.md`）；判决由 `scripts/dev/ci-verdict.sh line/app-export-midi` 读回后填。**第一次读数已回**：见 §10 |
| 2 | 真二进制（含 Slint）的 `cargo test -p yeban-app --all-targets --locked` | 本机不编 Slint（§5-1） |
| 3 | `cargo clippy -p yeban-app --all-targets --locked -D warnings` | 同上；本机只跑了零 Slint 半边的 clippy |
| 4 | `cargo test -p yeban-render --all-targets --locked`（整 crate） | 本机不编 `rayon` / `hound` / `sha2`；本机只真跑 `vlq.rs` + `midi.rs` |
| 5 | `lockfile` job（`cargo metadata --locked`） | 本机跑过 `cargo metadata --offline`（生成锁）；`--locked` 版由 CI 复核（**37254414896 已 ✓**） |
| 6 | 与 `theory-wiring` 线的锁合并 | 两条线都改 `Cargo.lock`；谁后合并谁重生成（D51） |

---

## 10. CI 判决记录（**只有这里的绿算绿**）

### 10.1 第一次推送：run **37254414896**（tip `d5275c0`）⇒ **failure**

| job | 结论 | 说明 |
| :--- | :--- | :--- |
| `deny (cargo-deny 开源合规)` | ✅ success | 没有新外部依赖 ⇒ 许可策略不变 |
| `lockfile (确定性 Cargo.lock)` | ✅ success | **`cargo metadata --locked` 通过** ⇒ §6 那条内部边与机械重生成的锁**自洽**（这是 D51 要求看到的关键证据） |
| `checks (fmt / 红线守卫 / schema)` | ✅ success | `cargo fmt --all --check` + 14 条守卫 + schema |
| `plan (受影响集合)` | ✅ success | 碰了 `Cargo.lock` ⇒ `workspace_wide = true` |
| `windows (yeban-mcp / yeban-model 的平台分支)` | ✅ success | 与本次改动无关的平台腿 |
| `rust (workspace 全量)` | ❌ **failure** | **`clippy --workspace (-D warnings)`** 红：`clippy::filter_next`（§5-7 的详细根因与修法） |
| `rust (${{ matrix.crate }})` | — skipped | `workspace_wide = true` ⇒ 窄矩阵腿按设计跳过（`.github/workflows/ci.yml:170`） |

### 10.2 第二次推送（修 `clippy::filter_next` + 探针去 allow）⇒ 见提交信息里的 run id

> 本节的判读口径与 `docs/CI_CD.md` 一致：**未读取的判决记为 pending**，
> 不把"本机绿"或"上一个 run 的绿"当成这次的绿。
