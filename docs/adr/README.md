# ADR 目录

Architecture Decision Record。当 Normative 规范之间冲突、或规范对某个实现细节**没有**定义，而
必须在今天做出选择才能继续开发时，把选择、依据、代价写在这里。

规则（来自 `docs/skills/yeban-dev-workflow/SKILL.md`）：

- **不修改 Normative 文档来做裁决。** 规范是权威事实源，改动它是人类的责任；Agent 的裁决写在 ADR 里，
  并明确标注是 Proposed 还是 Accepted。
- **每条裁决必须写代价。** "取更严的一个"、"取更晚的一份规范"都要写清楚代价是什么。
- **裁决必须可被推翻。** 写清"待人类批准"的条目，人类改判后更新本文件而不是另开一份。
- **规范从未定义的东西也要记。** 例如 UI 字体栈、间距/圆角 scale、DPR 细则、Splitter 约束对象——
  这些在 `ADR-0001` 的"缺口"里登记，实现时按登记的口径来，不各写各的。

| 编号 | 标题 | 状态 |
| :--- | :--- | :--- |
| [ADR-0001](./ADR-0001-workspace-topology-and-version-pinning.md) | 工作区拓扑、命名与版本钉死 | Accepted（负责人已于 2026-10-04 授权"按建议执行"，全部裁决追认） |
| [ADR-0002](./ADR-0002-baseline-verdict-hardware.md) | BASELINE 系列的判定硬件（参考机 vs 托管 runner） | **部分 Accepted / 部分 Proposed**（`BASELINE-003` 的**正式口径**一节（绘制回调耗时 ≤2ms）已获负责人批准，`HD-59`，2026-10-07；**硬件范围**那一节仍 Proposed，待 `HD-58`）|
| [ADR-0003](./ADR-0003-save-button-top-bar-slot.md) | 顶栏保存按钮的槽位（挪徽章 vs 改规范序列） | Proposed（待人类裁决）|
| [ADR-0004](./ADR-0004-track-height-timeline-zoom-and-folding.md) | 编排视图的轨道高度、横向时间轴缩放与轨道折叠（每轨持久数据 vs 视图缩放级；文件夹实体 vs 视图语义） | Proposed（待人类裁决）|
| [ADR-0005](./ADR-0005-single-mutable-project-authority.md) | GUI 进程里唯一可变的工程权威（选项 (a) 的追认、投影注入面与 `.yeban.lock` 写者边界） | Proposed（待人类裁决）|
| [ADR-0006](./ADR-0006-hardware-midi-input-via-midir.md) | 硬件 MIDI 输入经 `midir` | Accepted（负责人 2026-10-08 `HD-40`）|
| [ADR-0007](./ADR-0007-red-line-6-feature-wording.md) | 红线 6 的 feature 措辞 | Accepted（负责人 `HD-44`）|
| [ADR-0008](./ADR-0008-sfz-off-time-versus-arch-rt-004.md) | SFZ `off_time` 6 ms vs `ARCH-RT-004` 3 ms | Proposed（待人类裁决）|
| [ADR-0009](./ADR-0009-road-m1-005-historical-migration-scope.md) | `ROAD-M1-005` 历史迁移器的范围（条目是②，输入是③） | Proposed（待人类裁决）|
| [ADR-0010](./ADR-0010-clip-loop-semantics.md) | 片段循环的坐标语义（`start_tick`/`end_tick` 是**摆放相对**坐标，按区间长度平铺） | **Accepted**（集成者，负责人 2026-10-15 授权"择优自行裁决"）|
| [ADR-0011](./ADR-0011-render-depends-on-engine-for-the-sound-source.md) | 让 `yeban-render` 依赖 `yeban-engine`（`default-features = false`），把引擎音源接进 MCP 导出 | **Accepted**（负责人 2026-10-21 逐字裁决「统一」）|
| [ADR-0012](./ADR-0012-pitch-bend-curve-contract.md) | 弯音曲线（`MidiNote::pitch_bend_curve`）的坐标、形状与校验（音符局部 tick、`cents` ±1200、严格升序、首点必须为 0、线性插值＋末值保持） | Proposed（待人类裁决）|
| [ADR-0013](./ADR-0013-midi-track-name-fallback-order.md) | 标准 MIDI 文件（SMF）多轨导入音轨名回退机制与损失读数 | **Approved** (经 2026-10-25 常设授权裁决并完全落地 D1/D2/D3) |
| [ADR-0014](./ADR-0014-midi-pitch-bend-attribution-and-export.md) | MIDI 导入与导出弯音归属与线格式转换 | Proposed (依 Logic Pro 与 MIDI 1.0 通行规范裁决) |
| [ADR-0015](./ADR-0015-midi-cc-to-automation-mapping.md) | MIDI CC 到轨道自动化泳道映射（Volume CC 7 与 Pan CC 10） | Proposed (依 Logic Pro 与 MIDI 1.0 通行规范裁决) |
| [ADR-0016](./ADR-0016-propose-section-theory-generation.md) | `yeban_propose_section` 理论和声骨架生成支持（`materialSource: 'theory'`） | **Accepted（集成者依 2026-10-25 人类常设授权裁定）** |
| [ADR-0017](./ADR-0017-propose-section-melody-and-drums.md) | `yeban_propose_section` 旋律与鼓组理论生成扩展（`includeMelody` 与 `includeDrums`） | **Accepted（集成者依 2026-10-25 人类常设授权裁定）** |
| [ADR-0018](./ADR-0018-create-clip-with-placement.md) | `yeban_edit_notes` 支持在 `create: true` 时同时指定 `placement`（消除 WALL ①） | **Accepted（集成者依 2026-10-25 人类常设授权裁定）** |
| [ADR-0019](./ADR-0019-per-point-pitch-bend-editing.md) | `yeban_edit_notes` 支持逐点编辑弯音曲线（`setPitchBendPoint`，消除 WALL ⑤） | **Accepted（集成者依 2026-10-25 人类常设授权裁定）** |
| [ADR-0020](./ADR-0020-per-track-contribution-readout.md) | `yeban_render_master` 报逐轨贡献（`contributions`，消除 WALL ⑦） | **Accepted（集成者依 2026-10-25 人类常设授权裁定）** |
| [ADR-0021](./ADR-0021-tempo-map-playback.md) | 母带渲染真的读 `tempo_map`（分段常速回放） | **Accepted（集成者依 2026-10-25 人类常设授权裁定）** |
| [ADR-0022](./ADR-0022-offline-per-track-arming-and-silent-source-warning.md) | 离线渲染逐轨武装；母带回执点名静音源 | **Accepted（集成者依 2026-10-25 人类常设授权裁定）** |
| [ADR-0023](./ADR-0023-remove-track-routing-cleanup.md) | 取走音轨连带清理路由图（`removeTrack` 的孤儿节点与悬空边） | **Accepted（集成者依 2026-10-25 人类常设授权裁定）** |
| [ADR-0024](./ADR-0024-sfz-load-sample-warnings.md) | SFZ 装载对缺样本发非阻塞警告；采样外取流程入文档 | **Accepted（集成者依 2026-10-25 人类常设授权裁定）** |
| [ADR-0025](./ADR-0025-edit-notes-batch-projection.md) | `yeban_edit_notes` 的批内投影：路由建边可与建节点同一批（消除 G7） | **Accepted（集成者依 2026-10-25 人类常设授权裁定）** |
| [ADR-0026](./ADR-0026-generation-gain-staging.md) | 生成声部出厂即分级；种子轨电平 −12 dBFS | **Accepted（集成者依 2026-10-25 人类常设授权裁定）** |
| [ADR-0027](./ADR-0027-timbre-param-consumption-honesty.md) | 音色参数：撤掉 `deviceChainDsp` 的谎，注册口径改为"引擎是否真的消费" | **Accepted（集成者依 2026-10-25 人类常设授权裁定）** |
| [ADR-0028](./ADR-0028-section-voicing-register-and-drum-density.md) | 理论段 3 声部低音区修正；`drumDensity` 档位（normal / sparse） | **Accepted（集成者依 2026-10-25 人类常设授权裁定）** |
| [ADR-0029](./ADR-0029-propose-auto-merge.md) | 提案类工具新增 opt-in `autoMerge`：一次调用完成「提案 + 合并」 | **Accepted（集成者依 2026-10-25 人类常设授权裁定）** |
| [ADR-0030](./ADR-0030-stem-export-one-call-per-track-bounce.md) | 分轨导出（stem export）：一次调用完成逐轨 32-bit Float 落盘 | **Accepted（集成者依 2026-10-25 人类常设授权裁定）** |
| [ADR-0031](./ADR-0031-device-chain-ops-stay-in-edit-notes.md) | 设备链 CRUD **不**另开工具：继续走 `yeban_edit_notes` 的 op 词汇；摩擦由 `knownParams` 补偿 | **Accepted（集成者依 2026-10-25 人类常设授权裁定）** |
| [ADR-0032](./ADR-0032-receipt-self-describes-commit-model.md) | 每条成功回执自述 `commitModel`：把「这次要不要再合并」变成读数而不是记忆 | **Accepted（集成者依 2026-10-25 人类常设授权裁定）** |
| [ADR-0033](./ADR-0033-section-voice-subdivision.md) | 理论段和声声部的节奏切分（`voiceSubdivision`） | **Accepted（集成者依 2026-10-25 人类常设授权裁定）** |
| [ADR-0034](./ADR-0034-transpose-clip-note-op.md) | 片段级移调（`transposeClip`）：转调走已有 `MoveNote`，⛔ 不新增 op 变体 | **Accepted（集成者依 2026-10-25 人类常设授权裁定；D4 三条仍为待人类裁决的规范缺口）** |
| [ADR-0035](./ADR-0035-per-voice-subdivision.md) | 逐声部节奏粒度（`voiceSubdivisions`）：均匀切分按**声部名**给值 | **Accepted（集成者依 2026-10-25 人类常设授权裁定）** |
| [ADR-0036](./ADR-0036-onsets-per-bar-derived-from-meter.md) | 鼓与旋律的每小节音头数：默认从**拍号**导出，并开放 `onsetsPerBar` | **Accepted（集成者依 2026-10-25 人类常设授权裁定）** |
| [ADR-0037](./ADR-0037-generation-velocity-range.md) | 生成内容的力度：把既有的两级映射开放为 `velocityMin` / `velocityMax`（缺省逐字节 = 96 / 112） | **Accepted（集成者依 2026-10-25 人类常设授权裁定）** |
| [ADR-0038](./ADR-0038-generation-melody-register.md) | 旋律音域：把 `MelodyConstraints { lower, upper }` 接成 `melodyLower` / `melodyUpper`（缺省逐字节 = 48 / 84；不可行窗口原样转发 theory） | **Accepted（集成者依 2026-10-25 人类常设授权裁定）** |
| [ADR-0039](./ADR-0039-generation-melody-max-leap.md) | 生成旋律的跳进上界：把 `MelodyConstraints.max_leap` 开放为 `melodyMaxLeap`（缺省逐字节 = 12） | **Accepted（集成者依 2026-10-25 人类常设授权裁定）** |
| [ADR-0040](./ADR-0040-generation-harmony-voice-jump.md) | 生成和声的声部跳进上界：把 `VoicingConstraints.max_voice_jump` 开放为 `harmonyMaxVoiceJump`（缺省逐字节 = theory 出厂的 12；可行性唯一来源是 theory 的 `NoFeasibleVoicing`） | **Accepted（集成者依 2026-10-25 人类常设授权裁定）** |
| [ADR-0041](./ADR-0041-generation-drum-swing-permille.md) | 生成鼓的摇摆比例：把 `GenreRule::drum_pattern` 里写死的 `permille` 实参开放为 `swingPermille`（界 = theory 公开的 `500..=1000`；⭐ 契约故意⛔ 写 `default` —— 缺省值随流派登记移动） | **Accepted（集成者依 2026-10-25 人类常设授权裁定）** |
| [ADR-0042](./ADR-0042-harmony-voice-class-coverage.md) | 和声生成的**音级覆盖性**：`voice_leading::realize` 当下⛔ 保证"和弦每个构成音都被发出"，本票把这一实测缺口与三条路线登记成人类裁决（⚠ 卡住 `harmonyVoices` 与 `harmonyLower`/`harmonyUpper` 两张票） | **Proposed（待人类批准；⛔ 集成者代签 —— 本票不改代码）** |
| [ADR-0043](./ADR-0043-generation-drum-backbeat.md) | 生成鼓的军鼓落点：把 `swung_styled_drum_pattern` 里派生的 `backbeat` 实参开放为 `drumBackbeat`（界 = `1..=该拍号的拍数`，现读 `felt_beats_per_bar(meter)`；⭐ 契约故意⛔ 写 `default`、⛔ 写 `maximum` —— 两者都随拍号移动） | **Accepted（集成者依 2026-10-25 人类常设授权裁定）** |
| [ADR-0044](./ADR-0044-generation-drum-style.md) | 生成鼓的底鼓口径：把 `GenreRule::drum_pattern` 里最后一个写死的实参 `self.drum_style` 开放为 `drumStyle`（界 = theory 公开的 `DrumStyle::ALL` × `name()`，候选⛔ 在 MCP 抄字符串；⭐ 契约故意⛔ 写 `default` —— 缺省值随流派登记移动：182 条里 169 走 `metric`／13 走 `four-on-the-floor`） | **Accepted（集成者依 2026-10-25 人类常设授权裁定）** |
| [ADR-0045](./ADR-0045-generation-melody-seed.md) | 生成旋律的**取音种子**：把 `melody_contents` 里写死的第 5 实参 `section_seed(section_id)` 开放为 `melodySeed`（界 = `0..=u64::MAX`＝`arg_u64` 的类型面；⭐ 契约故意⛔ 写 `default` —— 缺省那条是随身份移动的派生值，⛔ 写 `maximum` —— 种子无自然上界。⚠ 换种子换的是**音高与由它派生的力度档**（`chord_tone = chord_mask[pitch % 12]`），⛔ 换格位、⛔ "只换音高"） | **Accepted（集成者依 2026-10-25 人类常设授权裁定；暂记缺口 `G8m`）** |
| [ADR-0046](./ADR-0046-generation-drum-beat-grouping.md) | 生成鼓的**拍内分组**：把 `swung_styled_drum_pattern` 里写死的 `None`（分组）开放为 `beatGrouping`（单位＝**拍数序列**，唯一裁判是 theory 的 `BeatGrouping::new`；⭐ 契约故意⛔ 写 `default` —— 缺省那条随拍号移动，⛔ 写 `maxItems`／`items.maximum` —— 长度与元素上界都由拍号决定。⚠ 只动 `weight`／组起点 ⇒ 换底鼓落点与重音档，⛔ 换格点、⛔ 换踩镲） | **Accepted（集成者依 2026-10-25 人类常设授权裁定；缺口 `G8n`）** |
| [ADR-0047](./ADR-0047-generation-melody-motif.md) | 旋律**动机**（motif／句法／终止式）：theory（`melody.rs:16-18` 边界声明）与 MCP（`project_create.rs` 的片段默认名 `"Motif"` 只是名字）两侧**都没有**对应概念；本票把三条路线（D1 明确不做／D2 最小轮廓重复／D3 完整句法）登记成人类裁决（⚠ 卡住 `G8f` 动机半片） | **Proposed（待人类批准；⛔ 集成者代签 —— 本票不改代码）** |
| [ADR-0048](./ADR-0048-idempotency-cache-size-bound.md) | 幂等缓存的**条目数**无上界（只有键长 `maxLength: 256`；`BTreeMap` 无淘汰、跨工程生命周期不清）：设上限的三条路（拒绝／驱逐／降级）**都要动闭合契约或核心保证** ⇒ 登记成人类裁决（⚠ `D1` 仅补文档、零代码） | **Proposed（待人类批准；⛔ 集成者代签 —— 本票不改代码）** |
| [ADR-0049](./ADR-0049-project-field-write-reachability.md) | 四个**读得到、写不了**的工程字段（`audio_config`／`rng_seed`／`metadata.description`+`tags`／`solo_safe`）要不要开放为可写：S3 常驻判据钉的零触点面 + 每字段的消费者读数（`solo_safe` 引擎 **19** 行、`audio_config` 设备投影 `device.rs:341-346`、`metadata.description` 读取面 **0** 处）+ "开放会撞到哪条既有保证"登记成五选一裁决（⚠ `D1` 仅补文档、零代码；`D5` 撞设备生命周期，建议最后裁） | **Proposed（待人类批准；⛔ 集成者代签 —— 本票不改代码）** |
| [ADR-0050](./ADR-0050-device-param-table-dedup-and-seen-semantics.md) | 设备参数**两张表**（dsp 落点表 41/6/8 ↔ 实时臂表 41/6/11，唯一允许差集 = 3 个 IR 形状键）的去重：名字面已被 S2 的常驻判据钉住，⛔ 钉住的是"名字→目标字段"这一轴，而实时侧的**级启用推导**（`seen`，`insert.rs:891/895/897/907`）在共享表里**无对应物** ⇒ 四条路线（D1 只补钉映射／D2 表加 `level()`／D3 投影改集合式／D4 整张投影搬进 dsp）登记成人类裁决（⚠ 闸是 `compressor_insert.rs:129-130` 那条"不认识的效果器必须逐字节相同"的 C0） | **Proposed（待人类批准；⛔ 集成者代签 —— 本票不改代码）** |
| [ADR-0051](./ADR-0051-sidebar-preset-category-carrier.md) | 左侧资源栏「预置」分类**没有事实源**：实测模型层 `preset` **0 行**、`ops.rs` 的 `Preset` 变体 **0 行**，而唯一成规模的只读候选是 theory `genre.rs:537` 的 `GENRES` —— **182 条**流派（唯一消费者是控制面 `yeban_propose_section` 的 `stylePreset`，落**隔离分支** `ai/proposal-{ulid}`）；「预置」一词的唯一出处是已降级为 Advisory 的 UI/UX 稿 `:84` 「预置模板」⇒ 三种读法（patch／采样包／流派）都无绑定依据。五条路线（D1 维持空态／D2 投影 182 条／D3 模型层新增预置实体+Op／D4 = D2+改名「风格预设」／D5 删格）登记成人类裁决，并已落表 `HD-71`（⚠ 单独批 D2 会造出**假控件**：UI 侧零个消费流派的 `UiAction`） | **Proposed（待人类批准；⛔ 集成者代签 —— 本票不改代码）** |
| [ADR-0052](./ADR-0052-actionless-drawn-controls-disposition.md) | 界面上「画了但今天**没有任何动作**」的三个控件（现读 `KNOWN_ACTIONLESS_CONTROLS` ＝ **3** 条：`ai-rail-intent-button`／`transport-commit-button`／`transport-branch-button`）怎么处置：三条常驻判据已把"今天确实不可用"逐名钉死，但它们钉的是**今天**、不是路线 ⇒ 维持／置灰／移除／补能力／换角色五选一登记成人类裁决。⚠ 三键各缺**不同的一层**（产品入口／会话状态层／领域操作面＋撤销契约），**且**分支键⛔ 缺模型能力 —— `CommitGraph::create_branch`（`commit.rs:492`）等四个原语**已在**并被提案路径用着（`domain/mod.rs:4391`），该判据"API 一处都不存在"只在领域面成立；任何用户可见处置的代价＝**5 张 Linux 基准重录 ＋ 人工复核**（`HD-56`） | **Proposed（待人类批准；⛔ 集成者代签 —— 本票不改代码）** |
| [ADR-0053](./ADR-0053-stems-export-ui-entry-blocked-by-mcp-dependency-edge.md) | 界面上的「导出分轨」入口⛔ 接不了：它唯一诚实的落点 `yeban_mcp::domain::stems`（现读 `crates/yeban-mcp/src/domain/stems.rs` **764** 行：`validate :87`／`export :390`／`write :521`／`stem_file_name :689`）在**默认发行构建里不存在**——三重现读 `crates/yeban-app/Cargo.toml:189` 的 `optional = true` ＋ `src/lib.rs:176-185` 整块 `#[cfg(feature = "in-process-mcp")]` ＋ `.github/workflows/release.yml:434` 那条 `[ "$DEFAULT_MCP" = "0" ]`（`:437-438` 还带正对照 ⇒ ⛔ 是空绿）；而"补第二份分轨实现"违反 `ADR-0005`，"先画按钮再接"会造出一颗**有** `clicked` 却无宿主 handler 的假控件（**骗过** `ADR-0052` 那条按"无点击源"扫的常驻判据）⇒ 已回退那份未提交的 `.slint`（＋88 行 / 3 文件）。四条路线（D1 给 app 开**默认**内部边 → 推翻红线 6 那句理由／D2 抽共享叶子 crate（`yeban-diagnostics` 先例）→ 需切 `domain/render.rs` **5539** 行且与 MCP 线排队／D3 按 feature 门控控件 → 最差但最便宜／D4 界面侧第二份实现 ＋ D5 承认这一格⛔ 属于界面并关掉本票）登记成人类裁决 | **Proposed（待人类批准；⛔ 集成者代签 —— 本票不改代码）** |
| [ADR-0054](./ADR-0054-singing-synthesis-v2-deferral.md) | Zimmer 2026-10-31 的 `G3`（无歌声合成）该怎么读：现证**载体早已在规范里**（`YEBAN_ARCHITECTURE_AND_SYSTEM_DESIGN.md:342-343` 的 `syllable` / `phonemes`）且工具面**真的收**（`tools.rs:930`），`syllable` 已能写成 SMF `FF 05`（`#312`）并按"唯一才归属"读回（`#317`），母带侧 `unsupported.noteLyrics`（`render.rs:56/675/4423`）**明说**不唱 ⇒ 回执没撒谎。承重的那一行是 `ROADMAP:478`：「本地神经歌声合成 (SVS)」排在 **v2.0.0** ⇒ `G3` 是**按规范推迟**、不是 Phase 4 漏做（`DESKTOP_UI_UX` §9.3 那句"绑定 SVS 声学集合"出自已降级的 Advisory 稿 `:372-373`）。四条路线（D1 关掉本票、只给 `noteLyrics` 补一句"属 v2.0.0"的可发现性／D2 借**既有** `VOCAL` 波表做"哑唱"——会改所有带歌词工程的 digest ⇒ 5 张基准重录／D3 提前开工 SVS——同时撞红线 7（回调零分配）／8（`unsafe` 审计）／9（权重登记）／D4 合唱采样走 SFZ，被 `G4` 素材缺口挡住：现读 `assets/samples` **1 586** 个文件 = `.sfz` 1 398 ＋ `.txt` 170 ＋ `.md` 11 ＋ `.xml` 6 ＋ `.json` 1、**音频样本 0 个**，清单 `policy` 逐字写明登记式外取 9.371 GiB）登记成人类裁决；⛔ 四份规范 `formant` 0 命中、`vocal` 0 命中 ⇒ 音素→共振峰映射表**无规范依据**，不得私自发明 | **Proposed（待人类批准；⛔ 集成者代签 —— 本票不改代码）** |
