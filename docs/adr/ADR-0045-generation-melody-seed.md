# ADR-0045 — 生成旋律的取音种子：把 `melody_contents` 里写死的 `section_seed(section_id)` 实参开放为 `melodySeed`（缺省 = 不覆盖派生种子 ⇒ 契约⛔ 写 `default`，且⛔ 写 `maximum`）

- 状态：**Accepted（集成者依 2026-10-25 人类常设授权裁定；暂记缺口 `G8m`）**
- 日期：2026-10-28
- 关联：`ADR-0017`（D3 派生种子的出处）、`ADR-0016`（键缺席逐字节兼容）、`ADR-0038`/`ADR-0039`（旋律那三把标尺与 `MelodyRulers`）、`ADR-0041`/`ADR-0043`/`ADR-0044`（同族的"派生值不可覆盖"三张鼓票）、`AGENTS.md` §6 与 §6.3、规则第 19 条（先 grep 消费点）

## 背景（先测后改，AGENTS.md §6）

**本节在数什么**：① 现在这个实参是什么（单位 = 处，`grep` 现读）；② 种子在 theory 里到底被谁消费
（单位 = 处）；③ 换种子会改变旋律的哪一维（单位 = 不同的 `(起点, 时值, 音高)` 形状条数／`onset`
去重集合大小）。全部现读，⛔ 凭记忆。

1. **覆盖点只有一处，且写死为派生值**：`crates/yeban-mcp/src/domain/section_build.rs` 的
   `melody_contents` 调 `melody_over_chords(&theory.scale, spans, &grid, MelodyConstraints { … },
   section_seed(section_id))` —— 第 **5** 个实参。`section_seed`（同文件 `:2076`）把 `EntityId`
   的 128 位 ULID 拆成 `low ^ high` 两个 `u64` 再亦或 ⇒ **段落身份决定旋律**。
   `ADR-0017` D3 明写这是有意的（"同一 `section_id` ⇒ 同一条旋律"）。
2. **种子的消费面比想象的小、但⛔ 只到音高**：`crates/yeban-theory/src/melody.rs` 里 `seed` 只经
   `pick_within_leap`（`:387..411`）走到 `crate::derive_index`（SplitMix64，纯 64 位整数），
   作用是**在满足跳进上界的可行候选里确定性地取一个音高**；节奏与 `onset` 来自
   `GenreRule::rhythm_grid`，⛔ 读种子 ⇒ **换种子换的是 `start_tick`/`duration_ticks` 之外的两样**：
   `pitch` 与由它派生的 `chord_tone`（`melody.rs` 结尾 `chord_tone: chord_mask[(pitch % 12)]`）。
   ⚠ **`chord_tone` 是 MCP 层力度阶梯的开关**（`section_build.rs` 的
   `if note.chord_tone { ladder.1 } else { ladder.0 }`，`ADR-0037`）⇒ **力度档也会跟着换**，
   判据⛔ 能声称"只换音高"。另外 `pick_within_leap` 的可行性依赖 `previous`（上一个音高）
   ⇒ **换种子可以改变后面上 onset 时取不取得到音**（`NoFeasibleVoicing` 的出现与否⛔ 是种子无关量）。
   ⚠ 这与鼓线三键不同：那三把改的是击点，这一把改的是音高与力度档 ⇒ 判据形状⛔ 照抄鼓票。
3. **代理测量**（本机那段跑的是 `genre_melody_with`，`spans` 取自 `GenreRule::sketch`）：
   5 个流派 × `seed 0..=63` = **320 组** ⇒ **320 个不同的 `(起点, 时值, 音高)` 形状**（无一碰撞）、
   `onset` 去重集合**恒定**、`Melody::seed()` 恒回显被消费的 seed、`Err` **0 次**。
   ⚠ **这组数⛔ 能当 MCP 路径的证据**：MCP 的 `spans` 来自调用方给的 `progression`。
   ⇒ 真正的测量由本票判据在 CI 里做（读数 2 的"只换音高"若是假的，判据 3 会红）。
4. **类型面**：`arg_u64`（`domain/mod.rs:1892`）= `Value::as_u64` ⇒ 接受 `0..=u64::MAX` 的任意
   非负整数，负数／浮点／字符串一律 `INVALID_PARAMETER_RANGE` 且报文点名该键。
   ⇒ 本键**没有自然上界**（种子是标识量而⛔ 数量），形状与 `progression` 同一支豁免。

## 决定

- **D1 — 新增可选键 `melodySeed: integer`，界 = `0..=u64::MAX`，缺省 = 不覆盖派生种子** ⇒
  键缺席必须**逐字节**产出同一份 `SectionPlan`（`ADR-0016`）。单位 = **SplitMix64 的 64 位输入**。
  ⚠ 契约里⛔ `default`（缺省那条是 `section_seed(section_id)`，它随身份移动，写死任何一个数都是撒谎）；
  ⛔ `maximum`（读数 4：无自然上界）；`minimum: 0` 仍登记（那一支不随任何东西移动，且与
  `arg_u64` 的报文措辞"非负整数"逐字一致）。
- **D2 — ⛔ 在 MCP 层校验"这个种子好不好"**：种子没有合法/非法之分（只有"类型对不对"，那是
  `arg_u64` 那一条），可行性与音级正确性全归 theory（`melody.rs` 的既有保证）。本层唯一的
  额外判定是 D3 的消费耦合。⛔ 抄一张"推荐种子表"，⛔ 做"这个种子会产生无聊旋律"的判断。
- **D3 — 消费校验只要求 `materialSource:"theory"` 且 `includeMelody:true`**：与 `ADR-0041` D3 /
  `ADR-0043` D3 同一支（两个报文分别点名 `materialSource` / `includeMelody`），
  ⛔ 与鼓那三键的 `includeDrums` 折成同一个 `is_some()`；⚠ **反钉**：`includeDrums:false`
  时必须放行（本键⛔ 读鼓）。
- **D4 — `MelodyRulers` 加字段 `seed: u64`**（`ADR-0039` D6 的那个结构，现有 `register`/`leap`/`ladder`），
  ⛔ 再走"加第 10 个位置参数"。字段类型取**已解析**的 `u64` 而⛔ `Option<u64>`：缺省值在
  `plan_with_options` 里就已经折成 `options.melody_seed.unwrap_or_else(|| section_seed(section_id))`
  ⇒ 结构体里不留"没给"这个中间态（留了就要在每个消费点重做一遍 `unwrap_or_else`，
  而 `section_id` 在那一层⛔ 总在场）。
- **D5 — 接线只换那一个实参**：`melody_contents` 的第 5 实参由 `section_seed(section_id)` 改为
  `rulers.seed`；`section_seed` 的调用点**上移一处**到构造 `MelodyRulers` 的地方（同一函数内），
  这样"键缺席 ⇒ 传入 `section_seed(section_id)`"是**结构**保证而⛔ 判据保证。其余两支
  （`MelodyConstraints` 的三个字段、`theory_invalid(..., "melody_over_chords", ...)` 转发）不动。
- **D6 — ⛔ 新增故障词**：D3 的两个拒绝复用既有 `InvalidParameterRange` + `field`/`value`/`hint`
  载荷形状，⛔ `reason` 键；D1 的类型越界复用 `arg_u64` 现成报文 ⇒ `fault_vocabulary` 词表不变。
- **D7 — ⛔ 把 `melodySeed` 说成"随机种子"**：它是**确定性**取音的输入（`ARCH-DET` L1/L2 语义），
  同一 `(section_id, melodySeed, progression, …)` ⇒ 同一份字节。文档与 `tools.rs` 的 `param`
  描述必须写"确定性取音的输入，换值 ⇒ 换**音高与由它派生的力度档**，⛔ 换格位（`onset`/时值）"，
  ⛔ "随机/即兴"这类词，也⛔ 写成"只换音高"（读数 2 的 `chord_tone` 那一条）。

## 非范围（登记，⛔ 补齐）

- `N1` **鼓的取种面**：`theory_drum_pattern` ⛔ 读种子（鼓型完全由流派 + 三键决定）⇒ 本票⛔ 顺手给
  鼓加种子；那是另一张票，且要先回答"鼓型是否需要变体"这个数据裁决。
- `N2` **和声的取种面**：`voice_leading::realize` ⛔ 消费种子（走向由调用方给）⇒ ⛔ 本票发明。
- `N3` **音级/风格层面的"这条旋律好不好"**：theory 只保证合法（在调内、跳进不越界），
  审美筛选⛔ 是任何一层的职责（`ADR-0042` 的覆盖性问题是另一件事）。
- `N4` **旋律动机（重复／模进）**：`GAPS.md:139` 五轴里唯一未交付那一轴的半片，卡在 `§0.6.3`
  的人类裁决；本键只换"取哪一个可行音"，⛔ 产生动机结构。

## 判据（`crates/yeban-mcp/tests/section_melody_seed.rs`，6 条）

1. `absent_seed_is_byte_identical_to_the_derived_seed` —— 键缺席 ≡ 给 `section_seed(section_id)`
   ⇒ 整份 `SectionPlan` **逐字节**相同（走 D5 那一支，等价性是实现结构给的）。
   顺带钉夹具形状（旋律轨在场、多条音符、和声与鼓在场）。
2. `the_seed_is_idempotent_per_section` —— 同一份入参（含同一 `melodySeed`）跑两次 ⇒ 逐字节相同；
   ⚠ 这条钉的是 `ARCH-DET`，⛔ 把它写成"两次调用返回同一个对象"。
3. `a_different_seed_changes_pitches_and_velocity_never_the_grid` —— 换 `melodySeed` ⇒
   **音符条数不变**、`onset` 去重集合不变、`(起点, 时值)` **序列**逐条不变（格位维），
   而 `(音高)` 多重集改变（读数 2）；⚠ **力度档允许跟着变**（`chord_tone` 由音高派生），
   判据⛔ 把力度当成不变量，也⛔ 把形状表整条比对（那会把音高变化也判成"格位变了"）。
   鼓与和声两轨与缺省跑**逐字节**相同（消费面 = 只有旋律读它）。
   ⚠ ⛔ 比 id 集合（音符 id = `theory-melody:{section}:{note_index}`，与种子无关，比它必然恒等）
   ⇒ 比**排序后的形状表**，且分两张：格位表 `(start, dur)` 与音高表 `pitch`（多一个力度表做"可变"记录）。
4. `non_integer_or_negative_is_rejected_naming_the_key` —— `-1` / `1.5` / `"7"` ⇒
   `INVALID_PARAMETER_RANGE` 且报文点名 `melodySeed`；端点 `0` 与 `u64::MAX` 必须放行（读数 4）。
5. `the_seed_key_requires_the_theory_source_and_the_melody_part` —— 脱离 `materialSource:"theory"`
   ⇒ 拒且 `hint` 点名 `materialSource`；`includeMelody:false` ⇒ 拒且 `hint` 点名 `includeMelody`；
   ⚠ **反钉**：`includeDrums:false` 时必须放行。
6. `melody_seed_is_declared_on_both_param_surfaces_without_default_or_maximum` —— `tools.rs` 的
   `param("melodySeed", …)` 与 `schemas/mcp-tools.schema.json` 的 `properties.melodySeed`
   同时在场，契约里⛔ `default`、⛔ `maximum`，且 `tools.rs` 的文档⛔ 出现"随机"这类词（D7）。

### 待 CI 回答的两件事（⛔ 在本票里预先"补齐"）

- 读数 3 是**代理**：MCP 路径（`spans` 来自 `progression`）下"换种子⛔ 换格位"是否仍成立、
  以及 **`NoFeasibleVoicing` 会不会随种子出现**（读数 2 末尾：可行性依赖 `previous`）。
  ⇒ 判据 3 的取样必须**同时覆盖多个种子**并把"某种子报错"当作**要登记的事实**，⛔ 为了让判据绿
  而只挑不报错的种子（那是把测量改成结论）。
- "320 个不同形状"在 MCP 路径上的对应数（判据 3 只要求**改变**，⛔ 声称 distinct 数）。
