# 会话交接账（night-session handoff）

**台账类型**：交付清单 / 判决读数 / 未做项 / 续做指引。**记录时刻**：2026-10-08，`main` = `0a1c0d2`。
⚠️ **本文件是日期化记录**：后续会话应**加注**而不是改写（按本仓库对 `docs/ledger/*-notes.md` 的纪律）。

---

## 1. 判定口径（本会话遵守的）

- 判定来源只有两种：**真实运行的结果**与**读回的 CI 判决**。
- ⭐ **本会话每一个提交都按 sha 单独读回 `conclusion`**（`gh run list` 按 `headSha` 过滤），未读回的一律记 `pending`。
- ⚠️ **两处例外，如实登记**：`6a170ea` 与 `7de4f9f` 与各自的 tip 同批推送 ⇒ **只有 tip 有 run**；它们**没有自己的判决**（tip 已绿，覆盖同一棵树，但那不等于它们各自被判过）。

---

## 2. 本会话交付（51 个提交，全部已推送）

覆盖 **15 个 crate**。按主题分组：

### 2.1 真实设备与引擎（objective 的"端到端真跑通"）
| 主题 | 提交 | 判决 |
| :--- | :--- | :--- |
| 真声卡路径接线（本机 CoreAudio 实测 3 设备、`elapsed=133.042ms → 6400 帧`，比值 1.003） | `5d528b7` | ✅ success |
| 节拍器接进实时总线（逐样本路径只有比较/乘/加；`rt_zero_alloc` 40 → 44） | `b2b6d0f` | ✅ success |
| 限制器上移到 `yeban-dsp`（算法逐字未改；**迁移前后 4 个场景 sha256 相同**） | `b251348` | ✅ success |
| 复音合成器上移到 `yeban-dsp`（引擎侧逐位未改；5 个夹具指纹相同） | `4f6e2eb` | ✅ success |
| 逐轨 **compressor** 插入接线（`insert.rs` ＋ `rt.rs` ＋ `snapshot.rs` ＋ 零分配判据） | `c792fdc` | ✅ success |
| 逐轨 **channel strip** 插入接线（同款三件套；未武装路径哈希 `ec83e49a…` 不变） | `db1850f` | ✅ success |
| **reverb** 接线进行中（`tests/reverb_insert.rs` 已建；一条 doc 更正已落地） | `2ccdda7` | ⏳ pending |

### 2.2 `yeban-dsp` 器件（负责人清单）
| 器件 | 提交 | 判决 |
| :--- | :--- | :--- |
| compressor（前馈式；算术对账到 5e-7 dB；六种切分逐位相同） | `851dfdc` | ✅ success |
| channel_strip（组合既有器件；顺序契约判据用独立参照链；九种切分） | `8e46449` | ✅ success |
| limiter（上移） | `b251348` | ✅ success |
| polysynth（上移，双振荡器） | `4f6e2eb` | ✅ success |
| drums（4 配方 5 音色 ＋ **22 条未实现清单**） | `8532732` | ✅ success |
| 卷积混响（"对标 ReaVerb"） | `cb4bdaa` | ✅ success（⚠️ 刚起步，见 §4） |

### 2.3 格式与导出
| 主题 | 提交 | 判决 |
| :--- | :--- | :--- |
| 流式 `AssetHasher`（`yeban-model`） | `192ac07` | ✅ success |
| 离线 LRA ＋ 响度归一化预设 | `c458fb6` `89f444e` | ✅ success |
| MIDI tempo map 往返保真（**7/16 → 0/16**；工程路径字节不变） | `0e3c990` | ✅ success |
| MusicXML 只读 MVP（手写 pull parser，**零新依赖**；真语料 6/6 成功、6/6 `.mxl` 明确拒绝） | `d74d3ac` | ✅ success |
| SMF 拒绝路径 ＋ 真公有领域夹具（Beethoven） | `be04401` `acdb3fa` | ✅ success |
| `import_path` 容器流式哈希 | `ab42c82` | ✅ success |
| `history.dag` 编解码面公开化 | `5375585` | ✅ success |
| `bext` CodingHistory 不再写模板占位符 | `eaf0049` | ⏳ pending |
| `pcm_hash` 流式 | `ccee870` | ✅ success |
| WAV `fmt` 溢出拒绝 | `28ee6f6` | ✅ success |
| SFZ region → 可渲染采样描述 ／ ARIA `<master>` 四级 ／ offset-end-direction | `00354ec` `4bcf9fa` `18a8874` | ✅ success |

### 2.4 乐理
| 主题 | 提交 | 判决 |
| :--- | :--- | :--- |
| `GenreRule::sketch` 遵守规则自己的拍号 | `0ebcf75` | ✅ success |
| `swing` 整数网格（关掉台账 `pending 6`；34/182 流派，540–660‰） | `e191abd` | ✅ success |
| 拍号 ＋ swing → 节奏网格（新增 `rhythm.rs`） | `0a1c0d2` | ⏳ pending |

### 2.5 UI 可用性（objective 的第一目标）
| 主题 | 提交 | 判决 |
| :--- | :--- | :--- |
| 录音键不再假称"可勾选"（删除 `accessible-checkable`；保留的 `checked` 可证明惰性） | `066490f` | ✅ success |
| 另两处假 `accessible-checkable` 删除（`device_rack` 旁通开关、`app.slint` 声学诊断） | `7de4f9f` `042c3a5` | ✅ success |
| 编曲视图静音/独奏按钮接线（原为**点了没反应**） | `d3c954f` | ✅ success |
| 钢琴卷帘工具按钮接线 ＋ 撤销标签视图中性（R16 ＋ R17） | `1114f05` | ✅ success |

### 2.6 MCP 与文档
| 主题 | 提交 | 判决 |
| :--- | :--- | :--- |
| BASELINE-006 载荷统计（**UTF-8 字节口径**，不是 token） | `6a170ea` `2db665c` | ✅ tip success（⚠️ `6a170ea` 无独立 run） |
| 台账更正（R5 触发条件按实测改写） | `3328dca` | ✅ success |
| 裁决台账建立 ＋ 十四条裁决 | `894f04d` `a4e61ea` `5bd411f` `c6051b3` | ✅ success |
| 清行号漂移（17 → 0；后 9 → 0）＋ 两处错误陈述更正 | `f1ec03b` `bbad010` `677fa3f` `c6051b3` | ✅ success |
| 修 F1 判据缺第三个臂 | `c9717a8` | ✅ success |

---

## 3. 红线记录（本会话**从未调整任何红线**）

- **MUST-GATE-001**：新接进音频路径的器件都**新增或扩展了运行期零分配判据**（`compressor` / `channel_strip` / `polysynth` / `limiter` / 节拍器）；读数与注入证据写在各自提交的报告里。
- **逐位一致**：`limiter` 与 `polysynth` 上移都做了**迁移前后哈希对比**；`channel_strip` 接线做了**未武装路径哈希对比**（`ec83e49a…`）；`tempo map` 那次改动是**修缺陷**，已按 objective 要求**先明说并记录**（原判据钉住的是缺陷现状）。
- **`forbid(unsafe_code)`**：库目标未破；`dsp` 的集成测试用 `unsafe impl GlobalAlloc`（⭐ 树里有 ≥5 处先例）。
- **基准图**：本会话**从未改动**（R13 明确延后像素重录）。

---

## 4. ⚠️ 未做项（下一会话可直接接）

1. ⚠️ **`drums` 仍未接进引擎**（`compressor` / `channel_strip` / `reverb` 已在做/已做）。
2. ⚠️ **R9 状态栏诚实性未做完**（`selection` 可接 `bridge.rs:1800 selection_tick_span`；`device` 真实值是 `48 kHz / 32-bit float`，原文的 `24-bit` 是假的；`chord` **无能力** —— `yeban-theory` 不在 `yeban-app` 依赖图里）。⚠️ **动可见文本必须配一次基准重录**。
3. ⚠️ **卷积混响只是起步**（`convolution.rs` ＋ 一条零分配判据）；"对标 ReaVerb"远未完成。
4. ⚠️ **两条线在建**：`mcp-2`（4 项改动）与 `app-r9`（2 项改动）—— 会话结束时未提交。
5. ⚠️ **`reverb` 接线的判决未读回**（`2ccdda7` pending）。
6. ⚠️ **`6a170ea` / `7de4f9f` 无独立判决**。
7. ⚠️ **登记在案但未做**：`R6d`（`phase-status.md:108` 手抄旧读数）· `R14`（`ADR-0001` D44(a) 承诺的 errata 行在规范里不存在）· `R11b` · `R17` 之外的设备链票 · `sources` 类行号引用的长期维护。
8. ⚠️ **未跑默认档**（含 cpal）—— 本机纪律禁止；判决一律以 CI 的 `rust (*)` 作业为准。

---

## 5. 续做指引（本会话学到的、可直接用的）

1. ⭐ **多线用 worktree**：一个 crate 一条线，`git worktree add .worktrees/<name> -b line/<name> origin/main`；**crate 级隔离 ⇒ 文件面天然不相交**。
2. ⭐⭐ **推送前必须 rebase**：`git rebase origin/main` ⇒ 否则非快进被拒。**推完立刻 FF 本地 main**。
3. ⭐⭐ **一次推一个提交**：`git push origin line/<name>:main`。⚠️ **一次推多个 ⇒ 只有 tip 有 run**。
4. ⭐⭐ **每改 `crates/**` 必须真编译真测试**，并 **`--no-default-features`**（默认档含 cpal）。
5. ⭐⭐⭐ **接线音频路径的三件套**（已被 `c792fdc` / `db1850f` 验证）：
   - `insert.rs`：把 `InternalEffect` 的**名字约定**投影成器件参数，只收录非空链；
   - `snapshot.rs`：**构造期算一次 ＋ 快照边界武装**（`set_params` / `set_sample_rate`）；
   - `rt.rs`：逐样本只调 `process_*`；**并同步扩 `tests/synth_rt_zero_alloc.rs`**。
6. ⭐⭐ **没红的注入要如实报出**（本会话多条线都报过）。
7. ⭐ **"没有判据"必须全仓 grep**，不能只搜 `tests/`（判据常住在 `src/test_port_adapter.rs`）。
8. ⭐ **体裁纪律**：活表（`feature-alignment.md` / `gate-status.md` / `phase-status.md` / `human-decisions.md`）⇒ **就地改正**；`docs/ledger/*-notes.md` 与 `DEVELOPMENT_LEDGER.md` ⇒ **加带日期的注，保留原文**。
9. ⭐ **编造出处是最贵的错误**：本会话集成者写错过三处（"除 limiter 外都没接线"、"台账里 compressor 已接线"、"4× 过采样归给 ARCH-FMT-001"），**三处都被执行票核出并更正**。⇒ **凡在指令里给出处，必须可核对**。
