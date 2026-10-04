# decode-core 工作线笔记（口径 / 来源 / 缺口）

> 工作线: `line/decode-core` · 工作树 `.worktrees/decode-core` · 拥有目录 `crates/yeban-decode/**`
> 规范 ID: **ARCH-TOP-002**（解码属于后台任务池，不属于实时回调线程）、**ARCH-RT-001**（实时零分配，
> 本线是它的"反面边界"）、**ARCH-DSP-002**（高精多相重采样）、**ARCH-DET-001**（分级确定性）、
> **ARCH-SEC-003**（不可信输入的尺寸/资源防御）、**MODEL-AST-007**（CAS 内容寻址）、
> **ROAD-M-1-004**（只保留 SFZ v2 + 标准 PCM WAV/FLAC，剔除 NKI/EXS24/RVC）、**MUST-GATE-011**（解析零崩溃）。
> 本文件是 `docs/ledger/` 下的**工作线附件**，不是 Normative 规范；与规范冲突时以规范为准。

---

## 1. 交付物与规范 ID 映射

| 文件 | 内容 | 规范 ID |
| :--- | :--- | :--- |
| [`crates/yeban-decode/src/lib.rs`](../../crates/yeban-decode/src/lib.rs) | 模块地图 + "离线/后台、不可变资产、绝不进实时回调"的边界声明 | ARCH-TOP-002, ARCH-RT-001 |
| [`crates/yeban-decode/src/error.rs`](../../crates/yeban-decode/src/error.rs) | 统一 `DecodeError` + symphonia 错误逐条映射 | MUST-GATE-011 |
| [`crates/yeban-decode/src/limits.rs`](../../crates/yeban-decode/src/limits.rs) | **零依赖**：尺寸预算、`frames × channels` 算术、重采样长度契约 | ARCH-SEC-003, ARCH-DSP-002 |
| [`crates/yeban-decode/src/duration.rs`](../../crates/yeban-decode/src/duration.rs) | **零依赖**：容器声明帧数 ↔ 解出帧数对账 | ARCH-DET-001 |
| [`crates/yeban-decode/src/decode.rs`](../../crates/yeban-decode/src/decode.rs) | 路径 / 内存 / `Read + Seek` → `DecodedAsset` | ARCH-TOP-002, ROAD-M-1-004 |
| [`crates/yeban-decode/src/asset.rs`](../../crates/yeban-decode/src/asset.rs) | `PcmFormat`、`DecodeFacts`、`DecodedAsset`、CAS 索引、`pcm_hash` | MODEL-AST-007, ARCH-DET-001 |
| [`crates/yeban-decode/src/resample.rs`](../../crates/yeban-decode/src/resample.rs) | `rubato` sinc（多相）重采样封装 + 长度/延迟契约 | ARCH-DSP-002, ARCH-DET-001 |
| [`crates/yeban-decode/src/testfix.rs`](../../crates/yeban-decode/src/testfix.rs) | **零依赖**：用代码生成最小 WAV / FLAC 字节（仓库不放音频文件） | 红线 9 |
| [`crates/yeban-decode/src/propcheck.rs`](../../crates/yeban-decode/src/propcheck.rs) | `proptest` 属性测试（纯逻辑层） | AGENTS.md §3 DoD 2 |
| [`crates/yeban-decode/Cargo.toml`](../../crates/yeban-decode/Cargo.toml) | 依赖与 **feature 决策**（逐条注释） | ADR-0001 D5/D20/D21 |

`lib.rs` 显式 `#![forbid(unsafe_code)]`：AGENTS.md §2 红线 8 点名的四个 crate 里没有 `yeban-decode`，
但本实现**全程不需要 `unsafe`**，所以取"更好的状态"主动加严。

---

## 2. 已核验的 symphonia / rubato API 与出处

**纪律**：任务书明确要求"不要凭记忆写这两个 crate 的 API"。两个 crate 的 0.6.1 / 5.0.1 都是
**大版本 API 断裂**（symphonia 0.5 的 `Decoder`/`SignalSpec`/`AudioBufferRef` 在 0.6 全部改名或重构；
rubato 的 `SincFixedIn`/`FftFixedIn`/`FastFixedIn` 在 1.0 起就**不存在了**）。
因此下面每条都经过**两轮独立核验**：(1) docs.rs 官方文档 + 官方示例源码；(2) `cargo metadata`
解析成功后，直接读本地 registry 里的**真实源码**逐条比对。

### 2.1 symphonia 0.6.1

| 事实 | 签名 / 取值 | 出处 |
| :--- | :--- | :--- |
| 探测器 | `pub fn get_probe() -> &'static Probe` | <https://docs.rs/symphonia/0.6.1/symphonia/default/fn.get_probe.html> |
| 探测 | `Probe::probe<'s>(&self, hint: &Hint, mss: MediaSourceStream<'s>, fmt_opts: FormatOptions, meta_opts: MetadataOptions) -> Result<Box<dyn FormatReader + 's>>` | <https://docs.rs/symphonia-core/0.6.1/symphonia_core/formats/probe/struct.Probe.html> |
| 提示 | `Hint::new()`；`Hint::with_extension(&mut self, extension: &str) -> &mut Self`（**不是** builder；无 `#[must_use]`） | 源码 `symphonia-core-0.6.1/src/formats/probe.rs:267-276` |
| 解封装 | `FormatReader::next_packet(&mut self) -> Result<Option<Packet>>`（**0.6 改了**：EOF 是 `Ok(None)`） | <https://docs.rs/symphonia-core/0.6.1/symphonia_core/formats/trait.FormatReader.html> |
| 轨道 | `FormatReader::default_track(&self, TrackType) -> Option<&Track>` | 同上 |
| 轨道字段 | `Track { id: u32, codec_params: Option<CodecParameters>, time_base, num_frames: Option<u64>, duration, start_ts, delay: Option<u32>, padding: Option<u32>, flags, language }` | <https://docs.rs/symphonia-core/0.6.1/symphonia_core/formats/struct.Track.html> |
| 编解码参数 | `CodecParameters::audio(&self) -> Option<&AudioCodecParameters>`；`AudioCodecParameters { sample_rate: Option<u32>, bits_per_sample: Option<u32>, channels: Option<Channels>, … }` | 源码 `symphonia-core-0.6.1/src/codecs/mod.rs:99`；<https://docs.rs/symphonia-core/0.6.1/symphonia_core/codecs/audio/struct.AudioCodecParameters.html> |
| 解码器工厂 | `CodecRegistry::make_audio_decoder(&self, params: &AudioCodecParameters, opts: &AudioDecoderOptions) -> Result<Box<dyn AudioDecoder>>` | 源码 `symphonia-core-0.6.1/src/codecs/registry.rs:330-334` |
| 解码 | `AudioDecoder::decode(&mut self, packet: &Packet) -> Result<GenericAudioBufferRef<'_>>` | 源码 `symphonia-core-0.6.1/src/codecs/audio.rs:279` |
| 音频缓冲 | 类型名是 **`GenericAudioBufferRef<'a>`**（10 个变体 `U8/U16/U24/U32/S8/S16/S24/S32/F32/F64`），**不是** 0.5 的 `AudioBufferRef`；方法 `spec()`、`num_planes()`、`frames()`、`samples_interleaved()`、`copy_to_slice_interleaved<Sout, Dst: AsMut<[Sout]>>(&self, dst: Dst)` | <https://docs.rs/symphonia-core/0.6.1/symphonia_core/audio/enum.GenericAudioBufferRef.html> |
| 媒体源 | `MediaSource: Read + Seek + Send + Sync`，必须实现 `is_seekable()` / `byte_len() -> Option<u64>`；symphonia **只**为 `File` 与 `Cursor<T: AsRef<[u8]> + Send + Sync>` 提供实现（**没有** `Read + Seek` 的通用 blanket impl） | <https://docs.rs/symphonia-core/0.6.1/symphonia_core/io/trait.MediaSource.html>；源码 `symphonia-core-0.6.1/src/io/mod.rs:42-90` |
| 流包装 | `MediaSourceStream::new(source: Box<dyn MediaSource + 's>, options: MediaSourceStreamOptions) -> Self` | 源码 `symphonia-core-0.6.1/src/io/media_source_stream.rs:76` |
| 错误 | `#[non_exhaustive] enum Error { IoError, DecodeError, SeekError, Unsupported, LimitError, ResetRequired }` | <https://docs.rs/symphonia-core/0.6.1/symphonia_core/errors/enum.Error.html> |
| 模块路径（0.6 改了） | `probe` 在 `symphonia_core::formats::probe`（**没有** `symphonia_core::probe`）；`MetadataOptions` 在 `symphonia_core::meta`；`AudioDecoderOptions` 在 `symphonia_core::codecs::audio` | 官方示例源码 <https://docs.rs/symphonia/0.6.1/src/basic_interleaved/basic-interleaved.rs.html> |

**没有**用到的 symphonia API（诚实声明）：`AudioSpec`、`Channels`、`Metadata`/容器标签、
`SeekMode`/`SeekTo`、`FormatInfo`、`CodecParameters` 的 codec id 显示。声道数刻意改用
`GenericAudioBufferRef::num_planes()`、位深改用**缓冲区变体**（见 §4）——只依赖已核验的方法。

### 2.3 读上游源码时发现的**两个真实坑**（都已防御，并且都有判据）

这两条不是假想的健壮性，是逐行读源码时撞到的具体行为：

**(a) 截断的 RIFF/WAVE 会让解封装器"不报错、也不推进" ⇒ 解码线程会永久空转。**
链路上有三个事实叠在一起：

1. RIFF 解封装用**声明的** `data` 块长度算边界：
   `let data_end_pos = data.len.map(|len| data_start_pos + u64::from(len));`
   （`symphonia-format-riff-0.6.1/src/wave/mod.rs:145`），
   然后 `WavReader::next_packet` 把它直接交给公共实现（`wave/mod.rs:230-237`）；
2. 公共 `next_packet` 只按 `data_end_pos - pos` 判断"还剩多少块"：
   `if pos < data_end_pos { (data_end_pos - pos) / block_size } else { 0 }`
   （`common.rs:404-408`）——**它不看真实文件长度**；
3. `read_boxed_slice` 对 EOF 是**截短返回**而不是报错
   （`symphonia-core-0.6.1/src/io/mod.rs:553-584` 的 `safe_read_into_boxed_slice`）。

于是"`data` 头完整、数据体被截断"的 WAV 会：`num_blocks_left > 0` → 读到 0 字节 →
返回**空包** → `reader.pos()` **不动** → 下一轮同样 → 永远转下去。
本 crate 的对策是 [`limits::MAX_IDLE_PACKETS`] + [`limits::IdleGuard`]：解码循环里
**每一条**"这一轮没产出样本"的路径都必须过 `bump_idle`，连续 1024 个空转包即
`DecodeError::Malformed`。闸门的算术住在零依赖层，因此**本机可单独跑红**
（判据 `limits::idle_guard_trips_exactly_at_the_cap_and_resets_on_progress`，
注入 D 已取证），CI 上的 `decode::truncated_wav_is_an_error_not_a_panic` 证明它被接进了循环。

**(b) RIFF 的奇数长度块必须补一个不计入块长度的填充字节。**
`ChunksReader::next` 在 `consumed & 0x1 == 1` 时会**真的去读那一个字节**并把
`consumed += 1`（`symphonia-format-riff-0.6.1/src/common.rs:75-79`）。
8-bit（1 字节/样本）与 24-bit（3 字节/样本）很容易凑出奇数长度，少了填充字节就会在
文件末尾吃一个 `UnexpectedEof`。因此 `testfix::wav_with_declared_len` 按**声明的**长度
补零，并有判据 `testfix::wav_fixture_pads_odd_length_chunks_to_the_next_word_boundary`。

顺带记录一个**没踩上但差点踩上**的坑：`AudioDecoder::decode` 返回的是
`GenericAudioBufferRef`（0.6 改名），而 `symphonia` 的 `fmt` 解析里
`WAVE_FORMAT_IEEE_FLOAT` 只接受 32/64 位（`chunks.rs:175-179`）；
"浮点 WAV 必须有 fact 块"是规范建议而非 symphonia 的要求（`fact` 只用来覆盖帧数，
`wave/mod.rs:158-160`），所以夹具没有 fact 块也能解。

### 2.2 rubato 5.0.1

| 事实 | 签名 / 取值 | 出处 |
| :--- | :--- | :--- |
| 类型名 | 只有 **`Async`（异步，`new_sinc`/`new_poly`）、`Fft`（同步，需 `fft_resampler` feature）、`Slip`**。`SincFixedIn`/`FftFixedIn`/`FastFixedIn` **在 5.0.1 里不存在** | <https://docs.rs/rubato/5.0.1/rubato/index.html> |
| sinc 构造 | `Async::<T>::new_sinc(resample_ratio: f64, max_resample_ratio_relative: f64, parameters: &SincInterpolationParameters, chunk_size: usize, nbr_channels: usize, fixed: FixedAsync) -> Result<Self, ResamplerConstructionError>` | 源码 `rubato-5.0.1/src/asynchro.rs:297-304` |
| 参数 | `SincInterpolationParameters::new(sinc_len: usize, window: WindowFunction) -> Self`（`f_cutoff: None` = 自动推导）、`Default` = `new(256, BlackmanHarris2)` | 源码 `rubato-5.0.1/src/asynchro_sinc.rs:54-84` |
| 枚举 | `FixedAsync::{Input, Output}`；`WindowFunction::{Blackman, Blackman2, BlackmanHarris, BlackmanHarris2, Hann, Hann2}` | 源码 `rubato-5.0.1/src/asynchro.rs:22`、`src/windows.rs:6` |
| 整体重采样 | `Resampler::process_all_into_buffer(&mut self, buffer_in: &dyn Adapter<T>, buffer_out: &mut dyn AdapterMut<T>, input_len: usize, active_channels_mask: Option<&[bool]>) -> ResampleResult<(usize, usize)>` | <https://docs.rs/rubato/5.0.1/rubato/trait.Resampler.html> |
| **返回值语义（关键）** | 返回的第二个数是 `expected_output_len = ceil(resample_ratio() * input_len)`，**不是**"实际写入缓冲的帧数"；缓冲里前 `expected_output_len` 帧才是有效音频 | 源码 `rubato-5.0.1/src/lib.rs:330`、`lib.rs:398` |
| 缓冲尺寸 | `process_all_needed_output_len(&mut self, input_len) -> usize` = `output_delay() + output_frames_max() + ceil(ratio × input_len)` | 源码 `rubato-5.0.1/src/lib.rs:456-461` |
| 延迟 | `output_delay(&self) -> usize`（**输出帧**口径）；`process_all_into_buffer` 内部用 `trim_startup_delay` 把这段前置静音裁掉并前移数据 | 源码 `rubato-5.0.1/src/lib.rs:595-616` |
| 比例校验 | `max_resample_ratio_relative` 必须 `is_finite() && >= 1.0`；`resample_ratio` 必须有限且 `> 0` | 源码 `rubato-5.0.1/src/asynchro.rs:158-172` |
| 适配器 | `rubato` 直接 `pub use audioadapter; pub use audioadapter_buffers;`；`InterleavedSlice::new(buf: &'a [T], channels: usize, frames: usize) -> Result<Self, SizeError>`、`InterleavedSlice::new_mut(buf: &'a mut [T], channels, frames) -> Result<Self, SizeError>` | 源码 `rubato-5.0.1/src/lib.rs:6-7`、`audioadapter-buffers-5.2.0/src/direct.rs:1097/1113` |
| 错误 | `ResamplerConstructionError` / `ResampleError` 均 `#[non_exhaustive]` + `Debug` + `Display` | 源码 `rubato-5.0.1/src/error.rs:82-170` |

**代价与收益**：`Async::new_sinc` 只依赖 `audioadapter` / `audioadapter-buffers` / `num-*` /
`visibility` / `windowfunctions`，这些**都是非可选依赖**，所以"不启用任何 rubato feature"
不需要新增任何直接依赖 —— `InterleavedSlice` 走 `rubato` 的 re-export 拿到，因此
`crates/yeban-decode/Cargo.toml` 里**没有** `audioadapter-buffers`（也就没有 `TODO(hoist)`）。

---

## 3. 格式 / 位深 / 声道矩阵，以及**实际测过什么**

### 3.1 启用的 feature（根 `Cargo.toml` 把 symphonia 钉死为 `default-features = false`）

这一点必须显式说明：根清单的 `default-features = false` 意味着**不在这里列出 feature，
就一个 format/codec 都不会注册**。本线启用：

| 类别 | feature | 理由 |
| :--- | :--- | :--- |
| 容器 | `wav` | `ROAD-M-1-004` 点名"标准 PCM WAV" |
| 容器 | `flac`（codec）+ FLAC native 流 | `ROAD-M-1-004` 点名 FLAC |
| 容器 | `ogg` | 上游默认集里的开放标准容器 |
| 编解码 | `pcm` / `adpcm` / `vorbis` | 上游默认集，全部免专利 |
| 元数据 | `ape` / `id3v1` / `id3v2` | 与上游默认集一致 |

**刻意不启用**：`mkv`（视频容器，会扩大不可信输入解析面）、`aiff` / `caf` / `isomp4`
（上游默认关闭、规范未点名）、`mp3` / `mpa` / `aac` / `alac`（有损或需额外 feature）。
**刻意不启用 `opt-simd-sse` / `opt-simd-avx` / `opt-simd-neon`**：它们会拉入 `rustfft`，
并让解码路径依赖**运行时 CPU 探测** —— 与 ARCH-DET-001 的 L1/L2 目标相悖。
三个 feature 全部属于上游 MPL-2.0 包，无需新白名单条目。

`rubato` 侧：**不开任何 feature**（不开 `fft_resampler`，因此不引入 `realfft`/`rustfft`；
不开 `log`，因为它的文档明确说日志会分配字符串、不适合热路径）。选择 sinc 而非 FFT 的
完整理由写在 `resample.rs` 的模块文档里（规范措辞"多相"、少一层 SIMD 依赖、少一个舍入源）。

### 3.2 支持矩阵与测试状态

| 容器 | 位深 | 声道 | 状态 |
| :--- | :--- | :--- | :--- |
| WAV (RIFF/WAVE) | 8-bit **U8** | 1, 2 | ✅ **构造的字节，实测**（`wav_pcm8_unsigned_is_supported`） |
| WAV | 16-bit **S16** | 1 | ✅ **构造的字节，实测**（含逐样本期望值） |
| WAV | 16-bit **S16** | 2 | ✅ **构造的字节，实测**（含通道交织顺序） |
| WAV | 24-bit **S24** | 1 | ✅ **构造的字节，实测** |
| WAV | 32-bit **S32** | 1 | ✅ **构造的字节，实测** |
| WAV | 32-bit **F32** | 2 | ✅ **构造的字节，实测**（逐样本**精确相等**） |
| FLAC (native) | 16-bit **S16**（CONSTANT 子帧） | 1, 2 | ✅ **构造的字节，实测**（含 CRC-8/CRC-16 自洽） |
| OGG / Vorbis | — | — | ⚠️ **只有代码路径存在**：feature 已启用、探测器能认领，但**没有构造字节、没有测试** |
| WAV ADPCM | — | — | ⚠️ **只有代码路径存在**（feature 已启用，未测） |
| WAV 8/16/24/32-bit 无符号整数（U16/U24/U32） | 任意 | 任意 | ⚠️ 只在 `PcmFormat` 枚举里存在；WAV 的 `fmt` tag 1 只产出 S8/S16/S24/S32，所以**代码路径存在但不可达** |
| F64 | 64-bit | 任意 | ⚠️ 同上（WAV tag 3 + 64-bit 可达，未测） |
| MKV / AIFF / CAF / MP4 / MP3 / AAC / ALAC | — | — | ❌ **feature 未启用**：遇到即 `DecodeError::UnsupportedFormat` |

**所有夹具都是代码生成的字节**（`src/testfix.rs`），仓库里**没有任何音频文件** ——
既满足 AGENTS.md §2 红线 9（>10MB 未登记二进制 / 未登记采样），也让判据能断言**每一个采样点的期望值**，
而不是只能断言"能解出来"。夹具的字节仍然写进 `target/`（`decode_path` 判据用 `std::env::temp_dir()`），
不落库。

### 3.3 重采样矩阵

| 输入率 → 输出率 | 状态 |
| :--- | :--- |
| 48 kHz → 44.1 kHz | ✅ 实测（长度契约 + 直流中段 + 半幅交叉点） |
| 44.1 kHz → 48 kHz | ✅ 实测（长度契约，含 f64 `ceil` 差 1 帧的情况） |
| 48 kHz → 96 kHz | ✅ 实测（长度契约 + 通道顺序） |
| 同率（恒等） | ✅ 实测（逐位相同、零滤波） |
| 96 kHz → 48 kHz | ⚠️ 未单测，但由同一函数 + 同一契约覆盖（`limits` 的 3×3 全矩阵判据覆盖了长度侧） |
| 任意其他率 | 长度侧由 `limits` 的随机属性测试覆盖；音频侧未测 |

---

## 4. 尺寸上限口径（写死并判据钉住）

| 口径 | 值 | 常量 | 依据 |
| :--- | :--- | :--- | :--- |
| 单个输入字节数 | **2 GiB** | `MAX_INPUT_BYTES` | 直接沿用 ARCH-SEC-003 的"单个解压条目 ≤ 2 GB"，同一个数字只裁决一次 |
| 解码后**交织 f32 PCM** 字节数 | **2 GiB** | `MAX_PCM_BYTES` | 换算：(48 kHz 立体声 ≈ 93 分钟) / (96 kHz 立体声 ≈ 46 分钟) / (96 kHz 8 声道 ≈ 11.6 分钟) |
| 交织样本总数 | 2 GiB / 4 = 536 870 912 | `MAX_INTERLEAVED_SAMPLES` | 派生 |
| 声道数 | **64** | `MAX_CHANNELS` | 畸形文件常靠"声明 65535 声道"制造乘法溢出或 OOM |
| 采样率 | **768 000 Hz** | `MAX_SAMPLE_RATE` | DXD 之上再留一倍余量 |

**口径的原则**（三条一起才成立）：
1. **在分配之前检查**。`decode` 在读出第一个解码缓冲之前，先按容器**声明**的帧数做一次廉价
   前置检查；每个 packet 再按累计样本数检查一次；`resample` 在 `process_all_needed_output_len`
   之后、`try_reserve` 之前检查。
2. **用 `Vec::try_reserve` 而不是 `vec![0.0; n]`**。`Vec::push` / `vec!` 在分配失败时**abort 进程**；
   `try_reserve` 会返回 `Err`，于是"分配器说不"变成 `DecodeError::Budget(AllocationRefused)`。
3. **乘法一律 `checked_mul`**（`limits::interleaved_samples`），溢出是
   `LimitViolation::LayoutOverflow` 而不是回绕成小值。

---

## 5. 确定性（ARCH-DET-001）落实清单

| 要求 | 本线落实 | 判据 |
| :--- | :--- | :--- |
| 固定算法与参数 | sinc + `BlackmanHarris2` + `sinc_len=256` + `chunk=1024` + `max_relative_ratio=1.0`，全部 `pub const` | `resample::tests::configuration_is_pinned` |
| 不使用未固定种子的随机 | 全 crate 无 PRNG、不读时钟/环境变量/线程数 | `resample::tests::same_input_resamples_identically_in_two_threads` |
| 浮点超越函数走 `libm` | **不需要 `libm`**：唯一的浮点运算是 1 次 `f64` 除法（时长换算）与 `rubato` 内部乘加。因此本 crate **不依赖 `libm`** | 代码审查 + `Cargo.toml` |
| 同输入 → 同输出 | `DecodedAsset::pcm_hash()`（域前缀 `"yeban.pcm.f32le.v1"` + 声道 u16 + 采样率 u32 + 帧数 u64 + 每个样本 `f32::to_le_bytes()`） | `decode::tests::decoding_the_same_bytes_twice_is_bit_identical`、`asset::tests::pcm_hash_is_sensitive_to_the_last_bit_of_a_sample` |
| 摘要对**位模式**敏感 | `0.0` 与 `-0.0` 值相等但摘要不同（判据显式钉住） | 同上 |
| 不启用解码 SIMD | 见 §3.1（去掉运行时 CPU 探测这一非确定性来源） | `Cargo.toml` feature 列表 |

---

## 6. 延迟 / 预填充语义（ARCH-DSP-002）

`rubato` 的 sinc 重采样器内部有启动延迟。本线**一律**走 `Resampler::process_all_into_buffer`，
它按上游文档裁掉启动静音（源码 `lib.rs:595-616` 的 `trim_startup_delay`）。因此契约是：

- **预填充 = 0 帧**（`resample::PREFILL_FRAMES == 0`）；导出的资产**不带前置静音**；
- **输出帧数 = `ceil(输入帧数 × 输出率 / 输入率)`**（上游返回值语义，见 §2.2），
  并必须落在 `limits::resample_len_contract` 给出的闭区间内；
- **区间口径**：`⌊理想值⌋`（或 `⌈理想值⌉`）± `max(0.1% , 8 帧)`；当理想输出 < 4096 帧时
  再额外放宽 `SINC_LEN`(256) 帧 —— 短片段里"被裁掉的前置延迟"相对整段不再可忽略，
  这不是放水，而是把上游的真实语义写进契约；
- **比例不可调**：`MAX_RELATIVE_RATIO = 1.0`，本模块**不暴露** `set_resample_ratio`。
  一次导入只对应一个固定比例，杜绝"同输入两次导入结果不同"。

**可观测的判据**（用"半幅交叉点位置"而不是"第 0 个样本等于 1.0"）：输入是直流阶跃，
对称滤波器的阶跃响应在被正确对齐时第 0 个输出样本就已经在 0.5 附近；若延迟没被裁掉，
0.5 交叉点会被推到 `output_delay()` 帧之后（sinc_len=256 时远大于判据允许的 4 帧）。
用"第 0 个样本 ≈ 1.0"会写成一条**永远不成立**的假判据 —— 任何有限长滤波器的阶跃响应
都需要 `sinc_len/2` 个输出帧才爬到 1.0。

---

## 7. 本机验证 vs 交给 CI（**严格区分**）

### 7.1 本机**真的跑过**的（只覆盖纯逻辑层）

`limits.rs` / `duration.rs` / `testfix.rs` 三个模块**零第三方依赖**，因此可以把它们
`#[path]` 进一个独立驱动，用 `rustc --edition 2024 --test` 单独编成测试二进制：

```bash
cd .worktrees/decode-core
source scripts/dev/local-env.sh        # 受限沙箱里必须: RUSTUP_TOOLCHAIN=stable
rustc --edition 2024 --test target/local-verify/pure_logic.rs \
      -o target/local-verify/pure_logic && ./target/local-verify/pure_logic
```

驱动文件（**不入库**，在 `target/` 下，被 `.gitignore` 忽略）：

```rust
#[path = "../../crates/yeban-decode/src/limits.rs"]   mod limits;
#[path = "../../crates/yeban-decode/src/duration.rs"] mod duration;
#[path = "../../crates/yeban-decode/src/testfix.rs"]  mod testfix;
```

结果：**27 passed; 0 failed**。另外用第二个驱动 `target/local-verify/lang_patterns.rs`
（最小同构 trait，不链接任何第三方 crate）验证了 4 个语言层面的高风险写法：
`dyn Trait` 方法调用不需要 import trait、`Box<dyn Trait + 's>` 能从 `Cursor<&[u8]>` 构造、
`AsMut<[Sout]>` 的 `Sout` 推断对 `&mut Vec<f32>` 与 `&mut [f32]` 都成立、
`Adapter::new_mut(&mut out, …)` 的 `&mut Vec<T> → &mut [T]` deref 强制转换。
本机也跑了 `bash scripts/gates/run-gates.sh light` → **通过**。

**明确不覆盖**：任何经过 symphonia / rubato 的路径。本机**没有**、也**不允许**编译这两个
重依赖（`run-gates.sh crate yeban-decode` 会因此自动 SKIP）。所以
`decode.rs` / `asset.rs` / `resample.rs` 里的 **28 条集成判据全部只由 CI 执行**，
本机对它们的把握来自 §2 的源码级 API 核验与静态审查，**不是**"本机验证过"。

### 7.2 注入 → 变红 → 还原（四次，全部在本机可执行的那一层）

| # | 注入内容 | 变红的判据 | 还原后 |
| :--- | :--- | :--- | :--- |
| A | `limits::check_layout` 的 `if samples > limit` 改成 `if false && samples > limit`（**放行超预算资产**） | `limits::tests::layout_budget_rejects_an_asset_over_the_pcm_cap` FAILED（21 passed / 1 failed） | 22 passed |
| B | `limits::resample_len_contract` 的 `min` 改成 `floor + slack + 1`（**把理想输出排出区间**，等价于长度/预填充关系写错） | `length_contract_rejects_an_untrimmed_delay`、`length_contract_always_contains_the_ideal_output`、`length_contract_pins_the_three_normative_conversions` 三条 FAILED（19 passed / 3 failed） | 22 passed |
| C | `testfix::crc8` 的多项式 `0x07` 改成 `0x1d`（**夹具本身写错**） | `testfix::tests::crc_algorithms_match_the_published_check_values` FAILED（21 passed / 1 failed） | 22 passed |
| D | `limits::IdleGuard::bump` 的返回值硬改成 `false`（**防挂死闸门永不跳闸**，等价于"忘了接这个计数器"） | `limits::idle_guard_trips_exactly_at_the_cap_and_resets_on_progress` FAILED（23 passed / 1 failed） | 24 passed |

注入 A / B 覆盖的正是任务书点名的两条："把上限判定改成放行 ⇒ 判据红"（A）与
"把重采样预填充改错 ⇒ 长度关系判据红"（B）。C 是为了证明**夹具自身可证伪** —— 如果只让夹具
自洽（写和验用同一个函数），一个错的多项式不会被发现，CI 上的解码失败就会被人误读成
"解码器坏了"。D 针对 §2.3(a) 那个"解封装器不报错也不前进"的真实坑：它的表现是
**CI 挂到 60 分钟超时**而不是一条干净的断言失败，所以必须有一条能在本机跑红的闸门判据。

**诚实边界**：四次注入都发生在纯逻辑层，因为那是本机唯一可执行的层。
CI 专属的集成判据（例如 `the_pcm_budget_is_enforced_before_allocating`）**没有**做注入取证，
它们与注入 A 共享同一个 `limits::check_layout`，因此 A 的红可以视为对该判据的间接取证，
但这不等同于"注入过集成路径"。

### 7.3 CI 第 1 轮的完整读数（run 37225396909 @ `7a3a796`）

判决：`checks`（fmt/守卫/schema）/ `lockfile` / `deny` 全绿；
`rust (workspace 全量)` 红 —— 因为本线改了 `Cargo.lock`，集成者的 `plan` 把这次运行判成
**workspace 全量**，走的是**单条** `rust-workspace` 腿（`clippy --workspace` + `test --workspace`），
不是 23 条矩阵腿。该腿红在 `cargo clippy --workspace --all-targets --locked -- -D warnings`。

**关键信息（比"红了"更重要）**：报错全部落在 `crates/yeban-decode/`，且**只有 lint、没有类型错误**。
clippy 的 lint 在类型检查之后才跑，所以这一轮同时证明了：

- symphonia / rubato 的 feature 组合能解析、能编译；
- 本 crate 的 `lib` 与 `lib test` **类型检查全部通过** —— §2 那两轮 API 核验是对的，
  没有出现任何"名字写错/签名不匹配/借用不成立"的错误。

**逐条（`gh run view 37225396909 --log-failed` 全文抓取，不是只看第一条）**：

| # | lint 家族 | 位置 | clippy 的原文建议 |
| :--- | :--- | :--- | :--- |
| 1 | `clippy::doc_overindented_list_items`（"doc list item overindented"） | `limits.rs:14:5` | `help: try using '  ' (2 spaces)` —— 同一个列表项里后续行缩进必须自洽 |
| 2 | `clippy::manual_is_multiple_of` | `resample.rs:101:8` | ``help: replace with: `!samples.len().is_multiple_of(channels_usize)` `` |
| 3 | `clippy::chunks_exact_to_as_chunks` | `resample.rs:349:37` | ``help: consider using `as_chunks` instead: `as_chunks::<2>().0.iter()` `` |
| 4 | `clippy::chunks_exact_to_as_chunks` | `resample.rs:376:37` | 同上 |

修法：(1) 把该列表项的续行统一到 2 空格（顺带把多行项改写成单行项，从根上不给缩进留自由度）；
(2)(3)(4) 分别改成 `.is_multiple_of(n)` 与**带下标的 `for frame in lo..hi`**（后者既不触发
`chunks_exact_to_as_chunks`，也不会触发 `needless_range_loop`：循环变量不是直接下标，
而是 `frame * 2` / `frame * 2 + 1`）。

**教训（写给后面的线，也写给本线的下一轮）**：本机不能编译重依赖 ⇒ clippy 是**唯一**
必须在 CI 上才能看到的门禁，而它一次只会把已触发的家族报完。所以抓日志要抓**全文**，
不能只 `tail` 或只看第一条：本轮 4 条里 2 条在 `limits.rs`、2 条在 `resample.rs`，
只看第一条会以为改一处就够。

### 7.4 CI 第 2 轮（run 37225684396 @ `bf08abd`，rebase 到 `60424a3` 之后）

判决：`lockfile` ✓；`rust (workspace 全量)` 里 **`clippy --workspace (-D warnings)` 变绿** ✓
（4 条 lint 全部修掉），`test --workspace` 红在**一条**判据上；`checks` 红在**不属于本 crate** 的
"跨语言契约对账"步骤。

**本 crate 的测试读数：`64 passed; 1 failed`（+ 一条 `-p yeban-decode --lib` 失败）。**
唯一失败的判据：

| 判据 | 失败原因 | 结论 |
| :--- | :--- | :--- |
| `asset::tests::imported_asset_exposes_the_decoded_facts` | `panic at testfix.rs:106: i16::try_from(32_768)` —— **夹具数据越界**（i16 上界是 32767），不是解码器的问题 | 夹具 bug，已修 |

修法与加固：
- 把该判据的夹具数据改成 `[16_000, -16_000]`（半量程对称值，仍在 S16 合法区间内）；
- **新增 3 条本机可跑的夹具自检判据**（C45/C46/C47）：把每个位深的合法区间钉住，
  并用 `#[should_panic]` 让"越界值必须在夹具层炸"成为一条显式判据 —— 这样下一次
  同类错误会在**夹具自己的判据**里红，而不是在别的测试的调用栈里留下一个
  `testfix.rs:106` 的 panic（那是这次定位成本的来源）。

**这一轮顺带证明了什么**（比"还差一条"更重要）：`64 passed` 里包含了本线**全部**高风险集成判据 ——
WAV 8/16/24/32-bit 与 f32 的逐样本期望值、FLAC 端到端解码（含 CRC）、
声明/解出帧数不一致必须报错、7 个截断点全部返回错误（**防挂死闸门真的接进了循环**）、
44.1k/48k/96k 三个方向的长度契约、同输入逐位相同、跨线程逐位相同、
CAS 内容寻址与 `pcm_hash` 的位级敏感性。也就是说 §2 的 API 核验与 §5/§6 的判据设计都被 CI 证实了。

**不属于本 crate 的红**：`checks` 的"跨语言契约对账"红在
`target/schema-samples/mcp-tools.error-codes.json` 与 `mcp-tools.registry.json`
对 `schemas/mcp-tools.schema.json` 的违反（`missingFromSchema` 13 个错误码、
`registry.json` 不满足根 `oneOf`）。本线**没有**改 `schemas/**`、`crates/yeban-mcp/**`
或 `.github/**`，该步骤失败来自 `line/mcp-core` 的 D25 承重修复在 `main` 上的残留，
已上报集成者。

---

### 7.5 CI 第 3 轮（run 37226040237 @ `ce5fd95`，其后 notes 改动在 run 37226662719 @ `d6a6ff5` 复现同一读数）—— **本 crate 全绿**

逐腿读数（`scripts/dev/ci-verdict.sh line/decode-core`）：

| 腿 | 结果 | 说明 |
| :--- | :--- | :--- |
| `lockfile (确定性 Cargo.lock)` | ✓ | rebase 后的 `Cargo.lock` 与全部清单一致 |
| `deny (cargo-deny 开源合规)` | ✓ | 新增依赖的许可/advisory/bans 全部通过，**无需扩白名单** |
| `plan (受影响集合)` | ✓ | 因改了 `Cargo.lock`，判定为 workspace 全量 |
| `checks (fmt / 红线守卫 / schema)` | ✗ | **只有**"跨语言契约对账"这一步，内容是 `yeban-mcp` 的 `mcp-tools` 契约，与 `crates/yeban-decode/**` 无关 |
| `rust (workspace 全量)` | ✗ | `clippy --workspace (-D warnings)` **✓ 绿**；`test --workspace` 仅红在 `crates/yeban-mcp/tests/contract.rs` 的 2 条判据 |

**本 crate 的实测结论（从 CI 全文日志里逐行取回）：**

```text
     Running unittests src/lib.rs (target/debug/deps/yeban_decode-5fe1f316180328fd)
     ...
     test result: ok. 68 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.07s
```

即 `cargo test -p yeban-decode --all-targets --locked`：**68 passed / 0 failed**，
且 `cargo clippy --workspace --all-targets --locked -- -D warnings` 对本 crate 零告警。
这 68 条包含了 §8 里全部经过 symphonia / rubato 的判据（C16–C42）。
`d6a6ff5`（只改了本笔记）在 run **37226662719** 上复现同一读数：`68 passed; 0 failed`，
全 workspace 的失败仍然只有 `crates/yeban-mcp/tests/contract.rs` 那 2 条。

**剩下两处红的归因（都不是本 crate）**：`crates/yeban-mcp/tests/contract.rs` 的
`schema_root_references_the_tool_call_definition` 与
`the_error_code_gap_between_the_two_contracts_is_pinned`（"schema enum 是 7 个错误码"），
以及 `checks` 的同源失败（`mcp-tools.error-codes.json` 有 13 个错误码
`missingFromSchema`、`mcp-tools.registry.json` 不满足根 `oneOf`）。
本线未改 `schemas/**`、`crates/yeban-mcp/**`、`.github/**`，已上报集成者。

**"本线绿了吗"的诚实回答**：本 crate 的两道硬门禁（clippy + test）在 CI 上是绿的；
但**整条 run 仍是红的**，因为同一腿里掺了别人的 crate。按本仓库纪律"只有 CI 的判决算数"，
本线**不声称这一轮通过** —— 判定为：`yeban-decode` 的门禁通过，run 级判决为红（原因不在本线）。

---

## 8. 判据清单（任务要求 ≥6，实际远超）

| # | 判据 | 钉住什么 | 执行位置 |
| :--- | :--- | :--- | :--- |
| C1 | `limits::size_caps_are_pinned_to_the_documented_numbers` | 尺寸上限口径（改数字必须先改这里） | 本机 + CI |
| C2 | `limits::input_byte_budget_is_enforced` | 输入字节上限（含边界相等） | 本机 + CI |
| C3 | `limits::layout_budget_rejects_degenerate_declarations` | 0 声道 / 0 采样率 / 超上限声道 / 超上限采样率 | 本机 + CI |
| C4 | `limits::layout_budget_rejects_an_asset_over_the_pcm_cap` | 超预算必须拒绝（**注入 A**） | 本机 + CI |
| C5 | `limits::layout_product_overflow_is_detected_not_wrapped` | `frames × channels` 不回绕 | 本机 + CI |
| C6 | `limits::length_contract_always_contains_the_ideal_output` | 44.1/48/96 全矩阵 9 组合 × 8 长度 | 本机 + CI |
| C7 | `limits::length_contract_pins_the_three_normative_conversions` | 三个规范转换的**精确**期望值 | 本机 + CI |
| C8 | `limits::length_contract_rejects_an_untrimmed_delay` | 预填充/延迟语义（**注入 B**） | 本机 + CI |
| C9 | `limits::short_clips_get_an_explicit_delay_allowance` | 短片段额外放宽的边界 | 本机 + CI |
| C10 | `limits::zero_rate_has_no_length_contract` | 零采样率不产生契约 | 本机 + CI |
| C11 | `duration::*`（5 条） | 声明/解出对账：未知不当零长、精确、容差、越界报错 | 本机 + CI |
| C12 | `testfix::crc_algorithms_match_the_published_check_values` | 夹具 CRC 与标准一致（**注入 C**） | 本机 + CI |
| C13 | `testfix::wav_fixture_is_a_well_formed_44_byte_header_riff` | WAV 夹具每一个头字段 | 本机 + CI |
| C14 | `testfix::flac_fixture_has_a_valid_streaminfo_and_frame_chain` | FLAC 夹具 STREAMINFO 位打包 + 逐帧 CRC | 本机 + CI |
| C15 | `testfix::flac_fixture_records_a_constant_non_zero_dc_level` | CONSTANT 子帧的位布局 | 本机 + CI |
| C16 | `decode::wav_pcm16_mono_decodes_to_expected_samples` | S16 归一化缩放与逐样本值 | CI |
| C17 | `decode::wav_pcm16_stereo_keeps_channel_interleaving` | 交织顺序 + 通道镜像 | CI |
| C18 | `decode::wav_pcm8/24/32/float32_*`（4 条） | 8/24/32-bit 整数与 f32 的 `PcmFormat` 与归一化 | CI |
| C19 | `decode::flac_constant_block_decodes_with_its_declared_length` | FLAC 端到端解码 + 声明帧数 | CI |
| C20 | `decode::flac_declared_total_samples_mismatch_is_reported_and_rejected` | **不一致必须报错**，绝不猜（ARCH-DET-001） | CI |
| C21 | `decode::truncated_wav_is_an_error_not_a_panic` | 7 个截断点全部返回错误 | CI |
| C22 | `decode::arbitrary_garbage_is_an_error_not_a_panic` | 5 类畸形输入零 panic | CI |
| C23 | `decode::the_pcm_budget_is_enforced_before_allocating` | 集成层预算（与注入 A 同源） | CI |
| C24 | `decode::the_input_byte_budget_is_enforced_before_probing` | 探测之前就拒绝 | CI |
| C25 | `decode::decoding_the_same_bytes_twice_is_bit_identical` | 同输入 → 同输出（含 `to_bits()` 比对） | CI |
| C26 | `decode::decode_reader_and_decode_bytes_agree` | 三条入口路径一致 | CI |
| C27 | `decode::decode_reader_restores_the_start_position_of_a_partial_cursor` | `MeasuredSource` 不改动 reader 位置 | CI |
| C28 | `decode::decode_path_matches_decode_bytes` | 路径入口与内存入口一致 | CI |
| C29 | `resample::configuration_is_pinned` | 重采样算法/参数被钉死 | CI |
| C30 | `resample::identity_rate_is_a_bit_exact_passthrough` | 同率恒等 | CI |
| C31 | `resample::resampling_48k_to_44k1 / 48k_to_96k / 44k1_to_48k_*`（3 条） | **输入长度 → 输出长度**契约（ARCH-DSP-002） | CI |
| C32 | `resample::no_leading_pad_is_observable_as_an_immediate_half_level_crossing` | 延迟被裁掉（见 §6） | CI |
| C33 | `resample::channel_order_survives_resampling` | 通道顺序保持 | CI |
| C34 | `resample::same_input_resamples_identically_in_two_threads` | 无全局状态 / 跨线程逐位相同 | CI |
| C35 | `resample::degenerate_arguments_are_rejected_rather_than_guessed` | 0 声道 / 0 采样率 / 超上限率 / 非整倍数交织 | CI |
| C36 | `resample::resample_asset_updates_the_facts_and_the_hash` | 转换后事实与摘要语义 | CI |
| C37 | `asset::cas_index_matches_yeban_model_and_is_content_addressed` | **MODEL-AST-007**：路径留痕不参与寻址 | CI |
| C38 | `asset::pcm_hash_is_sensitive_to_the_last_bit_of_a_sample` | 位级确定性（含 ±0.0） | CI |
| C39 | `asset::pcm_hash_covers_the_layout_header_not_just_the_samples` | 摘要含声道/采样率/帧数 | CI |
| C40 | `asset::model_bit_depth_only_covers_the_normative_enum` | 只有 Int16/Int24/Float32，不发明近似值 | CI |
| C41 | `asset::imported_asset_is_reachable_only_through_shared_reads` | `Send + Sync` + 跨线程只读 | CI |
| C42 | `propcheck::*`（4 条属性测试，256 cases） | 溢出/预算包络/长度契约非空/对账对称 | CI |
| C43 | `limits::idle_guard_trips_exactly_at_the_cap_and_resets_on_progress` | **防挂死闸门**（注入 D；对应 §2.3(a)） | 本机 + CI |
| C44 | `testfix::wav_fixture_pads_odd_length_chunks_to_the_next_word_boundary` | RIFF 奇数长度块的对齐填充（对应 §2.3(b)） | 本机 + CI |
| C45 | `testfix::integer_fixture_range_is_pinned_so_out_of_range_data_fails_loudly_here` | 夹具每个位深的合法区间（CI 第 2 轮踩过的坑） | 本机 + CI |
| C46 | `testfix::sixteen_bit_fixture_rejects_a_value_that_does_not_fit_i16` | 越界值必须在夹具层炸（`#[should_panic]`） | 本机 + CI |
| C47 | `testfix::eight_bit_fixture_rejects_a_value_outside_the_unsigned_offset_range` | 同上，8-bit | 本机 + CI |

对账：共 **47 条**。其中**本机可执行 20 条**（C1–C15、C43–C47）；
**C16–C42 共 27 条只能在 CI 上跑**（它们经过 symphonia / rubato）。

---

## 9. needs / pending / TODO(hoist)

**没有 `TODO(hoist)`**：本线没有引入任何新的 registry 依赖 —— `symphonia` / `rubato` /
`thiserror` / `yeban-model` / `proptest` 全部已在根 `[workspace.dependencies]` 登记，
一律 `workspace = true`。`audioadapter-buffers` 走 `rubato` 的 re-export，因此不需要登记。

| 类型 | 条目 | 说明 |
| :--- | :--- | :--- |
| `needs` | **流式哈希 + 流式解码** | `import_path` 目前把 ≤2 GiB 的文件读进内存（为了算 `AssetHash`）。真正的流式方案需要在解码器之外维护一份 `sha2::Sha256` 状态机（要新增 `sha2.workspace = true` 直接依赖）。当前上限下内存是可接受的，但超过上限的文件只能被拒绝而不能被导入。 |
| `needs` | **容器标签元数据** | `Metadata` / ID3 / Vorbis comment 的提取**没有实现**：`DecodeFacts` 只承载"解码器真的吐出的事实"。规范里的资产元数据（标题/艺术家/循环点）需要再接一层，且需要先核验 `symphonia::core::meta::Metadata` 的 API。 |
| `needs` | **编码器延迟/填充的裁剪** | `Track::delay` / `padding` 只被**记录**，没有被用来裁剪样本（gapless）。本线启用的编解码里 WAV/FLAC 不上报它们，所以当前无实际影响；一旦启用 MP3/Vorbis 就必须落地，否则时长会偏。 |
| `needs` | **`f64` 重采样域** | 重采样在 `f32` 域进行（与资产样本格式一致、内存减半）。若要进一步压底噪可切 `Async::<f64>`，代价是中间缓冲翻倍。属于质量取舍，不是缺陷。 |
| `needs` | **`rubato` 的 CPU 特性失败路径** | `rubato` 在 sinc SIMD 初始化失败时会给出 `MissingCpuFeature`。本线**没有**主动探测/降级（`Async::new_sinc` 内部处理了标量回退），但也没有为"构造失败"写专门的用户可读提示。 |
| `pending` | **跨架构确定性对账** | ARCH-DET-001 的 L2（`< 1e-6`）需要 x86_64 + AArch64 双 runner 跑同一份资产并比对最大样本差。ADR-0001 D7 已把这类门禁标 `PENDING`（等自托管固定频率 runner），本线照此**不假装通过**。 |
| `pending` | **OGG / Vorbis 与 ADPCM 的字节级夹具** | 这两个格式的 feature 已启用、代码路径存在，但**没有构造字节、没有测试**（见 §3.2）。造一个合法 OGG 页 + Vorbis 包远贵于造 WAV/FLAC，收益也不如先把无损路径钉死。 |
| `pending` | **基准（`BASELINE-*`）** | 本线没有加 `criterion` / `iai-callgrind` 基准。解码速度与重采样吞吐没有打点，因此 AGENTS.md §3 DoD 4（基准无衰退）对本线**无法判定**（不是"通过"）。 |

---

## 10. 需要人类裁决 / 其它工作线须知

1. **工作区级共享文件改动**：本线为了跑通门禁修改了
   [`Cargo.lock`](../../Cargo.lock)（新增 symphonia/rubato 及其传递依赖）与
   [`docs/ledger/dependency-licenses.md`](../../docs/ledger/dependency-licenses.md)（机器重新生成）。
   这两份文件是**多线共享**的，集成者合并时需要留意冲突；`license_inventory.py --check`
   在 CI 上会重新对账，因此清单漂移不会静默通过。
2. **新增许可全部在白名单内**：MPL-2.0（symphonia 全家桶）、`MIT OR Apache-2.0`
   （rubato / audioadapter / num-*）、`MIT`（windowfunctions / extended）、
   `Zlib OR MIT OR Apache-2.0`（visibility）、`0BSD OR Apache-2.0`（audio-codec-algorithms，
   取 Apache-2.0 分支即可）。**没有**需要扩白名单的条目，因此**没有**新的 ADR。
3. **`0BSD` 说明**：`audio-codec-algorithms` 的表达式是 `0BSD OR Apache-2.0`。按 SPDX 的
   OR 语义，选 Apache-2.0 分支即可满足 `deny.toml` 白名单。若人类希望**显式**把 `0BSD`
   也列进 `allow`（而不是依赖 OR 分支），那是一次 `deny.toml` 修改，属于集成者 + 人类的范围。
4. **`--all-targets` 的 `target/ui-test-port` 上传步骤**：CI 里有一条"上传无头 UI 截图（若该 crate
   产出了）"的步骤对本 crate 是空操作（`if-no-files-found: ignore`），无需改动。
5. **规范措辞**：`ROAD-M-1-004` 的正文（roadmap 第 131 行）说的是"仅保留开放的 SFZ v2 与标准
   PCM WAV/FLAC"，而任务书把同一个编号描述为"只支持 SFZ v2 + PCM WAV/FLAC，剔除 NKI/EXS24/RVC"。
   两者一致，本线按"WAV/FLAC 必须 + 其余开放标准可留"的口径执行，并把 ADPCM/Vorbis/OGG
   标注为"代码路径存在但未测"。**没有**修改任何 Normative 文档。
6. **规范措辞 vs 上游 API（"规范 ≠ 上游"族，已上报集成者，将由 ADR 记录为 Proposed）**：
   (a) 若 `ARCH-DSP-002` 正文点名了 `SincFixedIn` / `FftFixedIn` / `FastFixedIn`，那是引用了
   **rubato 的旧版 API** —— 5.0.1 里这三个类型不存在（确切签名与 docs.rs URL 见 §2.2），
   与 ADR-0001 D18/D22 属于同一族"规范写了上游没有的东西"；
   (b) symphonia 0.6 的三处变更（`next_packet` 的 EOF 是 `Ok(None)`、缓冲类型是
   `GenericAudioBufferRef`、`MediaSource` 没有 `Read + Seek` 的 blanket impl）见 §2.1 表格。
   本线**没有**修改任何 Normative 正文，只把核验结果与出处留在 §2。
7. **待人类确认（非阻塞）**：`MAX_PCM_BYTES = 2 GiB` 是否足够？按当前口径，
   96 kHz 立体声约 46 分钟、96 kHz 8 声道约 11.6 分钟。若产品预期导入更长的素材
   （例如录音棚整场 96 kHz 多声道），需要人类给出新的上限，并同步修改
   `limits::MAX_PCM_BYTES` 与 §4 的口径表 —— 判据 C1 会强制这次同步。
