# 集成者裁决台账（integration rulings）

**台账类型**：裁决记录 / 依据 / 执行状态。**基线**：`main` 的 `4f6e2eb`。
**授权**：负责人指示「**这些裁决都你自己择优选择**」（本会话，2026-10-08）。

⚠️ **本文件记录裁决与依据，不复制别处的读数。** 每条的"依据"给出可复核的出处。

---

## 0. 裁决总表

| # | 事项 | 裁决 | 状态 |
| :-: | :--- | :--- | :--- |
| R1 | MusicXML 是否引入新依赖 | **不引入**。先做纯 `.musicxml` 只读 MVP，手写极简 pull parser | ✅ 已执行 `d74d3ac` |
| R2 | 复音数 16 / 失谐上界 ±4800 音分（规范无数字） | **维持现值**，登记为工程选择 | 已裁决 |
| R3 | `synth_filter.rs` F1 缺第三个臂 | **补臂**（加强判据，不弱化） | ✅ 已执行 `c9717a8` |
| R4 | `docs/**` 三处补记（polysynth 落地等） | **补记** | ✅ 已执行 `bbad010` |
| R5 | MIDI tempo map 往返不保真 | **修**（真缺陷；修后把"字面读数"判据改成断言） | ✅ 已执行 `0e3c990` |
| R6 | 17 处源码行号漂移 | **清**（机械修正，不改语义） | ✅ 已执行 `f1ec03b` ＋ `bbad010` |
| R7 | `engine-mix-notes.md:52/464` 的 `BusLimiter::apply` | **改为 `process_stereo`** | ✅ 已执行 `f1ec03b` |
| R8 | "真峰值限制器"口径缺口（实现按样本峰值） | **登记，暂不实现**（真峰值需 4× 过采样 ＋ 前瞻缓冲，出处 `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md:396` 的 §3.4；属独立器件票） | 已裁决 |
| R9 | 状态栏三处说假话（`selection` / `chord` / `device`） | **修**，但**先处理 D24 对照样本** | 待开票 |
| R10 | `branch-name` 双写者无判据 | **登记**，低优先级 | 已裁决 |
| R11 | 录音键 `accessible-checkable: true` 恒假 | **修**（诚实性小修） | 待开票 |
| R12 | `[ROAD-M2-001]` 实时线程优先级需新依赖 `libc` | **延后**（属新依赖；与"不加远期依赖"同口径） | 已裁决 |
| R13 | 节拍器无界面开关 | **延后**（必然改像素 ⇒ 需一次基准重录；另立票） | 已裁决 |
| R14 | `drums` 的 808/909"物理建模"范围 | **MVP ＋ 未实现清单**（⛔ 不许读成"建模完成"） | 进行中 |

---

## 1. 逐条依据

### R1 MusicXML：不引入依赖
- 事实：本仓库**零** XML 依赖；`.mxl` 被 `yeban_model::container::read_container` **明确拒绝**（`UnsupportedCompression { method: 8 }`，6/6 实测）。
- 事实：`.mxl` 的 ZIP 读取器**不能复用**（`read_zip` 是 `pub(crate)`；`zip.rs:13` 明写"本线裁决：不引入 `flate2`"）。
- 裁决理由：我们的需求是一个**白名单子集**（`note` / `pitch` / `duration` / `chord` / `tie` / `backup` / `divisions` / `time`）。
  手写 pull parser 可覆盖它，且**不引入新依赖**，与"不加远期依赖"同口径。`.mxl`（inflate）**另立票**。
- 可复核：`docs/ledger/feature-alignment.md` 的 MIDI 相关行；本会话 `grep -rn 'quick-xml|roxmltree|xml-rs' crates/*/Cargo.toml` ⇒ 无匹配。

### R2 复音数与失谐：维持现值
- 事实：规范**没有**给出复音数与失谐范围的数字；规范只写「16 个 PolySynth 减法合成器」（= 16 个**实例**/轨，不是每实例 16 声部）。
- 裁决理由：**既有实现**（上移前 `synth.rs`）用 16；改动它会改变听感与既有判据的期望值 ✗。⇒ **维持**，并在此**登记为工程选择**。

### R3 `synth_filter.rs` F1 补第三个臂
- 事实（转自 `polysynth` 票的注入 I7）：F1 的两臂是「旁通 vs 默认（**也是旁通**）」与「旁通 vs 300 Hz」⇒ 「旁通 = 20 kHz 透明滤波」这种错法在 F1 下**不可见**。
- 裁决理由：**加强**判据不属"调整红线" ✗。「旁通 ⇒ 逐位恒等」的**对照臂**必须在语义上不同于旁通。

### R5 tempo map 往返：修（已执行，`0e3c990`）
- 事实（⚠️ **措辞已按执行票的实测更正**）：`to_smf_bytes` 把 tempo 排 `rank 2`、拍号排 `rank 3`；`parse_smf` 把拍号挂到「同 tick 的**最后一条** tempo」⇒ **配对换人**；且 `parse_smf` 为**孤儿拍号**合成 `500000` µs 并**写进文件**。
  ⚠️ **更正**：本台账初稿写"触发条件是同 tick ≥2 条 tempo"✗ —— 执行票实测**源文件里同 tick ≥2 条 TEMPO = 0/16**，那不是源文件的形状。**真正的触发条件是「孤儿拍号」**（该 tick 找不到可配对的前驱 tempo），命中 **9/16**；往返多重集不相等 **7/16**。
- 裁决理由：这是**功能性错误**（工程存→读→存会漂移），不是风格取舍。⇒ **修**。
- 执行结果：`MidiTempo::microseconds_per_quarter` 由 `u32` 改为 `Option<u32>`（`None` = 只带拍号）；配对按出现顺序且只看紧邻前一条；写出按内容分组。往返多重集不相等 **7/16 ⇒ 0/16**。
- ⚠️ 影响面（已实测）：`yeban_export_midi` 的**工程路径字节不变**（`export_from_project(demo_project())` 旧 133 / 新 133，逐字节相等）；真夹具再导出 **7316 → 7309 字节**（差 −7 = 一条被删掉的合成 Tempo 元事件）。
- ⚠️ 本条的判据改动属"字面读数判据调整"，**已按 objective 要求明说并记录**：理由是该判据原本**钉住缺陷现状**（注释写明"不是承诺""修好后请改成 `assert_eq!`"），改动是**修缺陷**而非放宽；另有 3 处语义读数更新（原期望值含凭空合成的 `500000`，该值在源文件里**不存在**）。

### R8 "真峰值"口径：登记，暂不实现
- 事实：`yeban-dsp/Cargo.toml:3` 与 `roadmap:107` 写「真峰值」，而实现按**样本峰值**工作（原 `mixer.rs:174-179` 自陈）。
- 裁决理由：真峰值需要 **4× 过采样 ＋ BS.1770 滤波**，属**独立器件**规模 ✗。⇒ **登记**，并在文档里保留"按样本峰值"的事实措辞。⛔ 不许把现状写成"真峰值已实现"。
- ⭐ **出处（2026-10-08 补）**：这句话的**唯一**规范落点是 `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md:396`（**§3.4 内部插件延迟补偿架构 (PDC)** 一节的正文），逐字为"为确保真峰值限制器（BS.1770-4 规范需要 4× 过采样滤波与前瞻 Lookahead 缓冲）…"。
- ⚠️ **出处更正（2026-10-08）**：集成者的指令曾把这句归给 `[ARCH-FMT-001]` ✗ —— `ARCH-FMT-001` 落在 `docs/YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md:459` 的 **§5.2 广播级 RF64 / BW64 自研写入器**（`:459-462` 讲 RF64/BW64 ＋ `bext` ＋ TPDF 抖动），**不含**过采样或前瞻缓冲。复核量法：`grep -rn --exclude-dir=.git -E '真峰值|过采样|oversampl' . | grep 'ARCH-FMT'` ⇒ **0 命中** ⇒ 没有任何**已提交文件**把这句话归给 `ARCH-FMT-*`；该归错**只存在于指令里，未落盘**，因此本台账只登记出处与这次更正。

### R12 `libc` 依赖：延后
- 事实：cpal 0.18 的 `realtime` feature 只覆盖 WASAPI/AAudio/PipeWire/JACK；macOS 与 Linux-ALSA 要自研 `pthread_setschedparam` ⇒ **新依赖 `libc`** ＋ 平台 `unsafe` 审计。
- 裁决理由：objective 明写「**不加 tokenizer 等远期依赖**」⇒ 同口径延后。⛔ 且它会引入 `unsafe`，与 `forbid(unsafe_code)` 的现行姿态冲突 ⇒ 必须**先明说并记录**，不是本批工作。

### R13 节拍器界面开关：延后
- 事实：`metronome_enabled` 是**工程文件字段**；界面 `grep` 零命中 ⇒ 无开关。
- 事实：加开关**必然改默认帧像素** ⇒ 需一次基准重录（`gates-manual.yml` 的 `gate=goldens` 手动档）。
- 裁决理由：功能**已经能通过工程文件开启**（引擎侧已接线并 CI 绿）⇒ 界面开关是**可用性改进**，属另立票；本轮不做像素改动。

---

## 2. 红线声明（本批不变）

⛔ **本台账不调整、不放宽任何红线**：
- **MUST-GATE-001**：音频线程零分配 / 零释放 / 零锁 / 零阻塞 I/O / 零日志。
- `#![forbid(unsafe_code)]`：库目标。集成测试 crate 的 `unsafe impl GlobalAlloc` 沿用既有 ≥5 处先例。
- **逐位一致判据**：⭐ **R5 会改一条"字面读数"判据** —— 那是因为**缺陷被修**，不是放宽判据；改动的理由与前后读数必须写进那一票的报告。
- **基准图**：本批**不做**像素重录（R13 明确延后）。

---

## 3. 边界（本台账**不**主张的事）

- ⚠️ **此前的陈述有误（2026-10-08 就地改正）**：原句写「`compressor` / `channel_strip` / `limiter` 之外的器件**未接线**」✗ —— 它把 `compressor` 与 `channel_strip` 都读成"已接线"。**实测**：**接线前**两者在整个 `crates/yeban-engine/**` 里**没有任何调用点**，只出现在注释里（`crates/yeban-engine/src/mixer.rs:92-93`、`crates/yeban-engine/src/rt.rs:1007`；后者在该文件里现已下移到 `:1138`）。
  **量法（可复跑）**：`grep -rn 'use yeban_dsp' crates/yeban-engine/src`，**只数真实 `use` / `pub use` 行**命中的模块名（⛔ 注释行不算；本次读数：16 行命中里 3 行是注释 ⇒ **13** 行真实引用）。
  **已接线（8 个模块名）**：`compressor`（`crates/yeban-engine/src/insert.rs:93`，由 `c792fdc` 接进 `rt.rs` 的逐轨插入）、`limiter`（`crates/yeban-engine/src/mixer.rs:111`）、`meter`（`crates/yeban-engine/src/level.rs:34`）、`polysynth`（`crates/yeban-engine/src/synth.rs:98` ＋ `:108`；判据 `crates/yeban-engine/tests/synth_rt_zero_alloc.rs` 一直在量化它）、`envelope` / `filter` / `math` / `oscillator`（`crates/yeban-engine/src/synth.rs:94-97` ＋ `crates/yeban-engine/src/snapshot.rs:80`）。
  **仍未接线（1 个模块名，2026-10-09 就地改正）**：只剩 `drums` —— 量法 `grep -rn '\bdrums\b' crates/yeban-engine` ⇒ **只命中 3 行注释**（`crates/yeban-engine/src/insert.rs:283`、`crates/yeban-engine/tests/channel_strip_insert.rs:35`、`crates/yeban-engine/tests/reverb_insert.rs:37`），没有任何调用点。⚠️ **原句（2026-10-08 读数）写"3 个模块名 = `channel_strip` / `reverb` / `drums`"，已陈旧**：`channel_strip` 由 `db1850f` 接线（再导出 `crates/yeban-engine/src/insert.rs:252` ＋ `:269`；实时侧消费点 `crates/yeban-engine/src/rt.rs:1080` 与 `:1229-1232`）、`reverb` 由 `c4e9f01` 接线（再导出 `crates/yeban-engine/src/insert.rs:273` ＋ `:275`；实时侧消费点 `crates/yeban-engine/src/rt.rs:1121` 与 `:1262-1277`）。随之修订上文「13 行真实引用」这个读数：**2026-10-09 复测 = 15 行**（同一量法 `grep -rn 'use yeban_dsp' crates/yeban-engine/src` ＝ **23 行命中**，其中 **4** 行是注释（`crates/yeban-engine/src/mixer.rs:70`、`:71`、`crates/yeban-engine/src/insert.rs:14`、`:869`）＋ **4** 行是断言字符串（`source.contains(...)`：`crates/yeban-engine/src/mixer.rs:360`、`crates/yeban-engine/src/insert.rs:864`、`:896`、`:1049`）⇒ 23 − 4 − 4 = **15** 行真实 `use` / `pub use`；增量 = `insert.rs` 的 5 条再导出 `:252`/`:263`/`:269`/`:273`/`:275`；原句的 13 行读数 = 16 命中 − 3 注释，双方都未把断言字符串算进去）。⚠️ 本行原句引的 `crates/yeban-engine/tests/limiter_contract.rs:40` 与 `crates/yeban-engine/tests/compressor_insert.rs:121` 在 `crates/yeban-engine/src` 之外、不在该量法射程内，故不再作为该量法的读数；⚠️ 上文「已接线（8 个模块名）」这一计数我这一票**不动**（它不在被点名的两处之内，且其口径归属另一行；按现量法的 15 行读出来是 **11 个不同名**：`channel_strip` / `compressor` / `envelope` / `filter` / `limiter` / `math` / `meter` / `oscillator` / `polysynth` / `reverb` / `shaping`，其中 `channel_strip` / `math` / `polysynth` 各占 2 行 ⇒ 2+2+2+8×1 = 15）。
- 不主张 `drums` 实现了 808/909 的物理建模（见 R14）。
- 不主张 MusicXML 已可导入（见 R1）。
- 不主张跨架构对账（`ARCH-DET-002`）已完成（本机只有一条工具链）。

---

## 0.1 追加裁决（表 0 之后新增；未并入表 0 的行）

| # | 事项 | 裁决 | 状态 |
| :-: | :--- | :--- | :--- |
| R4b | `feature-alignment.md:163` 含一条假陈述（"`DeviceRack` 在 `ui/**` 里没有被任何组件实例化"）**——根因是那条 `grep` 只扫 `ui/` 一层，漏掉 `ui/console/`** | **修**（活表 ⇒ 就地改正；原句保留、依据从"不存在"换成 `visible: active-tab == 2`） | ✅ 已执行 `677fa3f` |
| R6c | 判据 7 够不到的陈旧行号（`phase-status.md:78` 的 `synth.rs:69` · `dsp-core-provenance.md` §1/§2.1 的模块数与行数 · 以及那 1 处漂移 `feature-alignment.md:143`） | **清**（活表就地改正；日期化记录加注）⇒ **漂移 1 → 0** | ✅ 已执行 `677fa3f` |
| R6d | `phase-status.md:108` 手抄的旧读数（`已完成 18 / 部分 23 / PENDING 6`；守卫今天算 `19 / 22 / 6`） | **加带日期的更正注，不改原句**（它引用一次过去的机械读数 ⇒ 就地改写会篡改日期化引文） | 待开票 |
| R11b | 录音键保留的 `accessible-checked: false`（可证明惰性） | **登记**；真接录音时连同 `test_port_adapter.rs:2905-2915` 一起改成真实状态源 | 已登记 |
| R11c | 另两处假 `accessible-checkable` 声明（`device_rack.slint` 旁通开关 · `app.slint` 声学诊断） | **修**（删声明；两处的 `accessible-checked` 一并删，因为没有判据读它们） | ✅ 已执行 `7de4f9f` ＋ `042c3a5` |
| R15 | 编曲视图静音/独奏按钮点了没反应（`arrangement_view.slint` 的两个 `TouchArea` 无 `clicked`） | **修**（只转发到既有宿主面，不发明状态） | ✅ 已执行 `d3c954f` |
| R16 | `piano_roll.slint` 的工具按钮没有 `TouchArea`、也没有工具选择回调（`active-tool` 只由键盘写） | **修需新宿主回调 ⇒ `src/**` 票** | ✅ 已执行 `1114f05` |
| R17 | 复用宿主面带来的标签错位：`host.rs:1649`/`:1657` 硬写 `"mixer: toggle track mute/solo"`，该消息成为撤销树节点标签 ⇒ 在编曲视图点击后标签仍写 "mixer:" | **修**（改中性文本，或给编曲视图自己的宿主回调） | ✅ 已执行 `1114f05` |

## 0.2 流程教训（本台账自身的）

1. ⚠️ **R11c 曾只存在于集成者的叙述里，从未写进本台账** ✗ —— 执行票在报告里点名核出（`grep -n 'R11'` 只有 `:24` 一行）。⇒ ⭐ **教训：凡在指令里给某条工作起了编号，必须当场写进台账；否则执行方无法复核出处，且编号会漂移**。
2. ⭐ **执行票独立复核缺陷本身成立，但出处不成立时，要同时报出这两件事**（R15 的票做到了：缺陷成立 ✓、出处不成立 ✗）。
3. ⭐ **"没有判据"这种断言必须全仓 grep**，不能只搜 `tests/` —— R11 曾因此漏判（判据实际在 `crates/yeban-app/src/test_port_adapter.rs`）。
