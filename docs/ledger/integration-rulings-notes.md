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
| R9 | 状态栏三处说假话（`selection` / `chord` / `device`） | **修**，但**先处理 D24 对照样本** | ✅ 已执行 `3714527`（接线）＋ `a062c38`（基准重录，手动档 run `37874676733`） |
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
| **R18** | `model` 收紧后 `mcp` 的一条判据前提过期（跨 crate 语义冲突） | 判定哪一侧的语义对，然后修前提 | ✅ 已执行 `a5ef1e3`（判定 (甲)：`model` 收紧对 ⇒ 改判据的**前提假设**，保留 `added_once` 全部实质断言）；`docs/adr/**` 未落文件，裁决只在此台账 |
| **R19** | `feature-alignment.md` 第 16 节里的 `path:NN` 指针是否可改（原文一字未删 vs 指针须指向真实行） | 判定：**指针可就地更正**，该节的**叙述文字仍一字不改** | ✅ 已执行 `d8826fc`（判据 7 读的是指针本身；那 4 行两边都不是散文而是 `path:NN`；与 `:86` 行的「就地更正」先例同族）。⚠️ 边界：判据够不到**裸文件名**引用，另有 4 处陈旧的 `tools.rs:NN`（`:103`/`:767-817`/`:582`/`:570-573`）**仍未修**，已登记 |
| **R20** | `crates/yeban-mcp/tests/tools_e2e.rs:1470` 把 `PROJECT_LOCKED` 的**整个错误对象**（含随时间变化的 `heartbeatAgeSecs`）与字面量整体比较 ⇒ 慢机器上间歇红 | 判定：**判据脆弱，非实现缺陷** ⇒ 只放宽时间量，实质断言全留 | ✅ 已执行（见下一条提交）。⚠️ 明说在先：**不动的三条实质判据**＝错误码 `PROJECT_LOCKED`、`advisoryLockHeld == true`、目标文件字节不变；**只改**随时间变化的心跳年龄字段的比较方式 |
| **R21** | 更正 R20：R20 说 `tools_e2e.rs` 用**整对象字面量**比较 `PROJECT_LOCKED` 错误 —— **不准确**。执行票 `833dd91` 读源码证明那是**逐字段**断言（`:1469` 错误码、`:1470` `lockFile`、`:1475` `advisoryLockHeld`、`:1487` 无活动项目、`:1488` 字节不变） | 判定：**保留 R20 的结论**（随时间变化的量不该钉精确值），**改正其机制描述** | ✅ 已执行。⚠️ 诚实边界：CI 那条红（run 37912390354，`52 passed; 1 failed`）是**真的**；`833dd91` 之后 `rust (yeban-mcp)` = success。但**具体是哪一条断言在慢机器上红了，我（集成者）没有逐条验证** —— 日志里 `left` 侧打印的是整个错误对象，被截断在 `heartbeatAgeSecs` 处 |
| **R22** | 新增一个 `kind`（`disconnectRouting`）后，`crates/yeban-mcp/src/domain/notes.rs` 的 `assert_eq!(OP_KINDS.len(), 11)` 未同步（本机实测 `left: 12, right: 11`） | 判定：**同步分母 11 → 12**，判据性质不变 | ✅ 已执行。⚠️ 明说在先：该判据检查的是「`kind` 全集 = 登记真相，且错误信息的 `supportedKinds` 与判据共用同一份真相」——**这条性质没有动**，变的只是被**有意新增**的一个成员。这不是弱化判据 |
| **R23** | 集成者的测量错误：用 `cargo fmt --all --check 2>&1 \| tail -1; echo "(exit=$?)"` 判定格式 —— `$?` 取的是 **`tail`** 的退出码，不是 `cargo fmt` 的 ⇒ 误判格式合规，把一盏红灯（`d51d947` 的 `checks` 格式步骤）送上 CI | 判定：**凡靠退出码判定的检查，绝不把命令放进管道后再取 `$?`** | ✅ 已执行 `c803960`（`style(render): rustfmt the RF64 guard`）⇒ `checks` = success。⭐ 正确写法：`cmd > /tmp/f 2>&1; echo "exit=$?"` 或 `if cmd; then …` |
| **R24** | `dsp-20` 新增的 FFT 判据 `math::tests::the_power_of_two_transform_is_frozen_bit_for_bit` 在 **Windows／x86_64** 上红：`assertion left == right failed: 2 的幂长度的输出位型漂移了`（run 37969761683 / 37969782229，`windows (…)` 作业，`299 passed; 1 failed`） | 判定：**这是跨架构逐位判据，按 ADR-0001:379-383 必须平台感知** —— IEEE 精确类才要求处处逐位相同；**FFT 的旋转因子是超越函数（sin/cos）⇒ 属 4096 ulp 预算类，只在冻结架构 aarch64 要求逐位相同** | ✅ 已执行（见下一条提交）。⚠️ 明说在先：**判据的性质（冻结位型）保留**，**只把它改成平台感知**（同平台硬断言、异平台点名跳过并明确标注**跳过不是通过**），与本会话 `6ce0ec8` 的 L1 修法同族 |
| **R25** | `8f57023`（render-26 抢救）把 `mastering` 的 LRA／绝对门限判据钉成**精确值**：本机（aarch64/macOS）`171 passed; 0 failed`，CI 的 `rust (yeban-render)` 上 `169 passed; 2 failed`（`the_absolute_gate_is_a_strict_lower_bound`、`lra_of_two_levels_is_hand_computable`，断言 `左 == 右` 报「刚过门限的一组同值短时值给出 0.0 LUFS」） | 判定：**这是跨平台精确值断言问题，与 R24 同族**。响度是 `powf`／对数链路，**绝对值在不同平台上不必相同**；判据应断言**性质**（严格不等、单调性、区间包含、门限两侧的方向差异），不得钉跨平台不成立的绝对值 | ✅ 已执行（见下一条提交）。⚠️ **明说在先**：**判据想守的契约不变**（Γa 是严格下界、两个电平静定 LRA 可手算），**只把断言形式从「精确值相等」改为「性质成立」** |
| **R26** | 工作模式：**每条票新建一次性 agent**（本会话前 630 轮的做法）导致每个 agent 必须从零重新推导它所负责模块的知识 —— 7 类缺陷清单在本 crate 的命中情况、文件布局、热点文件（`rf64.rs` / `project.rs` / `meter.rs` 三个文件反复冲突）、既有判据清单与命名、既知脆弱判据。实测代价：**纪律 A（先机械证明前提）之所以成为必需，正是因为新 agent 不知道模块历史**；多票为此先花大量篇幅重建上下文，且有票（`engine-29`）在重建后才发现"要重做的契约其实已存在"。 | 判定：**改为以模块为单位的常驻 agent**。一个模块（`dsp`/`render`/`engine`/`sfz`/`decode`/`theory`/`midi`/`model`/`mcp`）对应一个**长期存活、上下文可复用**的 agent，集成者用继续对话（`send_message`）派新工作项，而不是每条票新建 agent。模块知识（清单、布局、热点、判据命名、脆弱判据）**在 agent 内累积**，跨票复用。 | ✅ 已执行。⚠️ 明说在先：**红线不变**（零分配／`forbid(unsafe_code)`／逐位一致判据），**四道闸门不变**（`check` + `clippy --all-targets` + `test` 非零失败 0 + `fmt`），**零删除门槛不变**，**热点文件用追加法**不变 |
| **R27** | **并行 agent 共用 `/tmp` 会互相污染测量证据。** 本会话按 R26 同时起了 8 个模块常驻 agent，我给的派单里把四道闸门的输出路径统一写成 `/tmp/ck.txt`、`/tmp/cl.txt`、`/tmp/ts.txt`、`/tmp/fm.txt` —— 8 个 agent 同名同径 ⇒ **每个 agent 读到的都可能是别的模块 agent 的输出**。`mod-sfz` 实测：它用 `/tmp/g1.txt` 读四道闸门时，读到的是 `.worktrees/model-cat-5` 的 `clippy` 输出与 `yeban-model` 的测试结果，**不是它自己的**。这是一类**静默的错误证据**：退出码与"测试结果"文本都"看起来正常"，只是属于另一个 crate。 | 判定：**多 agent 并行时，每个 agent 必须用私有目录**（`/tmp/<模块名>/`）；派单模板里一律写私有路径，**⛔ 不再出现裸 `/tmp/<短名>.txt`**。⭐⭐ **并且：任何已用共享路径取得的四道闸门读数一律作废，必须在提交前用私有目录重取** —— 因为无法事后判断某个读数属于谁。已向 8 个模块 agent 广播并要求重取。 | ✅ 已执行（广播 8 个 agent）。**未采纳**"反正 git 里有真实代码、读数错了也能靠 CI 抓"这一说法：本会话的判定来源就是"读回的判决"与"真实运行结果"，**读数不可信即证据链断裂**；且本仓库已两次出现"`cargo test` 绿而实际是假的"（陈旧调用点、嵌套 `#[test]`），正是靠读数的正确归属抓到的 |
| **R29** | **`/tmp` 的子目录同样是共享的。** R27 只点了裸文件名，实测 **`/tmp/inj` 也被别的 agent 占用**：`mod-midi` 在其中发现 `mastering.rs.pristine`／`inj.py`／`table.md`／`results_pass1.jsonl`（它自己没写过这些），前 3 条注入读数因此作废。⇒ 只把**输出文件**私有化不够，**注入脚本与 pristine／原始副本也必须私有**。 | 判定：**每个模块 agent 的注入工作区一律放 `/tmp/<模块>/inj/`**（脚本、pristine 副本、结果 jsonl、表格全在其中）。派单模板里⛔ 不再出现任何共享的 `/tmp/<短名>` 路径。 | ✅ 已执行（广播 8 个 agent） |
| **R33** | **`mod-decode` 待裁决 C：`slurp_unseekable` 的 `Interrupted` 重试分支可达但**无上限****（`decode.rs:864`）。注入 `D10` 已证明该分支可达 ⇒ ⚠️ **一个永远返回 `Interrupted` 的源会让这个循环永不返回**，即**挂死**。这是本批唯一一条**资源与可用性**缺陷，不是判据问题。 | 判定：⭐ **属缺陷，应予修复** —— 给重试加上限（⭐ 并且把**已消耗的字节计入既有输入字节预算**，这样上限有物理意义而不只是一个魔数）。⚠️ **明说在先**：修复会改变「永远 `Interrupted`」这一情形的返回（挂死 → 明确 `Err`），⭐ **对正常源零影响**；需补一条判据证明「连续 N 次 `Interrupted` 之后返回明确 `Err`」且**不依赖墙钟**（⭐ 用可数上限，⛔ 不用超时判据）。 | ⏳ 待 `mod-decode` 执行（⭐ 排在最前） |
| **R35** | **`mod-mcp` 修掉了 MCP 工具面的最大缺口：设备 CRUD 此前不可达。** 普查后 `InsertDevice`／`RemoveDevice` 两个 `Op` 变体的构造点口径为 **0**（只有 2 处文档行文提及）⇒ **工具面能看见设备链（`yeban_query_engine_state`）却一台都动不了**。⭐ 而 `DeviceDefinition::latency_samples` 是 **PDC 的唯一延迟来源**（`ARCH-PDC-001`）⇒ 缺口直接卡住延迟补偿这一功能面。 | 判定：⭐ **接线**。`yeban_edit_notes` 的 `ops[]` 新增 `insertDevice`／`removeDevice` 两个 kind（`trackId` 寻址；`slotIndex` 可省＝追加链尾；取走按 `deviceId` 身份）。`OP_KINDS` 17 → **19**（同步判据改字面值 17 → 19）；新增判据 10 条（8 模块级 ＋ 2 端到端），`cargo test` 639 → **649 passed**。⭐⭐ **结果：31 个 `Op` 变体的构造点口径全部 ≥ 1，0 个仍为 0。** | ⏳ 待 `mod-mcp` 提交（⭐ 四道闸门本机已全 0） |
| **R36** | **形态 D 的还原基线是 `HEAD`，所以驱动器必须先把新判据提交再跑复跑证明。** 本会话的还原纪律是 `git checkout HEAD -- <文件>`（R30 后加 `git clean` 与 sha256 自证）。`mod-model` 第五轮实测出它的副作用：若先把新判据写进工作区、再跑「注入 ⇒ 该判据变红」的证明，则驱动器每注入一次就 `checkout HEAD` 一次，未提交的新判据在第一次注入后即被抹掉，复跑读成「新判据没变红」，等于把工作区状态当基线。同族两坑：① **字面歧义** —— 同一片段在文件里出现两次以上时，注入打在另一处，读数既不是 MISS 也不是真红，故 `出现次数 != 1` 必须记 AMBIG 并在第二轮消歧（`mod-model` 2 次、`mod-midi` 1 次被 manifest 拦下）；② **既有判据的名字会骗人** —— `default_snapshot_hash_is_deterministic_and_content_sensitive` 只探不同长度，不探同长度不同内容，故名字里声称的每个维度都要单独证一遍。 | 判定：驱动器固定为「先 commit 新判据，再跑注入与复跑证明」；`出现次数 != 1` 记 AMBIG（不记 MISS）；判据名声称的维度逐维验证。 | 已执行（`mod-model` 用 `fe31d74` 先提交再复跑；它靠这条找出 `default_snapshot_hash` 的同长度盲区） |
| **R37** | **`mod-render` 请裁决一处产线修复：`mastering.rs::within_measurement_resolution` 在目标取 `Some(f32::MAX)` 时把分辨率算成 `+inf`。** 探针读数（f32 位型）：`next_up(f32::MAX) = inf`、`resolution = inf`、`residual = 3.4028234663852886e38`、`residual <= resolution = True` ⇒ **任何残差都被判成「落在测量分辨率里」** ⇒ 返回 `gain_db = 0.0 / bound = LoudnessTarget` 而**样本一位未动**。这与该模块自己写的「`NormalizeOutcome` 永远描述一件真事」矛盾，也与既有的非有限目标判据同属一条契约。修法是 1 行 `resolution.is_finite() && residual.abs() <= resolution`（+ 文档 2 行，故该提交删除行 2）。⚠️ 派单写「提交只含新增判据」，但不加这一行，新判据在真代码上就是红的。 | 判定：**接受该产线修复**。理由：① 它是**契约内部矛盾**（返回值声称达成而样本未动），不是判据为了变绿而放宽；② 修法只加一个有限性合取项，对正常目标零影响（`f32::MAX` 之外的 `next_up` 都有限）；③ 同一契约在**非有限目标**上已有判据，本条只是把**有限但顶到量程顶端**那一格补齐；④ 判据与修复在同一提交、且判据已被证明在修复前为红。 | 已执行（`469cedd`，四闸门 0/0/0/0，225 passed）。⏳ 待读回 CI 判决 |
| **R34** | **注入驱动器被并发启动两次 ⇒ 整轮读数作废，且污染方式极其隐蔽。** `mod-mcp` 先用 `nohup … &` 起了一次驱动器（工具报 job finished，但 python 仍活着），随后又用 `run_in_background` 起了第二次 ⇒ 两个进程注入同一棵树：A 的 `restore_all()` 在 B 正注入时用 pristine 覆盖 B 的文件，实测 **13 个文件被截成 0 字节**。B 跑的是混血／截断的树，于是**整轮 results.jsonl 全部作废**（已删掉重跑）。⚠️ 这一条比 R27／R29／R30 更危险：前三条是「读到别人的正确读数」，这一条是「读到一棵不属于任何提交的树」。 | 判定：**注入驱动器必须单实例**（锁文件 `/tmp/<模块>/inj/driver.lock` 已存在即拒绝启动，`exit=4`）；⭐ **只用工具的 `run_in_background`，⛔ 不用 `nohup … &`**。⭐⭐ **可复用的机械识别法**：还原核对里出现空串 sha256 `e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855` ⇒ **该轮被并发污染，读数一律作废**。 | ⏳ 待广播模块 agent（⭐ 本条为新增，尚未发出） |
| **R32** | **`mod-decode` 待裁决 B：`PcmBudget::for_layout` 的 `max_resample_ratio` 恒为 `DEFAULT_MAX_RESAMPLE_RATIO`（1000×），与会话要求无关 ⇒ 它返回的预算不能转换「它自己采样率包络内的任意采样率对」**（`for_layout(1, 768000, 1)` 拒绝 1 Hz → 768 kHz，而两端都过采样率闸门）。 | 判定：⭐ **不改数值**（该上限是**全局安全帽**，按调用方的包络放宽它会削弱资源保护）；⭐ **改为在 `for_layout` 的文档注释里写明「`max_resample_ratio` 与会话要求无关，恒为全局默认」**，并**补一条判据钉住这一事实**（避免将来有人误以为它随要求推导）。 | ⏳ 待 `mod-decode` 执行（⭐ 文档 ＋ 判据） |
| **R31** | **`mod-decode` 待裁决 A：`resample_asset_with_budget` 的恒等路径不判第六道闸门（比例闸门）⇒ `max_resample_ratio = 0` 时两个入口结论相反**（样本级入口 `ResampleRatioTooHigh`、资产级入口 `Ok(原资产)`；其余五道闸门的错误文本两边逐字相同）。两条同样输入、同样预算的公开入口给出相反结论，是**契约不一致**，不是设计选择。 | 判定：⭐ **属缺陷，应予修复** —— 资产级恒等路径必须与样本级入口判同一组闸门。⚠️ **明说在先**：这会改变 `resample_asset_with_budget` 在 `max_resample_ratio = 0` 时的返回值（`Ok` → `Err`），因此**需要一条判据把两个入口的结论钉成相同**，并登记为本裁决的验收条件。⛔ 不许只改一处而让另一处保持旧结论。 | ⏳ 待 `mod-decode` 执行（⭐ 排在它下一个工作项） |
| **R30** | **`proptest` 失败会写出 `crates/<crate>/proptest-regressions/`（回归种子），注入扫查若只 `git checkout HEAD -- <文件>` 而不清它，种子会跨注入留存，污染其后**每一次**读数。** `mod-decode` 实测：本批第一条注入就因此中止过一次。这与 R27／R29 同族：**测量环境的残留物伪装成测量结果**。 | 判定：**每次注入后的还原固定为三件套** —— `git checkout HEAD -- <文件>` ＋ `git clean -qfd crates/<crate>` ＋ `git status --short` 必须为空；⭐ **`proptest-regressions/` 不得进提交**。⭐ 已广播模块 agent。 | ✅ 已执行 |
| **R28** | **`cargo test` 默认在第一个失败的目标就停，导致形态 D 注入扫查的「红判据数」被系统性截断。** 本会话的注入扫查一律用 `cargo test --lib --tests`，**未传 `--no-fail-fast`** ⇒ 第一个目标失败后其余目标根本不跑。`mod-midi` 实测：头三条注入读数因此只跑完 `--lib` 一个目标（四个目标里），「只有 N 条判据变红」是**假的** —— 把「没测到」当成了「判据没问题」。这是一类**方向性错误的度量**（它让判据看起来比实际更强）。 | 判定：**所有注入扫查与四道闸门里的 `test` 一律加 `--no-fail-fast`**；⭐ **任何不带它的注入读数一律作废、必须重跑**。`mod-midi` 已自行作废并重跑头三条；已广播 8 个模块 agent。 | ✅ 已执行。⚠️ 这是本会话第三条「看起来正常但归属／范围错误」的读数（前两条是 R27 的 `/tmp` 共享、以及早先「`cargo test` 绿而实际是假的」） |

## 0.2 流程教训（本台账自身的）

1. ⚠️ **R11c 曾只存在于集成者的叙述里，从未写进本台账** ✗ —— 执行票在报告里点名核出（`grep -n 'R11'` 只有 `:24` 一行）。⇒ ⭐ **教训：凡在指令里给某条工作起了编号，必须当场写进台账；否则执行方无法复核出处，且编号会漂移**。
2. ⭐ **执行票独立复核缺陷本身成立，但出处不成立时，要同时报出这两件事**（R15 的票做到了：缺陷成立 ✓、出处不成立 ✗）。
3. ⭐ **"没有判据"这种断言必须全仓 grep**，不能只搜 `tests/` —— R11 曾因此漏判（判据实际在 `crates/yeban-app/src/test_port_adapter.rs`）。
