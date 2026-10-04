# render-master 工作线账本

> 分支 `line/render-master`，工作树 `.worktrees/render-master`，地盘 `crates/yeban-render/**`。
> 本文件只由本工作线维护；`docs/DEVELOPMENT_LEDGER.md`、根 `Cargo.toml`、`schemas/**`
> 均由集成者独占，本线未触碰。

## 1. 核验过的上游格式事实与出处

**纪律说明**：下表把"**实际抓取过**的来源"与"只定位到、本机**不可机读**的来源"分开列。
本机 `web_fetch` 明确拒绝 `application/pdf`，因此 EBU Tech 3306/3285 与 ITU-R BS.2088
的 PDF **没有**被我读到，它们只作为规范编号引用；字段值一律由**可读的独立实现**核验。

### 1.1 实际抓取过的来源（字段值由此核验）

| 出处 URL | 用途 | 核验到的字段/行为 |
| :--- | :--- | :--- |
| <https://raw.githubusercontent.com/FFmpeg/FFmpeg/master/libavformat/wavenc.c> | FFmpeg 的 WAV/RF64 复用器源码 | `RF64` + `avio_wl32(-1)`；`WAVE` 后紧跟 `ds64`（在 `fmt ` 之前）；`ds64` chunk 长度 **28**；字段顺序 `u64 riffSize, u64 dataSize, u64 sampleCount, u32 tableLength`；`data` 的 32 位长度写 `-1`；`riffSize = file_size - 8` |
| <https://raw.githubusercontent.com/libsndfile/libsndfile/master/src/rf64.c> | libsndfile 的 RF64 实现 | 同上三条独立复现；**`ds64` 的第三个 `u64` 写的是 `psf->sf.frames`（帧数/每声道采样数）**，日志行也打印 `Frames`；`if (psf->dataend & 1) write 1 byte`（偶数补位）；`WAVE_FORMAT_EXTENSIBLE` 的 `fmt ` 负载 = 40 字节（`cbSize`=22、`wValidBitsPerSample`、`dwChannelMask`、16 字节 GUID）；默认声道掩码 1→`0x4`、2→`0x3`、4→`0x33`、6→`0x3F`、8→`0xFF` |
| <https://exiftool.org/TagNames/RIFF.html> | ExifTool RIFF 标签表 | `bext` 字段偏移：`Description`@0、`Originator`@256、`OriginatorReference`@288、日期@320、`TimeReference`@338、`BWFVersion`@346、`BWF_UMID`@348、**`CodingHistory`@602**（对 v1/v2 恒定）；`ds64` 的 `RIFFSize64`/`DataSize64`/`NumberOfSamples64` 以 8 字节为单位排在索引 0/1/2 |
| <https://en.wikipedia.org/wiki/RF64> | RF64/BW64 关系 | "32-bit chunk size field at offset 4 is set to `-1`，紧接一个 `ds64`（在 `FMT` 之前）"；BW64 = `BW64` 取代 `RF64`，加 `axml`/`bxml`/`sxml`/`chna`，并以 XML 元数据取代 `bext`/`ubxt` |
| <https://en.wikipedia.org/wiki/Dither> | TPDF 定义 | §Noise distributions："Triangular distribution can be achieved by adding two independent RPDF sources"；§"Which noise distribution to use"：TPDF 幅度是"**two quantization steps**"，取值例如 −1..+1 或 0..+2 |
| <https://raw.githubusercontent.com/chirlu/sox/master/src/dither.c> | SoX 抖动实现 | `flow_no_shape` 把**两个**各覆盖 1 LSB 的随机量相加后再四舍五入；钳位条件是 `i <= (-1 << (prec-1))` / `i > SOX_INT_MAX(prec)`；形状为两个 ±0.5 LSB 均匀量之和，即 **±1 LSB 的三角分布** |
| <https://ffmpeg.org/ffmpeg-formats.html#wav> | FFmpeg 格式文档 | 抓取成功但**不含** `ds64` 细节（如实记录：这条来源对本次核验没有贡献） |

### 1.2 本机不可机读、只作规范编号引用的来源（**未读到内容**）

| 来源 | 尝试结果 |
| :--- | :--- |
| EBU Tech 3306（RF64）`https://tech.ebu.ch/docs/tech/tech3306v2_0.pdf` | `web_fetch` 报 `unsupported content type "application/pdf"` |
| EBU BWF Embed Guideline v3 `https://www.digitizationguidelines.gov/audio-visual/documents/BWF_Embed_Guideline_v3_2021.pdf` | HTTP 403（Cloudflare 拦截） |
| EBU Tech 3285（BWF）`https://tech.ebu.ch/docs/tech/tech3285.pdf`、ITU-R BS.2088 | 未抓取（同为 PDF） |

**由此产生的规范缺口（登记在 §8 `needs`）**：`bext` v2 五个响度字段的"未知"哨兵值没有
从权威原文核验到。本实现取 `Loudness::UNKNOWN = i16::MIN`（0x8000），并在代码注释里标明
该值待人类按 EBU Tech 3285 s5 附录裁决。

### 1.3 由依赖源码直接核验的第三方 API（本地 registry 源码，非文档）

源码路径：`/Users/crow/work/music/yeban/.worktrees/.cargo-home/registry/src/index.crates.io-1949cf8c6b5b557f/<crate>-<version>`。

| crate | 版本 | 核验到的关键事实 |
| :--- | :--- | :--- |
| `midly` | 0.5.3 | `default = ["alloc","std","parallel"]`，**`std` 不是默认之外的东西而是需要显式打开的 feature**；`Smf::write_std` 带 `#[cfg(feature = "std")]`，签名 `fn write_std<W: io::Write>(&self, out: W) -> io::Result<()>`（`out` **按值**传）；`write_raw` **不会**自动追加 `EndOfTrack`，必须自己写 `Meta(EndOfTrack)`；`Track<'a> = Vec<TrackEvent<'a>>`；受限整数内部类型 `u4:u8, u7:u8, u15:u16, u24:u32, u28:u32`，`u28::try_from(u32) -> Option<u28>` 是**固有方法**而 `u28::new(u32)` 会掩码 |
| `hound` | 3.5.1 | 清单**没有** `[features]` 段，`default-features = false` 无副作用；`WavSpec{channels,sample_rate,bits_per_sample,sample_format}`；`WavWriter::create<P: AsRef<Path>>`、`write_sample<S: Sample>(&mut self, S) -> Result<()>`、`finalize(self)`；`WavReader::open`、`spec()`、`samples::<S>()`；`impl Sample for i32` 支持 `(24,3)` 读写、`impl Sample for f32` 支持 `(32,4)`；读 `WAVE_FORMAT_EXTENSIBLE` 要求 `chunk_len >= 40` 且 `cbSize == 22`，只认 `KSDATAFORMAT_SUBTYPE_PCM`/`_IEEE_FLOAT` 两个 GUID（16 字节常量已逐字节对照）；`Error` 实现 `Display` + `std::error::Error` |
| `rayon` | 1.12.0 | `[features]` 只有 `web_spin_lock`，**没有** `std` 之类的默认 feature，因此根清单的 `default-features = false` 不影响 `par_iter_mut` / `try_for_each` / `ThreadPoolBuilder` |
| `libm` | 0.2.16 | `default = ["arch"]`；抓到的源码里 `feature = "arch"` 无任何使用点（只在 build 脚本层），故保留根清单的默认；`powf(x: f32, y: f32) -> f32`、`exp2f`、`log10f`、`roundf`、`sqrtf` 均在 `libm::` 根下 |

### 1.4 工具链

`rustc 1.99.0 (b940084d7 2026-09-28)` / `cargo 1.99.0 (5f94df478 2026-08-27)`，
与 `rust-toolchain.toml` 的 `channel = "1.99.0"` 一致（本机 `local-env.sh` 在受限沙箱里
切到 `RUSTUP_TOOLCHAIN=stable`，解析到同版本）。

## 2. 落地文件与规范 ID

| 文件（相对 `crates/yeban-render/`） | 内容 | 规范 ID |
| :--- | :--- | :--- |
| `src/lib.rs` | 模块装配、crate 级契约文档、端到端契约测试 | 全部 |
| `src/render.rs` | 拓扑分层并行调度 + 按 `EntityId` 字典序的确定性串行归约 | `ROAD-M4-004`、`ROAD-M4-005`、`ARCH-DET-002` |
| `src/pdc.rs` | 关键路径延迟分析（Kahn + 全局 `L_max`）与环形延迟线 | `ARCH-PDC-001`、`ARCH-PDC-002` |
| `src/rf64.rs` | RF64/BW64 容器 + `bext` v1/v2 自研读写（零第三方依赖） | `ARCH-FMT-001` |
| `src/dither.rs` | TPDF 抖动 + 16/24/32f 位深转换 | `ARCH-FMT-001`、`ARCH-DET-001` |
| `src/wav.rs` | 普通 RIFF WAV 读写（`hound` 作独立第三方裁判） | `ARCH-FMT-001` |
| `src/midi.rs` | SMF 0/1 导出与严格回读 | `ARCH-FMT-001 §5.5` |
| `src/vlq.rs` | MIDI VLQ 的零依赖参考编解码（独立核验 `midly`） | `ARCH-FMT-001 §5.5` |
| `src/sum.rs` | 确定性有序归约核（刻意零第三方依赖） | `ARCH-DET-002` |
| `src/rng.rs` | `yeban_dsp::noise::Rng` → 抖动接口的适配 + 按节点派生种子 | `ARCH-DET-001` |
| `verify/pure_modules.rs` | **非 cargo 目标**的本机零依赖验证脚手架（见 §5） | — |
| `Cargo.toml` | 依赖与 feature 声明 | — |

## 3. 依赖决策（**没有引入任何新包**）

因此 **没有 `TODO(hoist)`**：全部依赖都已在根 `[workspace.dependencies]` 登记
（`yeban-model`、`yeban-dsp`、`libm`、`sha2`、`rayon`、`midly`、`hound`；
dev-dependencies `proptest`、`tempfile`）。

只有一处需要在本 crate 的清单里补 feature：

```toml
midly = { workspace = true, features = ["std"] }
```

理由见 §1.3：根清单用 `default-features = false` 登记 `midly`，而 `Smf::write_std` 属于
`midly` 的 `std` feature。**版本仍由根清单唯一决定**，本线只补 feature。

`Cargo.lock` 新增 6 个包：`rayon 1.12.0`、`rayon-core 1.13.0`、`crossbeam-deque 0.8.8`、
`crossbeam-epoch 0.9.21`、`midly 0.5.3`、`hound 3.5.1`。
`docs/ledger/dependency-licenses.md` 已按 `DEV_WORKFLOW §1「生成物冲突」` 在本分支重新生成
（`--check` 通过，642 行），集成者合并后应再统一重生成一次。

## 4. 判据清单（并明确哪些本机跑过）

共 **64 条纯计算判据**在本机执行（`rustc --edition 2024 --test`，见 §5）+
**约 60 条 cargo 目标内判据**（`render.rs` / `midi.rs` / `wav.rs` / `rng.rs` /
`lib.rs`，**交给 CI**）。核心判据：

| # | 判据 | 位置 | 本机？ |
| :--- | :--- | :--- | :--- |
| 1 | 同一输入 + 1/2/4/8 线程 ⇒ SHA-256 位级摘要相同 | `render::tests::output_is_bit_identical_across_thread_counts` | ❌ CI（需 rayon） |
| 2 | 同一输入 + 任意"轨道产生/完成顺序" ⇒ 母带字节逐位相同 | `verify::scaffold_tests::master_bytes_are_bit_identical_across_completion_orders` | ✅ 本机 |
| 3 | Master 的归约顺序 = `EntityId` 字典序，且**不是**边身份顺序 | `render::tests::master_reduction_order_is_entity_id_lexicographic` | ❌ CI |
| 4 | 归约核与共享 DSP 原语 `yeban_dsp::block::accumulate` 逐位等价 | `contract_tests::sum_kernel_matches_the_shared_dsp_primitive_bit_for_bit` | ❌ CI |
| 5 | 按完成顺序归约会给出**不同**位型（判据 1/2 的敏感度自证） | `sum::tests::unsorted_reduction_is_detectably_different` + `render::tests::completion_order_reduction_is_detectably_different` | ✅ 本机（sum 侧） |
| 6 | `fixed_order` 拒绝重复键（重复键会让结果重新依赖输入顺序） | `sum::tests::fixed_order_rejects_duplicate_keys` | ✅ 本机 |
| 7 | TPDF 严格零均值；且"求和版"会被同一条容差拒绝 | `dither::tests::no_dc_offset` + `one_sided_rpdf_dither_would_be_rejected` | ✅ 本机 |
| 8 | 量化输出恒在合法闭区间；`+1.0` 映射到 `2^(bits-1)-1` 不绕回 | `dither::tests::output_stays_inside_legal_range` + `positive_full_scale_does_not_wrap_around` | ✅ 本机 |
| 9 | 32f 路径**逐位透传**且**不消耗随机源** | `dither::tests::float32_path_is_bit_transparent` + `rng::tests::float_path_does_not_consume_the_rng` | 前者 ✅ / 后者 ❌ CI |
| 10 | RF64 头部字段与 chunk 顺序（`ds64`→`fmt `→`bext`→`data`）逐字段正确 | `rf64::tests::rf64_header_fields_and_chunk_order_are_correct` + `bext_sits_between_fmt_and_data_and_carries_the_ulid` | ✅ 本机 |
| 11 | 超过 4 GiB 走 RF64（**假想尺寸**参数化，不写 4 GiB） | `rf64::tests::payload_beyond_four_gib_switches_to_rf64` + `four_gib_boundary_is_exact` | ✅ 本机 |
| 12 | 三种容器都能被**自己的**读取器读回；`ds64` 声明超长即报 `Truncated` | `rf64::tests::every_container_kind_round_trips_through_our_reader` + `reader_refuses_a_file_shorter_than_its_ds64_declaration` | ✅ 本机 |
| 13 | `bext` v1/v2 往返；固定前缀恒为 **602** 字节；v0 被拒绝 | `rf64::tests::bext_version_two_round_trips` / `bext_version_one_has_602_byte_prefix` / `bext_version_zero_is_refused` | ✅ 本机 |
| 14 | 奇数负载补位不计入 chunk 长度、但计入 `riffSize` | `rf64::tests::odd_payload_gets_a_pad_byte_not_counted_in_the_chunk_size` + `odd_bext_coding_history_is_padded_consistently` | ✅ 本机 |
| 15 | `fmt ` 在 >2 声道时切到 40 字节 `WAVE_FORMAT_EXTENSIBLE` | `rf64::tests::fmt_chunk_switches_to_extensible_for_multichannel` | ✅ 本机 |
| 16 | `hound`（**独立第三方**）能读回我们写出的 24-bit 与 EXTENSIBLE 文件 | `wav::tests::hound_reads_the_wav_written_by_our_rf64_module` + `hound_accepts_our_extensible_six_channel_header` | ❌ CI |
| 17 | 我们的读取器能解析 `hound` 写出的 WAV，且 `fmt `/`data` 长度正确 | `wav::tests::our_reader_parses_the_wav_written_by_hound` | ❌ CI |
| 18 | SMF 0/1 导出 → 回读 = 同一音符集合 | `midi::tests::parallel_export_round_trips_the_same_note_set` + `single_track_export_merges_channels_into_one_chunk` | ❌ CI |
| 19 | **VLQ 边界**：`tick = 0x0FFF_FFFF` 必须写成 `FF FF FF 7F`，且用本 crate 独立解码器读出同值；`MThd` 的大端格式号/轨道数/PPQ 逐字段核对；轨道以 `FF 2F 00` 收尾 | `midi::tests::vlq_boundary_tick_is_encoded_as_four_bytes` | ❌ CI（编解码器的**逻辑** ✅ 本机） |
| 20 | 超过 28 位的 delta 被拒绝而不是截断 | `midi::tests::delta_beyond_vlq_limit_is_rejected` + `vlq::tests::decode_rejects_five_continuation_bytes` | 前者 ❌ CI / 后者 ✅ 本机 |
| 21 | 回读是严格的：未闭合音符与孤立 `NoteOff` 都报错 | `midi::tests::parser_rejects_unbalanced_notes` | ❌ CI |
| 22 | 端到端：渲染 → 抖动 → RF64+BEXT 落盘 → 读回，且 1/2/4 线程产出的**文件字节**逐位相同 | `contract_tests::full_lint_to_master_chain_is_thread_count_invariant` | ❌ CI |
| 23 | 延迟线语义正确（延迟恰好 N 帧、多声道不串台、**块切分不变**） | `pdc::tests::delay_line_*` | ✅ 本机 |
| 24 | PDC：短支路被补 `L_max`，长支路不补；每条入边到达 Master 的时刻都等于 `L_max` | `pdc::tests::every_branch_arrives_at_master_at_exactly_l_max` + `render::tests::pdc_compensation_delays_the_short_branch` | 前者 ✅ / 后者 ❌ CI |
| 25 | `track_latencies` 累加未旁通设备的 `latency_samples`、跳过旁通设备；`0` 按"未上报"处理 | `render::tests::track_latencies_sum_unbypassed_devices_only` | ❌ CI |

## 5. 本机做了什么 / 没做什么

**做了（零重依赖编译）**：

| 命令 | 结果 |
| :--- | :--- |
| `rustc --edition 2024 --test -D warnings -W missing_docs crates/yeban-render/verify/pure_modules.rs` | 通过；**64 条测试全绿，0 告警**。覆盖 `dither.rs` / `pdc.rs` / `rf64.rs` / `sum.rs` / `vlq.rs` 五个**零第三方依赖**模块 + 脚手架里的端到端镜像判据 |
| `bash scripts/dev/cargo-local.sh fmt --all` | 通过（同时证明全部 `.rs` 可被解析器解析 —— 账本 L1） |
| `bash scripts/gates/run-gates.sh light` | **通过**：`fmt --all --check` + 12 条红线守卫 + 文档链接 + 许可清单漂移 |
| `bash scripts/dev/cargo-local.sh metadata --locked --format-version 1` | 通过（`Cargo.lock` 与全部清单一致） |
| `python3 scripts/gates/license_inventory.py` + `--check` | 重新生成后 `--check` 通过（642 行） |
| `rustc --edition 2024 -O /tmp/find_nonassoc.rs` | 一次性搜索脚本：**实测**出顺序敏感的 f32 样本集，用来替代"猜一组数值"（见 §6） |
| `clippy-driver --edition 2024 --test -D warnings -D clippy::all -D clippy::dbg_macro -D clippy::undocumented_unsafe_blocks -D rust_2018_idioms crates/yeban-render/verify/pure_modules.rs` | **通过，0 告警**。这是**工作区 `[lints]` 的逐条等价集合**，因此那 5 个零依赖模块的 clippy 结果与 CI 同源（比只用 `rustc -D warnings` 强得多） |

**没做（纪律要求，一律交给 CI）**：

- `cargo clippy -p yeban-render` / `cargo test -p yeban-render`：本 crate 含 `rayon`
  （重依赖），`run-gates.sh crate yeban-render` 会**自动跳过**本机编译。
  `render.rs` / `midi.rs` / `wav.rs` / `rng.rs` / `lib.rs` 的类型检查与测试**从未在本机执行过**。
- 任何 `--workspace` 全量构建、benchmark、fuzz。
- [BASELINE-001] 的 "≥ 100× 实时" 与 `MUST-GATE-002` 的 L1 基准哈希：需要 `criterion`
  与锁定 ISA 的固定频率机器，本机不做。

### 5.1 本机静态审读在 CI-only 文件里抓出的 4 个缺陷

`render.rs` 本机不编译，因此这 4 条是**审读**（不是执行）发现的，如实登记：

| # | 缺陷 | 为什么本机没抓到 |
| :--- | :--- | :--- |
| S1 | 早先一次"先切头再拼回"的补丁流程把已改好的头部**覆盖**回未打补丁的版本，于是 `render.rs` 的实现里没有 `compile_with_latencies`/`UnknownNode`，而它的判据在调用这两个名字 —— 必然编译失败 | 该文件不在本机编译范围内；`cargo fmt` 只要求语法合法 |
| S2 | `for edge in &edges` 里 `edges: Vec<&RoutingEdge>`，`push(edge)` 实际推入 `&&RoutingEdge` —— 类型不匹配 | 同上 |
| S3 | `AudioSource` 只要求 `Send`，但层内并行会把"已完成节点"的只读视图（含源对象）跨线程共享，`&[NodeState]: Send` 要求 `NodeState: Sync`，进而要求 `dyn AudioSource: Sync` —— 缺一个 supertrait 就是 E0277 | 同上 |
| S4 | `match &reference { None => reference = Some(..), .. }` 与外层共享借用冲突；已改为 `Option::replace` 一步完成"写入 + 取旧值" | 同上 |

**结论（也是本线最重要的过程教训）**：`cargo fmt` 只能证明"语法合法"，
它**不能**替代类型检查。本机无法编译的文件，必须假定它"可能不编译"，
并把这一条写进报告 —— 而不是因为 `fmt` 绿了就当作通过。

## 6. 变异测试证据（SKILL 规则 3：从没红过的判据是注释）

变异在 `/tmp/yr-mut/` 的独立副本里做，不动仓库。每一条都先脚本断言"锚点恰好命中 1 次"
（否则就是账本 L3 的"注入没生效"），再"改坏 → 确认红 → 丢弃副本"。

| # | 变异 | 结果 |
| :--- | :--- | :--- |
| M1 | **把归约改成按"完成/产生顺序"**（脚手架里跳过 `fixed_order`，直接用输入顺序） | **红 ✓** — 只有 `master_bytes_are_bit_identical_across_completion_orders` FAILED，其余 29 条仍绿。这正是任务要求的注入：**改成完成顺序 ⇒ 确定性属性测试必须红** |
| M2 | TPDF 由"两个均匀量之差"改成"之和"（注入 +1 LSB 直流） | **红 ✓** — `no_dc_offset`、`constant_input_is_unbiased`、`positive_full_scale_does_not_wrap_around`、`tpdf_is_bounded_by_one_lsb` 四条 FAILED |
| M3 | 24-bit 上界从 `2^23-1` 改成 `2^23`（差一） | **红 ✓** — `positive_full_scale_does_not_wrap_around`、`depth_reporting_is_consistent`、`public_surface_constants_are_self_consistent` 三条 FAILED |
| M4 | VLQ 按 8 位分组而非 7 位 | **红 ✓** — 5 条 FAILED（含 `vlq_boundary_matches_the_constant_used_by_the_midi_export`） |
| M5 | 抖动加到**四舍五入之后**（而不是之前） | **红 ✓** — `constant_input_is_unbiased` FAILED |
| M6 | `fixed_order` 的排序退化为不排序（稳定排序 + `Ordering::Equal`） | **红 ✓** — 4 条 FAILED（含端到端镜像判据） |
| M7 | 去掉 `fixed_order` 的唯一性断言 | **红 ✓** — `fixed_order_rejects_duplicate_keys` FAILED |
| M1′ | 第一次的 M1 写法（直接删掉 `sort_by` 那行） | **编译失败，不计入** — `-D warnings` 报了 `unused_mut`。按账本 L3，这属于"注入无效"，因此重做成 M1 再判定 |

**变异测试同时抓出了三个真实缺陷**（本地验证的价值）：

1. `fixed_order` 原设计允许重复键，而稳定排序会让结果重新依赖输入顺序 —— **正是
   `ARCH-DET-002` 要消灭的不确定性**。改为 `assert!` 强制键唯一，并要求调用方用
   复合键 `(source_node, edge_id)`。这条断言刻意用 `assert!` 而非 `debug_assert!`：
   确定性契约不允许在 release 构建里静默退化。
2. `bext` v2 的固定前缀长度我一度写成 612。重算 256+32+32+10+8+8+2+64+10+180 = **602**：
   v2 是把 190 字节保留区缩为 180 字节再插入 10 字节响度字段，**总长不变**。这与
   ExifTool 的 `CodingHistory`@602（对 v1/v2 恒定）和检索片段 "180 bytes reserved for
   extension" 两条独立事实一致。
3. `riffSize` 漏算了 `data` 负载的**偶数补位字节**，导致奇数负载的 RIFF 文件长度字段
   与实际文件差 1。已由 `odd_payload_gets_a_pad_byte_not_counted_in_the_chunk_size`
   钉住（chunk 长度**不含**补位、`riffSize` **含**补位）。

## 7. 边界：这次**没有**证明什么

- 本机的 64 条判据**不覆盖**任何依赖 `rayon` / `hound` / `midly` / `yeban-model` 的代码。
  `render.rs` 的跨线程不变性、`midi.rs` 与 `midly` 的字节对账、`wav.rs` 与 `hound`
  的互操作，**全部只由 CI 判定**。
- 没有接入真实合成器、效果链、侧链键控；样本源是注入的 trait（`render::AudioSource`）。
- 所有节点缓冲共用同一声道数；没有 per-node 声道布局。
- BW64 只产出 "`BW64` 标识 + `ds64` + `fmt ` + `bext`" 子集，没有 `axml`/`bxml`/`sxml`/`chna`。
- `bext` 只支持版本 1/2 的读写；版本 0 被显式拒绝（字段表未核验）。
- 多声道只用 `WAVE_FORMAT_EXTENSIBLE`；未实现 `levl`(Peak Envelope)、`iXML`、`fact`、
  `cue `/`r64m` 等 BWF 补充 chunk。
- 未做 [BASELINE-001] 性能基准 → **"≥ 100× 实时" 未被证实**。
- 渲染失败时并行收集的是"某一个"错误，不承诺是哪一个（成功路径的确定性不受影响）。
- 侧链边按普通音频边处理，侧链键控语义未实现。

## 8. needs / pending（含与 `yeban-engine` 的算法复用待办）

| 类型 | 条目 | 说明 |
| :--- | :--- | :--- |
| **needs（阻塞式待办）** | **PDC 算法必须改为复用 `yeban-engine`** | `crates/yeban-engine/src/lib.rs` 在本分支上仍是 scaffold、没有任何 PDC API。`src/pdc.rs` 因此是一份**最小同构实现**（纯函数、零 cpal 依赖、零第三方依赖）。引擎线提供接口后应整体退役本模块的 `plan()`，改为调用 `yeban_engine::pdc`，并由 `render.rs` 的等价性判据防止两条实现漂移 |
| ~~needs~~ **已解决**（集成者 `8f40290`） | `yeban-model::DeviceDefinition::latency_samples` | 本线报告后，集成者已按 `ARCH-PDC-001` 在 main 补上 `latency_samples: u32`（`#[serde(default)]`，**0 = "未上报"**）。本线随即 rebase 并新增 [`render::track_latencies`] 从 `TrackV3` 的设备链累加该值（跳过 `bypassed` 设备），注入参数保留为显式输入 + 覆盖入口。语义（含"0 是未上报而不是零延迟"）写在函数文档与判据 25 里 |
| **needs** | `bext` v2 响度"未知"哨兵的权威定义 | EBU Tech 3285 PDF 本机不可机读（§1.2）。当前取 `Loudness::UNKNOWN = i16::MIN`。**需要人类按 EBU Tech 3285 s5 附录裁决** |
| **needs** | 工程 ULID 在 BWF 里的规范落点 | `bext` 没有 ULID 字段；当前写进 32 字节的 `OriginatorReference`。更规范的落点是 BS.2088 的 `axml` 里的 `<ULID>`；若后续实现 `axml`，应两者都写以保持向后兼容。**需人类裁决** |
| **needs** | 与 `yeban-engine` 的样本源接口对齐 | 目前是 `AudioSource` trait（本 crate 定义）。engine 线落地后应对齐/复用它的渲染图接口，避免两套 trait |
| pending | `midi` 未导出拍号变更链、调号、滑音、弯音曲线、连击、触发概率 < 1.0、歌词/音素 | `micro_timing_ticks` 已并入起始 tick；其余登记为后续切片 |
| pending | `bext` v0 的读取 | 字段表未核验，读到即报 `UnsupportedBextVersion(0)` |
| pending | [BASELINE-001] "≥ 100× 实时" 基准 | 需要 `criterion` + 基准机；本线未做 |
| pending | 真实工程的 32 轨参考工程 A 端到端渲染 | 本线的端到端判据用 **32 轨星形图 + 常量源**，不是真实乐器链 |

## 9. CI 判决

### 第 1 轮 —— run [37221419918](https://github.com/gradetwo/yeban/actions/runs/37221419918)：**红**（2 个 job）

| job | 结论 | 原因 |
| :--- | :--- | :--- |
| `plan` / `checks` / `lockfile` | ✅ 通过 | 受影响集合正确推导为只含 `yeban-render` |
| `rust (yeban-render)` | ❌ **clippy 阶段失败**（`test` 未执行） | 2 条 `clippy::all`：`midi.rs:547` `type_complexity`（`Result<Vec<([u8;4], Range<usize>)>, _>`）、`render.rs:400` `collapsible_if`。**本机无法复现**——这两个文件不在本机编译范围内 |
| `deny (cargo-deny)` | ❌ `bans FAILED` + `licenses FAILED` | 见下；**两条都在本线地盘之外** |

### 阻断项 A（licenses）：`midly` 是 `Unlicense`，不在 `deny.toml` 白名单里

```
error[rejected]: failed to satisfy license requirements
  └─ midly-0.5.3/Cargo.toml:34  license = "Unlicense"
     rejected: license is not explicitly allowed
     Unlicense  -  OSI approved, FSF Free/Libre
```

`deny.toml` 的 `[licenses] allow` 是一份**经过人工逐条裁决**的清单（`BSL-1.0` 那条还附了
"谁引入的、在哪些目标上编译、为什么不属于红线 2 的三类"的论证）。`Unlicense` 是
OSI + FSF Free/Libre + 极度宽松 + 与 GPLv3 兼容，按同一标准**几乎肯定应当被允许**，
但"往许可白名单里加一项"是**法律政策决定**，按 `AGENTS.md` §2 红线 1/2 与本线
"不得私自改写、规范缺口交人类裁决"的纪律，**不由本线自行添加**。

→ **需要集成者/人类二选一**：
1. 在 `deny.toml` 的 `[licenses] allow` 里加 `"Unlicense"`（附一句论证，与 `BSL-1.0` 同格式）；或
2. 本线把 `midi.rs` 改为**自研 SMF 0/1 编解码器**（`vlq.rs` 已在，编码器约 80 行、
   解码器约 100 行），从而彻底不依赖 `midly` —— 顺带让 `midi.rs` 变成零第三方依赖、
   可以进本机脚手架（本机可验证的判据从 64 条涨到 90+ 条）。
   代价：偏离任务书里"SMF 0/1 导出（`midly`）"的指示，且放弃一个久经考验的上游实现。

本线倾向 1（改动最小、保留成熟实现）；若人类希望减少依赖面则选 2，我可以在一轮内做完。

### 阻断项 B（bans/wildcards）：workspace 内部 **path 依赖没有 `version`** ⇒ cargo-deny 视为 `*`

```
error[wildcard]: found 4 wildcard dependencies for crate 'yeban-render'
  crates/yeban-render/Cargo.toml:17  yeban-model.workspace = true   ━━ wildcard dependency
  crates/yeban-render/Cargo.toml:20  yeban-dsp.workspace = true     ━━ wildcard dependency
```

诊断（已在本机用 `grep` 证实）：`yeban-render` 是**本仓库第一个依赖其它 workspace
crate 的成员**（`grep -rn 'yeban-model\.workspace' crates/*/Cargo.toml` 只命中本 crate；
`spikes/spike-04` 里那行还是注释）。根清单的内部 path 依赖写作
`yeban-model = { path = "crates/yeban-model" }` —— **没有 `version`**，于是
workspace 继承出来的依赖要求是 `*`，被 `[bans] wildcards = "deny"` 拦下。
registry 依赖不受影响（根清单给它们写了版本），所以这个坑此前从未暴露。

→ **需要集成者二选一**（两处都在本线地盘之外：根 `Cargo.toml` 是明令禁改的，
`deny.toml` 是根级共享政策文件）：
1. 根 `[workspace.dependencies]` 给内部 path 依赖补版本：
   `yeban-model = { path = "crates/yeban-model", version = "0.0.1" }`（其余内部 crate 同理）；或
2. `deny.toml` 的 `[bans]` 加 `allow-wildcard-paths = true`
   （cargo-deny 为"内部 path 依赖"提供的标准豁免；它只放行**带 path** 的 `*`）。

**提醒：这不是本线独有的问题**——任何做跨 crate 依赖的并行线（engine 依赖 dsp、
ui 依赖 model…）都会撞上同一堵墙。建议在合并本线之前先修主线，否则会连续阻塞多条线。

### 第 2 轮 —— run [37221658083](https://github.com/gradetwo/yeban/actions/runs/37221658083)：**红**

| job | 结论 | 原因 |
| :--- | :--- | :--- |
| `plan` / `checks` / `lockfile` | ✅ 通过 | 新计划器工作正常（`Cargo.lock` 是 ROOT_TRIGGER ⇒ 单条 `rust (workspace 全量)` 腿） |
| `rust (workspace 全量)` | ❌ clippy 阶段 **6 个类型错误** | 见下 —— **两条 clippy lint 已消掉**，这次是真正的类型错误 |
| `deny` | ❌ 与第 1 轮完全相同 | 阻断项 A/B 未变（仍在地盘之外，见上） |

第 2 轮的 6 个类型错误（全部在 `render.rs` 的**判据代码**里，全部只能由编译器发现）：

| # | 错误 | 修复 |
| :--- | :--- | :--- |
| T1/T2 | `E0308`：`sum::accumulate_into(&[0.5], &mut expected, 1.0)` 里 `expected` 是 `f32`，函数要 `&mut [f32]` | 改成 `let mut expected = [0.0f32; 1]`，断言用 `expected[0]` |
| T3–T6 | `E0369`：`assert_eq!(RenderPlan::compile(...), Err(...))` —— `RenderPlan` 只有手写的 `Debug`，**没有 `PartialEq`**（它含 `Box<dyn AudioSource>`，无法 derive） | 比错误值：`.err()` → `Option<RenderError>`（既有 `PartialEq` 又有 `Debug`） |

**教训（已并入本文件 §5.1 的结论）**：`assert_eq!` 要求两侧都 `PartialEq`；
对"含 trait object、只能手写 `Debug`"的类型，比 `Result` 必须走 `.err()`/`matches!`。

### 第 3 轮 —— run [37221854177](https://github.com/gradetwo/yeban/actions/runs/37221854177)：**只差 1 条 lint**

- `plan` 这次正确推出**只含 `yeban-render`**（集成者的计划器改动生效）；
- `rust (yeban-render)`：第 2 轮那 6 个类型错误**全部消失**，整 crate（含 `render.rs` /
  `midi.rs` / `wav.rs` / `rng.rs` / `lib.rs`）**类型检查通过**，只剩 1 条 lint：
  `lib.rs:267` `clippy::manual_is_multiple_of`（`x % n == 0` → `x.is_multiple_of(n)`）；
- `deny`：与第 1/2 轮完全相同（阻断项 A/B 未变）。

### 第 4 轮（单条一行修复）—— 本轮已修（待推送后读回）

- 只有 `lib.rs:267` 的 `manual_is_multiple_of` 一条；已按 clippy 的建议改用
  `usize::is_multiple_of`（Rust 1.87 起稳定，本工作区钉 1.99）。
- 这一轮**只有一行**：第 3 轮已经把五个 CI-only 文件全部类型检查过了，
  因此这是"最后一次已知红的收尾"，不是新一轮猜测。

### 第 3 轮及以前的修复清单

- T1–T6（上表）；
- 主动消掉同类 lint 风险：`bus_reduction_order` 的 `Option<Vec<(EntityId, EntityId, u32)>>`
  换成具名 `pub struct BusInput { source_node, edge_id, delay_frames }`
  （与 `TrackChunk` 同一个 `clippy::type_complexity` 形状）；
- `midi.rs` 用 `lanes.into_iter().unzip()` 取名字，去掉 `|(name, _)| name.clone()`
  （同时避开 `clippy::map_clone` 的可能命中）；
- `wav.rs` 的位置格式参数全部内联（`format!("{actual_bits} 位, {actual_kind}")`）。

## 10. 修改文件的绝对路径清单

```text
/Users/crow/work/music/yeban/.worktrees/render-master/crates/yeban-render/Cargo.toml
/Users/crow/work/music/yeban/.worktrees/render-master/crates/yeban-render/src/lib.rs
/Users/crow/work/music/yeban/.worktrees/render-master/crates/yeban-render/src/render.rs
/Users/crow/work/music/yeban/.worktrees/render-master/crates/yeban-render/src/pdc.rs
/Users/crow/work/music/yeban/.worktrees/render-master/crates/yeban-render/src/rf64.rs
/Users/crow/work/music/yeban/.worktrees/render-master/crates/yeban-render/src/dither.rs
/Users/crow/work/music/yeban/.worktrees/render-master/crates/yeban-render/src/wav.rs
/Users/crow/work/music/yeban/.worktrees/render-master/crates/yeban-render/src/midi.rs
/Users/crow/work/music/yeban/.worktrees/render-master/crates/yeban-render/src/vlq.rs
/Users/crow/work/music/yeban/.worktrees/render-master/crates/yeban-render/src/sum.rs
/Users/crow/work/music/yeban/.worktrees/render-master/crates/yeban-render/src/rng.rs
/Users/crow/work/music/yeban/.worktrees/render-master/crates/yeban-render/verify/pure_modules.rs
/Users/crow/work/music/yeban/.worktrees/render-master/docs/ledger/render-master-notes.md
/Users/crow/work/music/yeban/.worktrees/render-master/Cargo.lock                                  (重新生成)
/Users/crow/work/music/yeban/.worktrees/render-master/docs/ledger/dependency-licenses.md          (重新生成)
```
