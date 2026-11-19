# line/sfz-octave-override 进度

（上一线 line/sfz-file-size-gate 的进度原文在 git 历史 9d238f09c 的本文件中。）

## 审计结论（2026-11-19，基 9d238f09c）

设计原文：`docs/ledger/sfz-core-notes.md` §14.4（阶段 2 设计）。

### 已在位（勿重做）

- (a) 文件声明：parser `note_offset`/`octave_offset` 归约侧施加（阶段 1，91ce3c7a5）；
  `Instrument::declared_note_shifts()` 供回执；`Region::declared_note_shift`。
- (b) 自动检测：`yeban-sfz::notation::detect_notation_shift` 纯函数（门槛 |E|≥5、
  占比 ≥60%、一致 ≥90% 且对向为 0、候选仅 ±12）；S1 切片解析入参
  `parse_text_with_external_shift` / `parse_sources_with_external_shift` 已在位。
- (c) 模型本体：`SamplerBinding.note_octave_override: Option<i32>`（半音八度档
  −24/−12/0/12/24；`None` 与 `Some(0)` 语义不同；D43 已登记
  `#.tracks.*.sampler.note_octave_override`），`effective_octave_override()` 应用侧
  防御（非法值退回检测）。GUI 入口已在位：host.rs `next_octave_override` 步进 +
  混音台徽标/覆盖范围显示（731872cf6、12d2e0724，2026-11-17）。
- 装载编排：`sfz_load::load_sfz_adapted(path, probe, hz, override_shift)`；
  合成顺序 (c)＞(b)，(a) 在归约侧相加；`SfzLoadReport::notation_lines()` 响亮行
  已写好（(a)/(b) 命中/(c) 生效/疑似未命中四种）。
- 既有判据：`crates/yeban-app/tests/sfz_notation_adaptation.rs` 三条（Yamaha 命中、
  SSO 式散列根音不命中＋(c) 修复、普通库逐位不变）。

### 真缺口（本线要补）

1. **CLI 无入口设覆盖**：ops 的 `setTrackSampler` 只收字符串路径，解析处
  （`yeban-mcp/src/domain/notes.rs` SET_TRACK_SAMPLER 分支）硬编码
  `note_octave_override: None` ⇒ 经 `--apply-ops` 绑采样器必然清掉已有覆盖，
  且无任何办法写入覆盖。CLI 用户对 SSO 类库无自救手段（问题文件第 3 节）。
2. **CLI 看不见检测结论**：`cli.rs` 的 `--sfz` 块只 push `report.summary()`，
  不 push `notation_lines()`；`--export-wav` 的 `exported_wav_line` 与
  `render_core::sfz_decode_label` 的 `sfz(...)` 段不含记法信息；
  `SfzDecodeReport` 有 notation 字段但无 `notation_lines()`。
3. **未命中音符完全静默**（任务 B.3）：渲染/导出时音符打不中任何 region 无统计
  无告警。落点已定：render_core 在 `arm_pending_sfz` 后，用
  `PreparedRegion::accepts(pitch, velocity)`（引擎自身静态接受谓词）对每条
  sampler 轨的工程音符（非静音摆放、clip 内容为 notes 者）计数，结果进
  `SfzDecodeReport` 新字段，CLI 导出行外另起响亮行（不改 `sfz(...)` 标签
  形态——该形态有逐字同形判据钉住）。

### 任务 A 落法

- A1 覆盖本体：已在位（见上），本线补判据中未覆盖者：持久化（存工程→重开→
  仍生效，走 CLI 全链）、坏值响亮拒绝（op 写入侧）。优先级口径：合成公式
  总移位 = (a) + 外部，外部 = (c) 设置时取 (c)、否则 (b)——(c) 胜出指外部层
  压过 (b)，(a) 恒相加（设计原义）；判据按此钉。
- A2 CLI 入口：扩展 `setTrackSampler` 的 `sampler` 值形态：字符串（现状不变）
  或对象 `{"path": "...", "noteOctaveOverride": 12|null}`；未知键响亮拒绝；
  非八度档值响亮拒绝。可见性：cli.rs `--sfz` 块补 notation_lines；
  `SfzDecodeReport::notation_lines()` 新增（与 Load 报告共用文案构造）；
  `--export-wav` 块在 exported_wav_line 后逐份 push 解码报告的 notation 行。
- A3 判据：新夹具 = SSO 实体形状（音名键位 c3/c#3/d3＋WAV **无 smpl 块**，
  证据集为空）：无覆盖导出 60/61/62 静音（RMS=0）＋未命中告警出现；
  经 apply-ops 设 +12 覆盖后出声且与数字键位副本逐样本一致。

## 待办

- [ ] 查 SSO 实体是否在本地（find 后台跑）；无则用夹具并在回执写明。
- [ ] B.1 parse_note/as_note 全调用点复核（对照 §14.1 A4 的 11 个）。
- [ ] B.2 文件名根音证据：鼓组审计语料 30 套统计「音名键位＋无 smpl」清单。
- [ ] B.4 其他音名表面复核（含新增 builtin_sounds.rs 目录条目）。
- [ ] B.5 门槛边界结论登记。
- [ ] 实现 A2＋B.3，写判据（红→绿留证），盒上（yanshi-2，等 UI 验证构建结束）验证。

## 补充输入（2026-11-19 11:00 父 agent 转达 Wang 完整版报告 drum-octave-issue-2.md）
- 增量 1：模型字段既存——审计已确认，本线是接通既有字段（解析/编译两丢弃点），未另造存储。
- 增量 2：主验收改真实文件（~/workspace/yeban-samples/）；⚠ 本机实测该目录不存在（全盘 find 特征文件名 snare-roll.wav/bass_drum-f.wav/Sonatina 无果），已挂起等父 agent 确认投放位置；合成夹具判据照常推进，真实文件验收待文件到位后在盒上跑（原版 vs fixed 副本逐样本比对＋Timpani＋one_shot 尾段核查）。
- 增量 3①【必做，已落地】：零证据独立结论——yeban-sfz notation.rs 新增 NotationMissReason::NoEvidence（E=∅ 且音名拼写 region ≥1 时）＋ named_key_region_count 字段＋ machine_name()；sfz_load notation_report_lines 在 None 来源下对 NoEvidence 产独立响亮行（含 notation_shift=0 reason=no_evidence evidence=0/N 与覆盖出路）。key→pitch_keycenter 拼写出处传递已核（instrument.rs:2800，含 group 继承与既有测试）。
- 增量 3②：one_shot＋音名键位加载警告——与 ① 合并：① 的行在装载期对整个受影响形态触发（one_shot 打击乐 kit 是其子集），另加 one_shot 专项行会对同一批文件双重报告；B.3 未命中行覆盖渲染期症状面。取舍写入回执。
- 增量 4：one_shot 掐尾附带核查（只报告不修）——待真实 kit 到位后用 snare-roll 长样本短/长音符双渲比对尾段 RMS，与 2026-11-18 结论对账，数字入回执单节。

## 实现进度（截至 11:0x）
- sfz_load.rs：notation_lines 文案抽共享 notation_report_lines；SfzDecodeReport 补 declared_shifts＋notation_lines()＋unmatched 字段与 unmatched_notes_line()；构造点回填。
- render_core.rs：arm_pending_sfz 后逐轨 count_unmatched_notes（引擎 accepts 谓词；口径：跳过静音轨/静音摆放、每摆放谱面音符各计一次、不展开循环）。
- cli.rs：--sfz 块与 --export-wav 块均透出 notation_lines＋unmatched 行（普通乐器零新增行）。
- yeban-mcp：notes.rs 对象形 sampler 解析（read_sampler_binding/read_sampler_octave_override，未知键 unknownSamplerBindingField、坏步 samplerOctaveOverrideMustBeOctaveStep）＋编译层保留覆盖；op_schema anyOf 三形；tools.rs 文档句扩写；notes.rs 单元测试 3 个已加。
- 待办：app 集成判据文件、盒上验证（分批）、真实文件验收（待文件到位）、B.4 剩余抽查（builtin_sounds/鼓预置）、PROGRESS 收尾与回执。

## 变更（2026-11-19 11:16 父 agent 转达）
- 补充输入第 4 项（one_shot 掐尾附带核查）**取消**：Wang 确认报告人备注是报告人自己那边的问题、非引擎缺陷，2026-11-18 核查结论维持。本线不做、不在回执单列。
- 验证进度：盒上 yeban-sfz --lib 469/0 全绿（含新增 NoEvidence 单元测试）；fmt 已在盒上格式化并回同步提交（a176f1b6e）；app+mcp --all-targets --keep-going 检查在跑。

## 终局（2026-11-19 11:5x，yanshi-2 验证完毕）
- 判据套件 sfz_octave_override_cli **6/6 绿**（日志盒上 sfz-oct-final-suite.log）；红证据：4 个源文件退基线 9d238f09c 时套件编译失败 16 条 error（RED_EXIT=101，sfz-oct-red.log），恢复后复绿。
- 各 crate：yeban-sfz --lib 469/0（sfz-oct-sfz.log）；yeban-mcp --lib 754/0（含新增 3 个 setTrackSampler 测试，sfz-oct-mcp-lib.log）；yeban-app --lib 534/0（sfz-oct2-app-lib.log）；app 集成 apply_ops 9/0、sfz_model_chain 4/0、sfz_notation_adaptation 3/0；yeban-engine sfz_notation_adaptation 4/0。
- clippy：sfz 0、mcp 0；app 唯一 error 是基线既存 builtin_sounds.rs:90 vec_init_then_push（main 已于 43e657a15 修复、本线未碰该文件），本线改动文件零 clippy 发现。fmt 终查 FMT_EXIT=0。
- 中途事故（如实记）：① op_schema 内联 anyOf 撞 serde_json 宏递归上限，拆 sampler_binding_schema() 修复（60f795e45）；② 盒 /data 满致 yeban-app 链接 Bus error，清已合入线的 perlane-p5 target（9.7 GB）与 UI 补丁树 target 后恢复；③ 清 deps 可执行文件时误删 proc-macro .so 致 app-lib 一轮失败，cargo clean -p displaydoc 后复绿——今后清 deps 须排除 .so。
- **真实文件验收阻塞**：~/workspace/yeban-samples/ 在本 VM 不存在（多次 ls＋find 核查无果），SSO 原版与 fixed 副本未能实跑；验收由与报告形态 1:1 同构的夹具承担（c3/c#3/d3＋one_shot＋无 smpl WAV）。文件到位后按回执所列命令可直接复跑。
- 普查/钉数类按 §7 不在分支跑（main 上跑）。
