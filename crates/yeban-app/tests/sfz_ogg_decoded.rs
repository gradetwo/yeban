//! 判据（任务单 `#95`）：SFZ 样本的**通用解码腿** —— **OGG/Vorbis** 真的被解出来，
//! 且**确实**走的是 `#85` 接线的那条腿（`yeban_decode::decode_path` ＋ 折单声道）。
//!
//! ## 本单补的是 `#85` **自己登记**的那个缺口（⛔ 不是新发现）
//!
//! `#85` 把非 WAV 样本接到了通用解码腿，但它逐字登记：
//! 「**OGG 未做端到端**（仓内没有真 Ogg 夹具，`yeban-decode` 的 Ogg 夹具是 `#[cfg(test)]`
//! 私有）—— 它走**同一条腿**……**但我没有实测**」。
//! ⇒ 本单把那句「走同一条腿」从**推断**变成**判据**。
//!
//! ## 夹具来源（⛔ 不是"随便造的字节"；形状照 `sfz_decoded_formats.rs` 的 FLAC 先例）
//!
//! 三份夹具都由**参考编码器** `ffmpeg`（8.1.2，`--enable-libvorbis`）从**本判据自己写的**
//! 24-bit PCM WAV 现场转出，命令逐字：
//!
//! | 夹具 | 命令（cwd = 本判据的 scratch 目录） | 字节 | SHA-256 |
//! | :--- | :--- | ---: | :--- |
//! | `stair.ogg` | `ffmpeg -v error -y -i stair.wav -c:a libvorbis -q:a 10 stair.ogg` | 5147 | `323f46770d6b089768fb49acea0101943adc2e1b2a5ca733d9120132ca75345e` |
//! | `stereo44.ogg` | `ffmpeg -v error -y -i stereo44.wav -c:a libvorbis -q:a 10 stereo44.ogg` | 5867 | `10977ef07aaf409c4117896a7068dd1430cbd268118b69e96988e38c4989a7e9` |
//! | `stair_oggflac.ogg` | `ffmpeg -v error -y -i stair.wav -c:a flac -f ogg stair_oggflac.ogg` | 417 | `6e8befbb746d3a6fa6988efd9613e3fd4415e4c0a718a8571fd8b1703e7811e4` |
//!
//! 三份都以**十六进制常量嵌在本文件里**（⛔ 仓里没有新增任何二进制文件、⛔ 没有动
//! `assets/**`、⛔ 没有登记事项）—— 这正是 FLAC 先例的做法：每份的字节数与 SHA-256 在
//! 下面逐条登记，判据用 `yeban_model::AssetHash` **机械核对**（摘要不符 = 夹具被换过 = 红）。
//!
//! ⭐ **编码器版本读数（逐字，`/Users/crow/miniconda3/bin/ffmpeg -version | head -1`）**：
//!
//! ```text
//! ffmpeg version 8.1.2 Copyright (c) 2000-2026 the FFmpeg developers
//! ```
//!
//! 其 `configuration` 行含 **`--enable-libvorbis`**（⇒ 甲组那两份是 **libvorbis** 编出来的
//! 真 Vorbis，⛔ 不是 ffmpeg 自带的实验编码器）。⛔ 判据**不**在运行期调用 `ffmpeg`：
//! 编码只发生在**作者期**，判据只读嵌入的字节（CI 不需要装 ffmpeg）。
//!
//! ## ⚠ 一条**实测**出来的、与任务单预期不同的事实：Vorbis 是**有损**且**没有分母**
//!
//! 任务单 §3.3 的口径是「WAV 腿 `×1/(2²³−1)`、FLAC／OGG `×1/2²³`，相对差恒
//! `1.192093038e-7`」⇒「OGG 腿必须落同一侧」。**本单实测：这对 Vorbis 不成立**，两条
//! 独立的机械证据：
//!
//! 1. **`symphonia-codec-vorbis` 0.6.1 解出的是 `AudioBuffer<f32>`**（`src/lib.rs:72`），
//!    ⛔ 不是整型 ⇒ **根本没有整数分子**，也就没有 `2²³` 这个分母可言；
//! 2. **Vorbis 是有损的**：本夹具（`-q:a 10`，libvorbis 最高档）解出的样本与同一份 PCM
//!    的 24-bit 整数**逐样本都不同**，最大绝对差 **`1.510268450e-2`**（标称 `−0.75`
//!    那一段的过冲，`raw = −6291456`）—— 比分母差 `1.19e-7` **大五个数量级**。
//!
//! ⇒ 本单**如实报告**：FLAC 先例那条"逐样本差 = 分母差"的**逐位**判据，Vorbis 上**不可
//! 构造**（无损才可）。因此本文件分成两组：
//!
//! * **甲组（真 Ogg/Vorbis）**：证明「Vorbis 真的被解出来、且确实走 `yeban_decode` 那条腿」
//!   —— 用**逐位**的同一腿判据（`read_sample_file` ≡ `decode_path` 折声道 0）把"同一条腿"
//!   钉死；与 WAV 腿的对拍则用**实测出来的编解码器容差**（⛔ 不是分母差）。
//! * **乙组（Ogg 容器 ＋ 无损载荷）**：⚠ ⚠ **本条测的是 Ogg 容器 ＋ FLAC 载荷**（无损）；
//!   ⛔ **不证明 Vorbis 解码**。任务单 §3.3 要机械核对的那条性质（「OGG 腿与 FLAC
//!   腿**同分母**」）本身**是可判的** —— 只要载荷无损。`ffmpeg -c:a flac -f ogg` 产出的
//!   **真 Ogg 容器**（417 字节）走**同一个 `is_wav_suffix` 假分支 ⇒ 同一条腿**，于是
//!   「WAV／FLAC 逐样本差 = 分母差」那条判据可以**逐位**照搬（只把 `.flac` 换成 `.ogg`）。
//!   ⚠ 它是**补充**证据（⛔ 不证明 Vorbis 解码），**甲组**才是 `#85` 登记的那个缺口。
//!
//! ## ⭐ 甲组四条各自钉住什么（`#85` 登记的缺口 ＝「Vorbis 端到端」）
//!
//! | # | 判据 | ⭐ 它**单独**钉住的那一件事 |
//! | :--- | :--- | :--- |
//! | ① | `an_ogg_vorbis_..._within_the_codec_tolerance` | 真 Vorbis 经 **SFZ 样本路径**渲染**真的出声**（报告 `decoded_samples=1 silent=0 unreadable=0`），且与同一份 PCM 的 WAV 腿按**实测容差**对拍 |
//! | ② | `the_ogg_leg_is_bit_identical_to_the_shared_decoder_not_the_hound_leg` | ⭐ **最强**：`read_sample_file(ogg) ≡ decode_path(ogg).samples().step_by(channels)`，**逐位**（见下"为什么②最强"） |
//! | ④ | `a_stereo_ogg_vorbis_is_folded_to_channel_zero_like_the_wav_leg` | 立体声 Vorbis 折**声道 0**（`step_by(channels)`），⛔ 不是交织直传 |
//! | ⑤ | `a_44k_ogg_vorbis_is_read_at_its_own_rate` | 44.1 kHz Vorbis **原样上报**采样率、**不重采样**（换算交给既有 `RenderRates::new`） |
//!
//! ## ⭐ 为什么判据 ② 是甲组里**最强**的一条
//!
//! 其它三条都是"某个**可观测量**在容差内"；② 是**恒等式**：
//!
//! ```text
//! read_sample_file(<f>.ogg).pcm  ≡  decode_path(<f>.ogg).samples().step_by(channels)
//! ```
//!
//! ⇒ 它把 `#85` 那句「走**同一条腿**」从**推断**变成**恒等式**：左侧是 SFZ 样本路径真正
//! 交给引擎的缓冲，右侧是 `yeban-decode` 的解码结果按**同一个** `step_by(channels)` 折声道 0。
//! 两侧都用 `==`（`f32` 逐位），所以：分派把 `.ogg` 交给 hound ⇒ 左侧**报错**（`expect` 红）；
//! 折法被改成直传 ⇒ 立体声那侧长度不等（红）；期望值差 `1e-7` ⇒ 红（实测注入 ③ 就是这样）。
//!
//!
//! ## 判定量（每个数字都有单位）
//!
//! | 量 | 对象 | 单位 |
//! | :--- | :--- | :--- |
//! | `differing` | 两次导出（WAV 版 vs OGG 版）里 `f32` **位型不同**的样本 | 个 |
//! | `max_abs_diff` | 同上，逐样本 `abs(a − b)` 的最大值 | 线性幅度 |
//! | `fnv1a_64` | 一条导出的**交错 f32 样本**（逐位卷字节） | 无符号 64 位整数 |
//! | `decoded_samples` | 真的进了提供表的样本份数 | 份 |
//! | `silent_region_count` | 引擎口径：缺样本 ⇒ `NO_SAMPLE` 的区域数 | 区域数 |
//! | `sample_hz` / `frames` | 读取器上报的文件采样率 / **折单声道后**的帧数 | Hz / 帧 |
//!
//! ## 判据清单
//!
//! | # | 判据 | 作用 |
//! | :--- | :--- | :--- |
//! | ① | `an_ogg_vorbis_sample_is_decoded_end_to_end_and_matches_the_wav_leg_within_the_codec_tolerance` | **本单核心**：真 Vorbis ⇒ 渲染出声、报告干净、与同一份 PCM 的 WAV 腿按**实测容差**对拍 |
//! | ② | `the_ogg_leg_is_bit_identical_to_the_shared_decoder_not_the_hound_leg` | ⭐ **"同一条腿"判据**：`read_sample_file` 折声道 0 后**逐位**等于 `decode_path`（⛔ 不是 hound 那条） |
//! | ③ | `an_ogg_container_with_a_lossless_flac_payload_is_bit_identical_to_the_wav_leg` | ⭐ **§3.3 的分母判据（乙组）**：⚠ **Ogg 容器 ＋ FLAC 载荷**（无损）；⛔ 不证明 Vorbis 解码。`#254` 之后 `.ogg` 腿与 WAV 腿用**同一个** `raw × 1/2²³` ⇒ 逐位相同 |
//! | ④ | `a_stereo_ogg_vorbis_is_folded_to_channel_zero_like_the_wav_leg` | ⭐ §5.3 折单声道（注入 ② 的靶子） |
//! | ⑤ | `a_44k_ogg_vorbis_is_read_at_its_own_rate` | ⭐ §5.3 采样率原样进、不重采样 |
//! | ⑥ | `a_broken_ogg_is_classified_and_a_valid_ogg_in_the_same_batch_is_accepted` | ⭐ §5.4 可判别性（实测档位，⛔ 不猜） |
//!
//! ⚠ **本文件⛔ 不改 `src/sfz_load.rs` 的分派逻辑**（`#254` 只改归一化分母，见
//! `yeban-render/src/wav.rs` 的 `read_plain_wav_mono_f32`）。`#254` 之后
//! `sfz_sample_formats.rs` 的 `BASELINE_WAV_READ` 已按三件套重钉为
//! `0x684c580cf970b275`（旧值 `0xe8da0829d81c2115`）；本文件只复跑它。
//!
//! [`BitDepth::full_scale_max`]: yeban_render::dither::BitDepth::full_scale_max

use std::path::{Path, PathBuf};

use yeban_app::render_core::{RenderProjectOutput, render_project_f32_with_assets};
use yeban_app::sfz_load::{SampleReadFailure, decode_sfz_plan, read_sample_file};
use yeban_decode::{DecodeOptions, decode_path};
use yeban_model::{AssetHash, EntityId, SamplerBinding, YebanProjectV1};
use yeban_render::dither::{BitDepth, PcmBuffer};
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::{read_plain_wav_mono_f32, write_plain_wav};

/// `24-bit` 满量程（夹具写入常量样本用；与 `write_plain_wav` 同口径）。
const I24_SCALE: f32 = 8_388_608.0;

/// ⭐ 两条腿**统一后**的归一化分母（`#254`）：WAV 腿与 symphonia 腿都用 `2^23`。
///
/// `#254` 之前 WAV 腿用 `2^23 − 1`（[`BitDepth::Int24::full_scale_max`]），两条腿的相对差
/// 是 `1/8388607 ≈ 1.1920929e-7`。统一之后这个常量只剩**历史记录**用途（打印出来对照），
/// ⛔ 不再被任何"差必须 ≤ 界"的断言使用。
const DENOMINATOR_GAP: f32 = 1.0 / 8_388_607.0;

/// 阶梯夹具的八段：`(线性幅度, 帧数)`，共 `800` 帧（`8 × 100`）。
///
/// ⚠ **⛔ 刻意不是常量**：八个互不相同（含正负）的值让"WAV 腿 vs OGG 腿"的比较**不是**
/// "两份都恰好是 `0.25`"这种空对空 —— 任何增益／符号／分段的错位都会改变指纹。
const STAIR_SEGMENTS: [(f32, usize); 8] = [
    (0.25, 100),
    (-0.5, 100),
    (0.125, 100),
    (-0.75, 100),
    (0.5, 100),
    (-0.125, 100),
    (0.375, 100),
    (-0.25, 100),
];

/// ⭐ **实测**出来的 Vorbis 编解码容差（甲组判据 ① 的上界，单位：线性幅度）。
///
/// 两处读数（本机 `aarch64/macos`，2026-10-16）：
///
/// | 层 | 量 | 实测值 |
/// | :--- | :--- | :--- |
/// | 解码层（`stair.ogg` 的样本 vs 标称 `raw/2²³−1`） | `max_abs_diff` | `1.510268450e-2`（`raw = −6291456` 的过冲） |
/// | 渲染层（WAV 腿 vs OGG 腿，判据 ①） | `max_abs_diff` | `6.628990173e-3` |
///
/// 本常量取 **`1e-2`**（≈ 渲染层实测值的 1.51 倍）作为上界 —— ⛔ 留的余量刻意很小，
/// 因为夹具与代码都是确定的（读数不会漂）。
///
/// ⚠ 这个数**不是**分母差（`1.19e-7`）：Vorbis **有损**，误差由量化＋MDCT 交叠重建决定，
/// 与归一化分母无关（见模块文档"一条实测出来的…事实"）。把这条上界写成 `peak × 分母差`
/// 那样的量级会让判据**恒红**。
const VORBIS_LOSSY_BOUND: f32 = 1.0e-2;

/// 立体声夹具的**左**声道（= [`STAIR_SEGMENTS`] 展开后的 `800` 帧）。
fn stair_values() -> Vec<f32> {
    STAIR_SEGMENTS
        .iter()
        .flat_map(|(value, frames)| std::iter::repeat_n(*value, *frames))
        .collect()
}

/// 十六进制串 ⇒ 字节（夹具嵌入用；⛔ 不引第三方 hex crate）。
fn unhex(hex: &str) -> Vec<u8> {
    assert!(hex.len().is_multiple_of(2), "十六进制串必须是偶数长度");
    (0..hex.len() / 2)
        .map(|index| {
            u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).expect("夹具十六进制必须合法")
        })
        .collect()
}

/// 取一份夹具，并**机械核对**它的 SHA-256（模块文档里登记的那个）—— 换过即红。
fn fixture(hex: &str, expected_sha256: &str, expected_bytes: usize) -> Vec<u8> {
    let bytes = unhex(hex);
    assert_eq!(bytes.len(), expected_bytes, "夹具字节数必须与登记值一致");
    assert_eq!(&bytes[0..4], b"OggS", "夹具必须是 Ogg 容器");
    assert_eq!(
        AssetHash::of_bytes(&bytes).as_str(),
        expected_sha256,
        "夹具的 SHA-256 必须与登记值一致（摘要不符 = 夹具被换过）"
    );
    bytes
}

// ---------------------------------------------------------------------------
// 夹具（参考编码器产出；见模块文档"夹具来源"）
// ---------------------------------------------------------------------------

const STAIR_OGG_HEX: &str = concat!(
    "4f67675300020000000000000000e8fc7ef300000000bfebe490011e01766f72626973000000000180bb00000000000030a903000000",
    "0000b8014f67675300000000000000000000e8fc7ef30100000049b872a31140ffffffffffffffffffffffffffffffd503766f726269",
    "730d0000004c61766636322e31322e313032010000001f000000656e636f6465723d4c61766336322e32382e313032206c6962766f72",
    "6269730105766f726269732b424356010008000000314c20c580d0905500001000006024290e93664929a594a128799894484929a594",
    "c53089989489c518638c31c618638c31c618638c20346415000004008028098ea3e6496ace396718278e72a039694e38a720078a51e0",
    "3909c2f526636ea6b4a66b6ece2925080d590500000200404821851452482185146288218618628821871c72c821a79c720a2aa8a082",
    "0a32c820834c32e9a4934e3ae9a8a38e3aea28b4d0420b2db4d24a4c31d55663aebd065d7c73ce39e79c73ce39e79c73ce0942435601",
    "002000000442061964104208218514528829a698720a32c880d0905500002000800000000047911449b114cbb11ccdd1244ff22c5113",
    "35d1334553544d55555555755d577665d77675d7767d5998855bb87d59b8855bd8855df78561188661188661188661f87ddff77ddff7",
    "7d2034641500200100a0233996e329a2221aa2e239a2038486ac02006400000400200992222992a349a6666aae699bb668abb66dcbb2",
    "2ccbb20c8486ac0200000100040000000000a0699aa6699aa6699aa6699aa6699aa6699aa6699a665996655996655996655996655996",
    "65599665599665599665599665599665599665599665594068c82a0040020040c7711cc77124455224c7722c07080d590500c8000008",
    "0040522cc57234477334c7733cc7733c477444c9944ccdf44c0f080d59050000020008000000000040311cc5711cc9d1244f522dd372",
    "3557733dd7734dd7755d575555555555555555555555555555555555555555555555555555555555555555555581d090550000040000",
    "219d66966a800833906120346415008000000018a108430c080d5905000004000088a1e4209ad09af3cd390e9ae5a0a9149bd3c18954",
    "9b27b9a9989b73ce39e79c6cce19e39c73ce29ca99c5a099d09a73ce490c9aa5a099d09a73ce79129b07ada9d29a73ce19e79c0ec619",
    "619c73ce69d29a07a9d9589b73ce59d09ae6a8b9149b73ce89949b27b5b9549b73ce39e79c73ce39e79c73cea95e9ccec139e19c73ce",
    "89da9b6bb9095d9c73cef9649ceecd09e19c73ce39e79c73ce39e79c73ce0942435601004000000461d818c69d82207d8e06621421a6",
    "21931e748f0e93a031c829a41e8d8e464aa98350521927a57482d0905500002000008410524821851452482185145248218618628821",
    "a79c720a2aa8a4928a2aca28b3cc32cb2cb3cc32cbacc3ce3aebb0c310430c31b4d24a2c35d556638db5e69e73ae39486ba5b5d65a2b",
    "a594524a29a52034641500000200402064904106198514524821869872ca29a7a0820a080d59050000020008000000f024cf111dd111",
    "1dd1111dd1111dd1111dcff11c511225511225d1322d53333d55545557766d599775dbb7855dd875dfd77ddfd78d5f17866559966559",
    "96655996655996655996650942435601002000000042082184145248218594628c31c79c834e420981d0905500002000800000000047",
    "7114c7911cc991244bb2244dd22ccdf2344ff334d1134551344d53155dd11575d3166553365dd33565d35565d57665d9b6655bb77d59",
    "b67ddff77ddff77ddff77ddff77ddfd7752034641500200100a023399222299222398ee34892048486ac02006400000400a0288ee238",
    "8e23499224599226799667899aa9999ee9a9a20a8486ac0200000100040000000000a0688aa7988aa7888ae7888e28899669899aaab9",
    "a26ccaaeebbaaeebbaaeebbaaeebbaaeebbaaeebbaaeebbaaeebbaaeebbaaeebbaaeebbaaeebba4068c82a0040020040477224477224",
    "45522445722407080d590500c800000800c0311c435224c7b22c4df3344ff334d1133dd1333d55744517080d59050000020008000000",
    "0000c0900c4bb11ccdd12451522dd55235d5522d55543d55555555555555555555555555555555555555555555555555555555555555",
    "5555d5344dd33481d090950000190000e8c50821841092a3965a10be57ca392835f75e316614c4de7ba598418e72f099624a3928b5a7",
    "ce31a588915c5b2b9122c4610e3a554e29a841e7d649082d07424356040051000000428831c418628c41c820448c3108198488310621",
    "83d04108258594320821959052c41883d041c920849442491994905248a500008000070080000ba1d0901501409c0000829073883108",
    "1563103a08a9741052aa188390392725730e4a2825a5104a4a15631032e72464ce490925a4144a49a983905228a5a5504a6a29a51853",
    "4a2d7610520aa5a4144a6929b5145b4a2dc68a310899735232e7a484525a0aa5a4963927a583905207a19492526ba5a4d632e7a474d0",
    "49e92094525269a994d45a2825b592526b2595d65a6b31a6d6620ca5a4144a69ada4d4626a29b6d65aac15631032e7a464ce4909a5a4",
    "144a492d734e4a072195ce41292595d64a49a965ce49e92094d24128a5a4d25a49a5b5504a4b25a5d64229adb5d6624ca9b51a4a49ad",
    "a4d45a49a9b5d45aadadb5183b082985525a0aa5b4965a8a31a5166328a5b592526b25a5d65a6bb5b6d6620ca5b4545269ada4d45a6a",
    "adc6d65aaca9a518536b31b6d66a8d31c61c63cd39a514636a29c6d45a8c2db61c63ac357710520aa5a4164a492db514636a2dc6504a",
    "6a2595d64a492db6d66a4cadc51a4a69ada4d45a49a9b5d65a8dadb51a534a31a6d66a4ca9c51863ccb5b518736a2dc6d65aaca9b518",
    "63ac35c7186b2d000060c0010020c084325068c84a00200a0000310621c69c330829c51884c620a518831029c598731022a518730e42",
    "c69873104ac918730e42291d84124a49a983104a292915000050e00000106083a6c4e2008586ac040042020018849462cc39e7209492",
    "528490528c39e71c8452528a10528a31e79c83504a4a95524c31e61c8452526aa9524a31c69c83504a4aa9658c31e61c84104a49a9b5",
    "8c31c69c8310422929b5d639e71c74124a49a5a5d83ae79c83104a2925a5d65ae71c84104a49a5a5d662eb9c8310422925a5d45a8b21",
    "84524a4925a596628b3184524a2925a5945a8b319654524aa9a5d6628bb1c6524a4a29a5d65a8b31c69a526aa9b5d6628cb1c65a534a",
    "a9b5d65a8b31c65a6b010000070e00000146d049469545d868c28507a0d090150140140000600c620c31869c7310320891730c420721",
    "72ce49e9a46452426921a54c4a4825a41639e7a474523229a1a550522625a4545a290000ecc00100ecc04228346425009007000021a4",
    "14638c318694528a31c61c434a29c518638c29a518638c31e794528c31c698738c31c61c73ce39c61863cc39e71c63cc31e79c738e31",
    "c69c73ce39c71c73ce39e79c63ce39e79c73ce0900002a70000008b05164738291a04243560200a9000084314a31e61c84521a851873",
    "ce3908a5344831e69c73104aa91873ce3908a5945231e69c73104a292573ce390821949252e69c7310422825a5ce39082184524a4a9d",
    "73104228a194944208a59452524aa985104a29a594545a2aa59492524aa9b5564a2925a5945a6aad0000f0040700a0021b564738291a",
    "0b2c34642500900100c0188390410621631042082184104208090000187000000830a10c141ab21200480500000c528a3107a5a41429",
    "c59873104a4929528a31e720949252c59873104a49a9b58a31e7209492526b9d73104a49a9b5183be7209492526b3186104a49a9b518",
    "630c219492526b31d65a4a49a9b5186bccb59492526b31d65a6b4aadb5186bad35e7945a6b31d65a73ce050020343800801dd8b03ac2",
    "49d15860a1212b01803c00004829c618638c31a514638c31c698528a31c618638c31c618638c31a618638c31c618638c31c618638c31",
    "c618638c31c618638c31e618638c31c61863cc39c618638c39e61c638c31c69c734e00005081030040808d229b138c04151ab2120008",
    "0700008c61cc39e72094904aa394731042282595561aa59c8312422929b596392725a552526a2db6cc3929299592526b2d7612526a2d",
    "a5d662acb18390526ba9b516638d1d84525a8a2dc61a73ed2094925a6b31c65a6b28a5a5d862acb1d69a4329a9b51663ad35e75c526a",
    "2dc65a6bcdb5e792526b31c65a6badb9a7d662acb1d65c73ef3db516638db5e69c7bce0500983c38004025d838c34ad259e16870a121",
    "2b0180dc00004629c69c730e4208218410420895528c39e71c841042082184102aa51873ce3908218410420821648c39e71c74104208",
    "21841042c81873ce390821841042082184d039e71c841042082184504229a573ce390721841042082184504ae71c841042082184124a",
    "28a594ce39082184104208a584524a2921841042082184124a29a594523a082184104208a594524a29a5841042082184104a29a59452",
    "4a0921841042082194524a29a5941242082184124a29a594524a292584104208a194524a29a594524a082184524a29a594524a29a594",
    "1042282194524a29a594524a292184124a28a594524a29a5945242082594524a29a594524a29a584104228a594524a29a594524a0921",
    "94524a29a594524a29a594520000d0810300408011951662a719571e81230a1926a04243560200e10000002194524a29a594526a24a5",
    "94524a29a594522325a594524a29a594524a29a594524a29a594524a29a594524a29a594524a29a594524a29a594524a29a594524a29",
    "a594524a29a594524a29a594524a29a594524a29a594524a29a594524a29a594524a29a594524a29a594524a29a594524a29a594524a",
    "a594524a29a594524a29a594524a29a594524a29a500a8cb0c07c0e8091b6758493a2b1c0d2e346425009016000018c398638e4127a1",
    "94945a6b98825042e8a4a4d24a6cb1354a4108218452524aadb5d632e8a894924a4aadc516638c998352522a25a5d4628cb1d60e424a",
    "2db5165b8bb1e65a6b07a1a4945a8b2dc65a6baebd83904a6bade51863b039e7da412829b5d8628c35d75a7b0ea9b416638cb5f65c6b",
    "cd398852528a31d61a73cd35f7dc4b4aadc59a6bae35079f7310a6a5d86a8d35e79c7b103af8d45a8db9e61e74d041e71e744aadd65a",
    "6bce3d07217cf0b9b5586bcd35e7de830f3a08df6aab35e75c6bef3df79e836e31d65c73d0c107217cf041b8186bcf39f71c840e3af8",
    "1e0c00c88d7000405c3092903acbb0d2881b4fc01081141ab20a00880100208c4106218494524a29a594628a29c618638c31c618638c",
    "31c618638c31c6040000263800000458c1aeccd2aa8de2a64ef2a20f029fd0119b91219752319313418fd4508b956087567083178085",
    "86ac0400c800001088b1e65a738e1094d662edb9544a396ab1e79421829cb49c4bc90c414e5a6b2d64c8282731b614328414b4da5ae9",
    "94528c62abb1748c3149a9c5964ae7200000008200000311321308144081810c003840489002000a0b0c1dc34540402e21a3c0a0704c",
    "38279d3600004188cc108988c52031a11a282aa60380c505867c00c8d0d848bbb8802e035cd0c55d074208421082581c4001093838e1",
    "8627def0841b9ca05354ea2000000000001000e0010020d90022a29999e3e8f0f800091119212931394149510900000000002000f800",
    "00485680886866e6383a3c3e40424446484a4c4e50525402000000000000000080808000000000004000000080804f67675300042003",
    "000000000000e8fc7ef3020000009df69b080871787379677877614c326b0480097e7380f237f9ede9e6d6c9979ddf4bfdeeea4e9f57",
    "7fa7fdbb91dbef57cf5b36cf126f1d9dbb4cfa3af1d6e4799b0463a5926ad99b7573731242ce8f125265b4abfee66d343d49c55eb733",
    "bebceedbd592e234c68a7fb5482f3ebb69b61adaedb5d92e7661df1ea8319d0c943655008213609a031ccb3fd6556e3f3a7e69ffe657",
    "bd5efb0fdda5bd8769697bfe32f55cbfb4dd63e7c1d27a77b453c7561e489c737992f34afb56f81988dee76a740be2f2b435ac7a72a2",
    "4963dc801cc2f9a33ea4fdd0dada81adcb61a3f170fa9bf5d3a6b79bdf6d739a3687ee2f73d77273dfb6dabbc44a6394514e06a5ffe2",
    "8bb9cf6ac552fbf7b7b32fdfce67650be77571fffd87f6fae6dc295fdacbeba5b9bd4e0b7bc72ed71a87c7d91c8885ddd1b48264e307",
    "0cfdc3a6760dbeaf9b85b5d93987debd8167f096b643ab2883fb88ff2206c6dbb8bfdf91b0c46dff6cf7ddd764fb4ac7aaf900ac428d",
    "038000d31c60ef253f70fdd0bffedeb167efcdb674f393b3a7fcdb5658defd9997bf3ec4f27bffb2defee257ed2f9cdd4789dda79e6e",
    "95c1e67533b6a5c16d6d18166107e1d18729f99fa2d19547f79286255e9605db4596fdd751d6db9acb923e58b16d42ed8b0ac9d96968",
    "3a2d66eb6e9c3f4cb306ac2e6554090060ff39eca1afd67374f7bf3ab6a4d8ffea3816bbfbdf1c12ecee7f7bdadd31b9331df08efdd8",
    "ddf861a38f8aca0a692a2d294cb1188479ddc4a3c8f1ef1606d1f5f9ef6695b05e567dd46ea9de73f2aec1d867be39c86174ffebd34c",
    "70bd5b0f00942aad01aad241eb5f328b87b7d7b9b8f2fbdfbff4f4b678f9d2fa61cb4a0edcdf3585f2f5cef17d3dcbaf5f2ec5956fbe",
    "9c79cf3c39a983abdeb074af0f0d149109dfbcb209033e4a9af1fdb018600e043c5a34c955dba641a7326c25addf244a334968e491e8",
    "818ab9afef2d97037b15b4ebe18f5b3604a422670e2e3b02ec7fb3cf3cfef8e38f3feab9dc78ebc6d55b6fbd95fffbbf0ffdf0d9e5b4",
    "f87067d76deffbc007f281db6edbb36b24ac353dd9ffdc37cf3d677d6efffec97416b011c9d3c7fa551c37dac627ad43a3f1b5f7bede",
    "5f29f79db3a27bd3aaf7e6dd7c3170e0571f6d7af7871ffff8c73f062c12df2f025d3d61dad7ff5f79f77875d7f3cdbebfa4fc9fd57e",
    "6537ffe9f41bdb6f6cded7f14dc2fbda3e1cfac08af3e1ee0f9c3e3fd8b338ac1ac0e660bb765a42a59faa0c6bf61cfb8577adb76ca7",
    "99169001c93af6d17dff9eb627f1f61300",
);

const STEREO44_OGG_HEX: &str = concat!(
    "4f676753000200000000000000009527d93c000000002ca0f541011e01766f72626973000000000244ac0000000000006da007000000",
    "0000b8014f676753000000000000000000009527d93c01000000afd108401140ffffffffffffffffffffffffffffff5303766f726269",
    "730d0000004c61766636322e31322e313032010000001f000000656e636f6465723d4c61766336322e32382e313032206c6962766f72",
    "6269730105766f726269732b424356010008000000314c20c580d0905500001000006024290e93664929a594a128799894484929a594",
    "c53089989489c518638c31c618638c31c618638c20346415000004008028098ea3e6496ace396718278e72a039694e38a720078a51e0",
    "3909c2f526636ea6b4a66b6ece2925080d590500000200404821851452482185146288218618628821871c72c821a79c720a2aa8a082",
    "0a32c820834c32e9a4934e3ae9a8a38e3aea28b4d0420b2db4d24a4c31d55663aebd065d7c73ce39e79c73ce39e79c73ce0942435601",
    "002000000442061964104208218514528829a698720a32c880d0905500002000800000000047911449b114cbb11ccdd1244ff22c5113",
    "35d1334553544d55555555755d577665d77675d7767d5998855bb87d59b8855bd8855df78561188661188661188661f87ddff77ddff7",
    "7d2034641500200100a0233996e329a2221aa2e239a2038486ac02006400000400200992222992a349a6666aae699bb668abb66dcbb2",
    "2ccbb20c8486ac0200000100040000000000a0699aa6699aa6699aa6699aa6699aa6699aa6699a665996655996655996655996655996",
    "65599665599665599665599665599665599665599665594068c82a0040020040c7711cc77124455224c7722c07080d590500c8000008",
    "0040522cc57234477334c7733cc7733c477444c9944ccdf44c0f080d59050000020008000000000040311cc5711cc9d1244f522dd372",
    "3557733dd7734dd7755d575555555555555555555555555555555555555555555555555555555555555555555581d090550000040000",
    "219d66966a800833906120346415008000000018a108430c080d5905000004000088a1e4209ad09af3cd390e9ae5a0a9149bd3c18954",
    "9b27b9a9989b73ce39e79c6cce19e39c73ce29ca99c5a099d09a73ce490c9aa5a099d09a73ce79129b07ada9d29a73ce19e79c0ec619",
    "619c73ce69d29a07a9d9589b73ce59d09ae6a8b9149b73ce89949b27b5b9549b73ce39e79c73ce39e79c73cea95e9ccec139e19c73ce",
    "89da9b6bb9095d9c73cef9649ceecd09e19c73ce39e79c73ce39e79c73ce0942435601004000000461d818c69d82207d8e06621421a6",
    "21931e748f0e93a031c829a41e8d8e464aa98350521927a57482d0905500002000008410524821851452482185145248218618628821",
    "a79c720a2aa8a4928a2aca28b3cc32cb2cb3cc32cbacc3ce3aebb0c310430c31b4d24a2c35d556638db5e69e73ae39486ba5b5d65a2b",
    "a594524a29a52034641500000200402064904106198514524821869872ca29a7a0820a080d59050000020008000000f024cf111dd111",
    "1dd1111dd1111dd1111dcff11c511225511225d1322d53333d55545557766d599775dbb7855dd875dfd77ddfd78d5f17866559966559",
    "96655996655996655996650942435601002000000042082184145248218594628c31c79c834e420981d0905500002000800000000047",
    "7114c7911cc991244bb2244dd22ccdf2344ff334d1134551344d53155dd11575d3166553365dd33565d35565d57665d9b6655bb77d59",
    "b67ddff77ddff77ddff77ddff77ddfd7752034641500200100a023399222299222398ee34892048486ac02006400000400a0288ee238",
    "8e23499224599226799667899aa9999ee9a9a20a8486ac0200000100040000000000a0688aa7988aa7888ae7888e28899669899aaab9",
    "a26ccaaeebbaaeebbaaeebbaaeebbaaeebbaaeebbaaeebbaaeebbaaeebbaaeebbaaeebbaaeebba4068c82a0040020040477224477224",
    "45522445722407080d590500c800000800c0311c435224c7b22c4df3344ff334d1133dd1333d55744517080d59050000020008000000",
    "0000c0900c4bb11ccdd12451522dd55235d5522d55543d55555555555555555555555555555555555555555555555555555555555555",
    "5555d5344dd33481d09095000019000084c507a18c521293d45aecc1588a3108a506e5318514839684c79842ca514ea2630a21e530a7",
    "d239868c91da620a99324259f13d768c21873d189d42e8240642435604005100000649224924c9f23ca247f42ccfe3893c1180e47934",
    "8de7499e47f3781e0049f4781e4d933c91e7d134010000010e0000011642a1212b02803801008b24791e49f23c92e479344d14218a96",
    "a6891ecf13459e268a44d334a19a96a67922cf13459a278a4c5135619a9ee89926d37455a6a9aa5c599621bb9e279a26d3545da6a9aa",
    "64579621cb0000002c4f334d9a668a34cd3489a269c2342dcd334d9a269a34cd3489a269c2343d515455a6a9aa4c5355b9aeebc2753d",
    "d15455a2a9aa4c5355b9aeebc275010000489e669a34cd34699a291245d384695a9a679a34cd34699a681245d384697aa6e8aa4cd355",
    "99a2aa525dd785eb7aa2a9ba4c535589a6aa7255d785eb020000d04cd17589a2ab124555659aae0ad5d544d37589a2ea124555659aaa",
    "0b551555537699a6eb324dd7a5aaae0bd9154dd59599a6eb324dd7a5baae0b5706000000000000000080a89ab2cc345d9769ba2ed575",
    "5db8ae68aab2cc345d9769ba2e57955db8ae000080010700800013ca40a1212b01802800008be3489265791ec791244bf33c8e23499a",
    "e67924c9b2344d1461599a268ad034cf1345689ae7892200000200000a1c0000026cd094581ca0d090950040480080c57124c9b234cd",
    "f344d1344d93e44892a6799ee789a269aa2a49b22c4df33ccf1345d354559664599ae679a2689aaaaabab02c4df33c51344d55755d68",
    "9aa689a2289aa6aaba2e344df34451144d53555d179ae679a2689aaaeabab20c3c4f144d53555dd77501000000000000000000000000",
    "000000000400001c3800000418412719551661a309171e80424356040051000080318831c598610a4a2925348a41292594084248a9a4",
    "9449482db5d6322829b5d65a25a5b4565acaa4a4d6526b9994d45a6bad0000b0030700b0030ba1d0909500401e00008390528c31c618",
    "454829c618738e22a414638c394711528a31e79ca3942ac51873ce514a95628c39e728a54a31c698739452c61863cc394aa9948c31e6",
    "1ca59452c618638c524a29638c31260000a8c0010020c04691cd0946820a0d590900a40200381cc7b2344dd33c4f1425c7b12ccf1345",
    "51344dcb712ccbf34451144d9365699ae789a269aa2acbd234cf1345d35455a6e979a2689aaaeaba54d5f344d13455d5750100000000",
    "00000000000100e0090e00400536ac8e705234165868c84a0020030080310621640c42c8188410420821841012000030e00000106042",
    "192834642500900a004018a31463ce4949a9324629e72094d25a659052ce4128a5b56629a59c8392526bcd524a392725a5d69a291983",
    "504a4aad35953206a194945a6bce8910424aadc5d89c132184945a8bb13927632929b51863734ec652526a31c6e69c53aeb51663cd49",
    "29a55c6b2dc65a0b0040687000003bb061758493a2b1c0424356020079000090524a31c618634c29a518638c31a694528c31c698534a",
    "29c61863cc39a714638c31e69c638c31c61873ce31c618638c39e718638c31c69c73ce31c61863ce39e718638c31e79c738c31c69800",
    "00a002070080001b45362718092a34642500100e000018c39473ce412825950a21c62074504a4aad55083106218452526a2d6ace3908",
    "219492526bd173ce4108a194945a8baa85504a2925a5d65a742d74524a49a9b518a394228490524aadb5189d13218492526a2dc6e69c",
    "8ca5a4d45a8c3136e7642c25a5d6628cb139e79c6badb51663adcd39e75c6b29b6186b6dce39a77b6c31d6586b73ce399f5b8badc65a",
    "0b0030797000804ab0718695a4b3c2d1e04243560200b901008c528c31e69c73ce39e79c73ce49a51873ce3908218410420821944a31",
    "e69c7310420821841042281973ce390721841042082184504ae99c731042082184104208a194d239e720841042082184104229a573ce",
    "410821841042082184524a0821841042082184104208a59452420821841042082184104a29a58410420821841042082194524a092184",
    "1042082184104228a59412420821841042082584504a29a594104208a184104208a194524a292184524a292184104229a594524228a1",
    "841042082194524a29a5941242292184104208a594524a29a5945242082184104a29a594524a29a5845042082194524a29a594524228",
    "2584124228a594524a29a5845042082184504a29a594524a092184124208a10000a00307008000232a2dc44e33ae3c024714324c4085",
    "86ac0400d202000043acb5d65a6badb5d65a6b0d52d65a6badb5d65a6badb546296badb5d65a6badb5d65a6ba9b5d65a6badb5d65a6b",
    "adb5d65a6badb5d65a6badb5d65a6badb5d65a6badb5d65a6badb5d65a6badb5d65a6badb5d65a6badb5d65a6badb5d65a6badb5d65a",
    "6badb5d65a6badb5d65a6badb5d65a6badb5d65a6badb5d65a6badb5d65a6badb5d65a6badb5d65a6badb5d65a6badb5d65a6badb5d6",
    "5a6badb5d65a6badb5d65a4b29a594524a29a594524a29a594524a29a594524a29a594524a29a594524a29a594524a29a594524a29a5",
    "94524a29a594524a29a594524a29a5945201d8051b0e80d1134612526719561a71e309182290424356020069010080318c31e6187410",
    "4a4929a50a21e720844e422aadc5166384907310422825a5d6628b31780e42082194d2526c31c6583c07218410526a2dc618630cb285",
    "504a2929b5d6628cb516d94228a594945a8b31d65a8331a69492526aadd55863acc518134a48a9b5d662ccb5d6627cac25a5d4628cb1",
    "c6586b31c6b61452892dc6586b8db518618c6aadc55863adb1d65a8c31c29516628ab5d65a732d460863738b31d6586baeb916618cd1",
    "b9955a6a8db1d65a8b2fc618616bac35c65a6bcec5182384b0b5b61a6bcd35d7628c31c6081f63acb5d6dc7331c61863849031c61a6b",
    "ceb90080dc080700c4052309a9b30c2b8db8f1040c1148a121ab008018008021008462b2010080090e00000156b02bb3b46aa3b8a993",
    "bce883c02774c46664c8a554cce444d02335d46225d8a115dce00560a1212b0100320000c459cd39c79c2be4a4b5d86a2c15520e528a",
    "3176c820e524c55a32641083d462ea143288416aa9740c190425c6543a850c835c632ba1630e5aabb1a5123a080000802000c04084cc",
    "04020550602003000e1012a40080c20243c7701110904bc82830281c13ce49a70d004010223344226231484ca8068a8ae90060718121",
    "1f00323436d22e2ea0cb00177471d781108210842016075040020e4eb8e189373ce10627e814953a1000000000000800780000483680",
    "886866e6383a3c3e40424446484a4c4e50525404000000000010003e0000921520229a99398e0e8f0f90109111921293139414950000",
    "4000010000000010400002020200000000000100000002024f676753000420030000000000009527d93c0200000046a1b19108e1e5e7",
    "e4c5e2e9bd440eb305800aeea8f4fbe2c20f1d18b62fe02fcbe263fd9f5b7ffddfee6f233e8bfd5fbfdfffcdbc6dd8dff4c6f7dff26b",
    "69afd6f7fdfcabbc5f9bd57d7d763e5d6b1be77afddafae7d3c5e7fad2bfee5caea1c6f9ab9ffa1e28cfaeef8bb3ec987fff783c7a9e",
    "4ffad18c027f628c9b71cef07b7c15de5decfc96993f41668ef34ccbe9d9f9de4e4f314db9fc26a95314b7516b55796c04e375ad4fda",
    "2f8593ae5053faaf7fb22eaa3fbf2b1ebd23ad26e5c5bb4fa62f947aa679eeebec3de24276ff50757e58f6f36f46bf91399db2f67ca6",
    "cd36bf61fcd7d3edd8d7e785fb6e9412a5024226034ea5ffdb5be46718a67680ef7f3f46bed85b6f6fff6bff7a7cd9dceddb1ffaf4f9",
    "afeffe8fe1721a5e3f8ffd83e079d7f759af5bb5b7eaebaccfba3eab3cfb39fd41d75ff6fbf9f9bcf6f3beafb3eb337856fbac67d7bb",
    "d6be6a5f7041ed1af3f3d93fe7e7637ef6fc1c1ed5fee444731af5e5e0302cce4defdba86fa7ffe4322fb0a505311e77ccc2c34e0a5f",
    "3f7585dcb362213b35ba7b5fa44ad9358c40ca6c1a11feb20c6979876624f0a36811fceeb628d5e95c001af7fcff37de8863ea1e8600",
    "fc361b710c30c1fe0a88354133c267396ffec3e6db7f8eee73dd02c4069d9151018053e9fff616f91986a91da0f4fffc67be8c8fdae8",
    "5f5b1eb7d9f3be30eeeb66f7ffe78dec1ffffb946792c3f3f773bdcee21ff7d3daefebf955ebd9cffa59bd9fd7e7f528ba3cfbac5cf5",
    "e75ccdd967c3552feacdd927d45a2f6a73de275deb552bdd679ff599972d53fd34cd65f958aaf108dbf55e08e8405670ea48b7c8b170",
    "d0bcc4e2eed3ed7684c1e906d90446331ea7edb6e8acd44f359c16e148d695c9163224e67e83ff113f76bcfe8fedc64d5c8beb3b0843",
    "60ffa5cd5a5c3b741050ff86dd00ebe077079bfb9fb2c3f1469c0477fa16eaf6b3a95efab53fb93faf13ac1a95050000a7d2ffed2df2",
    "330c533bc0fdffbcffb61f1fe5b3bdf179e4cbf2d7efbff693f797b10cc365390f3ffbf5ecfd93f317d7f3ebf3175cbfec677bfd25e7",
    "16cfebfd8cebe259d7abbe9aabde675f95b36bad67d75aefa65edd5d2b7dd65aef937ad1cd453717ef7ef5421e370d7bb1c6ca3552ea",
    "f077595f69b5dc1ad34715d7231ed77645491733c93bf03df470bbb567044d751bbc2e864c15feaf5a996c64a9144ca14bf81ddeeae2",
    "1fd4ef74180674fd1fdaf9f5adf1ffd57f8d5b5eac4b7f899e8c256a3291ecdb6f7ddfe4443a90c69b10ebb43997dbfaf8f9a59feb19",
    "ac26555001003895fef8c34dbe76c3d40e10feaa3299f3c68df36bbaf1a67cc7ac51a62e31a41cfa8ae3ec37fb50f52c6af7a58fe7f3",
    "673f1fbff1f77cccdfe3b1f4cb2c0ffe753c86c73c9b273d3c18d73ce9e14193f9d18477df3ec9db58da9d09afd47e10dc7fbb50f7f9",
    "de7b5567ce1041d5314421bd9205c2df43c4974fa5b518bba85506fd2dbbe48b777349a235dc1b9fdefad3d68f37e783ffde87c88c00",
    "88fc06ff5fde32c77fb1d1a7e3e8518cc6e35f3e1e1d43b3d6d63fe2aefddb4ffe4e008c0a9504aa5201a7d2ffed2df2330c533bc0fd",
    "9feba82f5f4636faeb29e7ed7b2fff67f9f0e3fbfd31b2ff8df36bdad2d7c92fd83afbd96c9dfd8c67dd5bf5199cd7e705cffb79adbf",
    "e497f57acebfacd7739e735d5c5d6bd7bbd2f5a49b93b3398bb33969fae7fc847efed49d96bc95c25fda5ab97a6ed70c205d79a2d0c9",
    "d70e96e354f517178b7f38acffc2e9607f4abc767a7c4fccb78c227d093dffcdd92a52342d636be90fb548feccd294a8bf44e5a41b68",
    "48b95bfd9f0f6ff9cd7afc03373a0c0210807bb016c509fdcb4bbf9f2d58196f86dd6fdaa49f8ffc3e637bbde6a4fe66062e3b14702a",
    "fdf970f05513a67680ed67cffd7fdb1af9ba37f8ffe6fffc2f9f6f97ffdffecfffb2fed773ffbffdaf3f637d5dc627431f1ba7dcdd69",
    "f73e0fe577e3fd32ffcc57f9dd78bfcccf83dca566e68444d7bdf6d1d7399c439d57d7ae5d1fcfc7f480ca3bbcc3f3f17c3c1fcfc7a5",
    "c8dd2307c7f9405df74bf1fc8be3f71766d8fa5f59f6184ac0c00f5d727697cb7cb5677b5f364a6cb17738342eaa29c35a73d3ee9466",
    "eb779d069374cd2f3e56e5d4a4acf6a098dd335df522c17bfff8b6f7e9f65b57fc24982b415f9a7c7c7c3cd76d05808c8caf39f487cc",
    "5e9ad771bbd9de4a2cf6def63cfb1a82c37436ff79a835f6ec2a09a7fe2b3f7dbb17315ceeb5e513f6be7b77307caa2f1f8c9152ab8d",
    "0f656f35d4f4e583c3fae5738d835da3fc3b2bf2f6fc553dff9dbadf9ebf2c71feffeafabff397e587ff75fddf8ff3fc1ce8aba49e95",
    "ae8dca1b00c28d5baf96fb694dd3e7e17cafb059cf3f093eee3e1a5f774f87e7d97d71341f6dee47be9f3215f7f03dc1fafea2c177a7",
    "2879ae7752aeafb99a6e1236f69a4deab013a58fe222299e957ef7138c4e3914077505",
);

const STAIR_OGGFLAC_HEX: &str = concat!(
    "4f676753000200000000000000003381391200000000f78e1a3001337f464c414301000001664c614300000022120012000000000036",
    "160bb8017000000000000000000000000000000000000000004f676753000000000000000000003381391201000000ea5cfae9013784",
    "0000330d0000004c61766636322e31322e313032010000001a000000656e636f6465723d4c61766336322e32382e31303220666c6163",
    "4f6767530004200300000000000033813912020000006b90ac6d01e3fff87a0c00031ffd41000012e73ec40009249249249249249249",
    "249249249249249249249249249249249249249249249249249249200fffffffffffffffffffffffff00249249249249249249249249",
    "249249249249249249249249249249249249249249249249249001ffffffffffffffffffffffffe00001249249249249249249249249",
    "2492492492492492492492492492492492492492492492492480fffffffffffffffffffffffff0092492492492492492492492492492",
    "492492492492492492492492492492492492492492492407ffffffffffffffffffffffff80cadf",
);

// ---------------------------------------------------------------------------
// 工程/渲染夹具（形态照 `sfz_decoded_formats.rs`）
// ---------------------------------------------------------------------------

/// 临时目录。
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-sfz-ogg-{tag}-{}-{}",
        std::process::id(),
        EntityId::new().to_canonical_string()
    ));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

/// 32f 位深让"落盘"这一步**不引入抖动**（`quantize` 对 `Float32` 是透传）。
fn float_options() -> yeban_app::export_wav::WavExportOptions {
    yeban_app::export_wav::WavExportOptions {
        bit_depth: BitDepth::Float32,
        ..yeban_app::export_wav::WavExportOptions::default()
    }
}

/// 写一份**多段常量**的 `24-bit` 单声道 WAV（⛔ 不是单一常量 ⇒ 指纹有区分力）。
fn write_segments_wav(path: &Path, segments: &[(f32, usize)], sample_rate: u32) {
    let values: Vec<i32> = segments
        .iter()
        .flat_map(|(value, frames)| std::iter::repeat_n((I24_SCALE * value) as i32, *frames))
        .collect();
    write_plain_wav(
        path,
        &PcmFormat::integer(1, sample_rate, 24),
        &PcmBuffer::Int24(values),
    )
    .expect("写 WAV 夹具");
}

/// 写一份 `24-bit` 立体声 WAV（左右声道**逐帧交替** ⇒ 折单声道那一步必须取声道 0）。
fn write_stereo_wav(path: &Path, left: &[f32], right: &[f32], sample_rate: u32) {
    assert_eq!(left.len(), right.len(), "两声道帧数必须相同");
    let mut values: Vec<i32> = Vec::with_capacity(left.len() * 2);
    for (l, r) in left.iter().zip(right) {
        values.push((I24_SCALE * l) as i32);
        values.push((I24_SCALE * r) as i32);
    }
    write_plain_wav(
        path,
        &PcmFormat::integer(2, sample_rate, 24),
        &PcmBuffer::Int24(values),
    )
    .expect("写立体声 WAV 夹具");
}

/// 工程里那条**带 MIDI 音符**的非主总线轨。
fn midi_track_of(project: &YebanProjectV1) -> EntityId {
    *project
        .tracks
        .iter()
        .find(|(id, track)| {
            **id != project.master_bus_track_id
                && track.clips.values().any(|placement| {
                    project
                        .clip_pool
                        .get(&placement.clip_id)
                        .is_some_and(|entry| entry.content.notes().is_some())
                })
        })
        .map(|(id, _)| id)
        .expect("filled_project 必须有一条带 MIDI 音符的非主总线轨")
}

/// 那条轨上出现过的全部音高（排序 ＋ 去重）。
fn pitches_of(project: &YebanProjectV1, track_id: EntityId) -> Vec<u8> {
    let mut pitches: Vec<u8> = project
        .tracks
        .get(&track_id)
        .expect("轨必须在")
        .clips
        .values()
        .filter_map(|placement| project.clip_pool.get(&placement.clip_id))
        .filter_map(|entry| entry.content.notes())
        .flat_map(|notes| notes.values().map(|note| note.pitch))
        .collect();
    pitches.sort_unstable();
    pitches.dedup();
    assert!(!pitches.is_empty(), "夹具必须真的带音符");
    pitches
}

/// 写一份"每个工程音高一条区域、全部指同一份 `sample`"的 SFZ。
fn write_sfz(dir: &Path, name: &str, pitches: &[u8], sample: &str) -> PathBuf {
    let mut text = String::new();
    for pitch in pitches {
        text.push_str(&format!(
            "<region> sample={sample} key={pitch} pitch_keycenter={pitch}\n"
        ));
    }
    let path = dir.join(name);
    std::fs::write(&path, text).expect("写 SFZ 夹具");
    path
}

/// 把工程那条 MIDI 轨的 `sampler` 指向 `sample` 的 SFZ，然后**导出一次**。
fn render_bound(dir: &Path, sfz_name: &str, sample: &str) -> RenderProjectOutput {
    let mut project = yeban_model::samples::filled_project();
    let lead = midi_track_of(&project);
    let pitches = pitches_of(&project, lead);
    let sfz = write_sfz(dir, sfz_name, &pitches, sample);
    project.tracks.get_mut(&lead).expect("轨必须在").sampler = Some(SamplerBinding {
        path: sfz.to_string_lossy().into_owned(),
    });
    render_project_f32_with_assets(&project, float_options(), None, &[])
        .expect("样本读不到**不是**错误（区域静音 ＋ 计数）")
}

/// 非零样本**个数**（单位：个）。
fn nonzero(samples: &[f32]) -> usize {
    samples.iter().filter(|sample| **sample != 0.0).count()
}

/// FNV-1a 64（逐位；把每个 `f32` 的 4 个字节按小端卷进哈希）—— 与 `sfz_model_chain` 同一算式。
fn fnv1a_64(samples: &[f32]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for sample in samples {
        for byte in sample.to_bits().to_le_bytes() {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    hash
}

/// ⭐ 判据 ①（**本单核心**）：真 Ogg/Vorbis 样本走 SFZ 样本路径 ⇒ **真的出声**，
/// 且与"同一份 PCM 的 WAV 腿"按**实测的编解码器容差**对拍。
///
/// ## 夹具
///
/// * `stair.wav` —— 本判据用 `write_plain_wav` 现写的 `24-bit` 单声道八段阶梯（800 帧）；
/// * `stair.ogg` —— **同一份 PCM** 经参考编码器 `ffmpeg -c:a libvorbis -q:a 10` 转出
///   （5147 字节，SHA-256 见模块文档；⛔ **有损**，见下）。
///
/// ## 判定量
///
/// * OGG 那次的报告：`decoded_samples == 1`、`unreadable_samples` 空、
///   `silent_region_count == 0`（⭐ 这一条同时钉住"真 Ogg/Vorbis **不再**是 `UnsupportedFormat`"）；
/// * `nonzero(ogg) > 0` 且 `nonzero(wav) > 0`（两条腿都真的发声 ⇒ ⛔ 不是"缺样本 ⇒ 全静音"）；
/// * `max_abs_diff > 0`（有损 ⇒ 不可能逐位相同）且 `>= DENOMINATOR_GAP`
///   （⭐ 任务单 §5.1 的字面要求："差 ≥ 分母差那一项" ⇒ OGG 腿**确实不同**于 WAV 腿）；
/// * `max_abs_diff <= VORBIS_LOSSY_BOUND`（⭐ **实测**上界，⛔ 不是分母差那个量级）。
///
/// ## ⚠ 为什么上界不是 `peak × 分母差`（实测，⛔ 不是"放宽"）
///
/// FLAC 那条判据的上界能写成 `peak × 分母差 × 2` 是因为 **FLAC 无损**：两条腿解回来的是
/// **同一个整数**，差只出在归一化分母。**Vorbis 有损**（`symphonia-codec-vorbis` 是
/// `AudioBuffer<f32>`，见模块文档）⇒ 解码层与标称 PCM 的最大差实测 `1.51e-2`（`-q:a 10`，
/// 已是 libvorbis 最高档）。**逐位相等的判据在 Vorbis 上不可构造**；本单如实登记这一点，
/// 并用下面的判据 ② / ③ 把"同一条腿"与"同分母"分别**逐位**钉死。
///
/// ## 实测读数（本机 `aarch64/macos`，2026-10-16）
///
/// `wav_samples = ogg_samples = 768000`、`differing = 1598`（= 全部非零输出样本）、
/// `max_abs_diff = 6.628990173e-3`、`first_diff = (0, 0.10973183, 0.111011155)`、
/// `peak = 0.329195499`、`wav_nonzero = ogg_nonzero = 1598`、
/// `fnv1a_wav = 0x162634cbf9fddf99`、`fnv1a_ogg = 0x5a26f9f1339b6e45`、
/// OGG 那次报告 `samples=1 silent=0 unreadable=0`。
///
/// ⚠ **`#254` 复查（2026-10-20，本机 `aarch64/macos`）**：上表是 2026-10-16 的历史读数，
/// 其中的 `differing`／`nonzero`／指纹此后**已经漂移**（与 `#254` 无关的部分也存在，
/// 例如 `#243` 清自动化）。`#254` 当时的实测是：
/// `differing = 6392`、`max_abs_diff = 6.614714861e-3`、
/// `first_diff = Some((0, 0.07768417, 0.07858988))`、`peak = 0.328704327`、
/// `wav_nonzero = ogg_nonzero = 6392`、`fnv1a_wav = 0x26a43fe5bccb0a75`、
/// `fnv1a_ogg = 0xe1aaee04617eff11`。
/// ⛔ 本判据**不钉**这些数（Vorbis 有损 ⇒ 只用容差判），所以它们漂移不会变红。
///
/// ⚠ **怎么变红**：① 把分派关掉（`.ogg` 走 hound）⇒ OGG 版全静音 ⇒ `decoded_samples`
/// 与 `unreadable` 两条同时红；② 换掉夹具（SHA-256 不符）⇒ `fixture()` 红。
#[test]
fn an_ogg_vorbis_sample_is_decoded_end_to_end_and_matches_the_wav_leg_within_the_codec_tolerance() {
    let dir = scratch("ogg-e2e");
    write_segments_wav(&dir.join("stair.wav"), &STAIR_SEGMENTS, 48_000);
    std::fs::write(
        dir.join("stair.ogg"),
        fixture(
            STAIR_OGG_HEX,
            "323f46770d6b089768fb49acea0101943adc2e1b2a5ca733d9120132ca75345e",
            5147,
        ),
    )
    .expect("写 OGG 夹具");

    let wav = render_bound(&dir, "wav.sfz", "stair.wav");
    let ogg = render_bound(&dir, "ogg.sfz", "stair.ogg");

    // OGG 那一次必须**真的**走了通用解码腿（⛔ 不是"两边都缺样本 ⇒ 都静音"）。
    assert_eq!(ogg.sfz_decoded.len(), 1, "模型链条必须注入一条 SFZ 摊平表");
    let (_, report) = &ogg.sfz_decoded[0];
    assert_eq!(
        report.decoded_samples, 1,
        "`stair.ogg` 必须真的进提供表：{:?}",
        report.unreadable_sample_reasons
    );
    assert!(
        report.unreadable_samples.is_empty(),
        "⭐ 真 Ogg/Vorbis 不再落任何读失败档：{:?}",
        report.unreadable_sample_reasons
    );
    assert_eq!(report.silent_region_count, 0, "所有区域都必须有样本");

    // ---- 逐样本差（本单核心读数）----
    assert_eq!(
        wav.samples.len(),
        ogg.samples.len(),
        "两条腿的导出帧数必须相同"
    );
    let mut differing = 0usize;
    let mut max_abs_diff = 0.0_f32;
    let mut first_diff: Option<(usize, f32, f32)> = None;
    for (index, (left, right)) in wav.samples.iter().zip(&ogg.samples).enumerate() {
        if left.to_bits() != right.to_bits() {
            differing += 1;
            let difference = (left - right).abs();
            if difference > max_abs_diff {
                max_abs_diff = difference;
            }
            if first_diff.is_none() {
                first_diff = Some((index, *left, *right));
            }
        }
    }
    let wav_hash = fnv1a_64(&wav.samples);
    let ogg_hash = fnv1a_64(&ogg.samples);
    let peak = wav
        .samples
        .iter()
        .fold(0.0_f32, |acc, sample| acc.max(sample.abs()));
    println!(
        "[app-sfz-ogg/e2e] wav_samples={} ogg_samples={} differing={differing} \
         max_abs_diff={max_abs_diff:.9e} first_diff={first_diff:?} peak={peak:.9} \
         denominator_gap={DENOMINATOR_GAP:.9e} vorbis_lossy_bound={VORBIS_LOSSY_BOUND:.9e} \
         fnv1a_wav=0x{wav_hash:016x} fnv1a_ogg=0x{ogg_hash:016x} \
         wav_nonzero={} ogg_nonzero={} ogg_report={}",
        wav.samples.len(),
        ogg.samples.len(),
        nonzero(&wav.samples),
        nonzero(&ogg.samples),
        report.summary(),
    );

    assert!(
        nonzero(&wav.samples) > 0 && nonzero(&ogg.samples) > 0,
        "两条腿都必须真的发声（⛔ 否则任何'差'都不可判）"
    );
    let samples = &wav.samples;
    assert!(
        samples.iter().any(|sample| *sample < 0.0) && samples.iter().any(|sample| *sample > 0.0),
        "阶梯夹具必须同时有正负样本（⛔ 单一常量会让比较失去区分力）"
    );

    // ---- ⭐ OGG 腿**确实不同**于 WAV 腿，且差 ≥ 分母差那一项 ----
    assert!(
        max_abs_diff > 0.0,
        "⭐ Vorbis 有损 ⇒ 与 WAV 腿**必然**有位型不同；若为 0，说明夹具或腿被换过"
    );
    assert!(
        max_abs_diff >= DENOMINATOR_GAP,
        "⭐ 任务单 §5.1：差必须 ≥ 分母差（实得 {max_abs_diff:.9e} < {DENOMINATOR_GAP:.9e}）\
         ⇒ 那意味着 OGG 腿被判成了与 WAV 腿同一口径"
    );
    assert!(
        max_abs_diff <= VORBIS_LOSSY_BOUND,
        "⭐ 最大逐样本差必须 ≤ **实测的** Vorbis 容差（实得 {max_abs_diff:.9e}，\
         上界 {VORBIS_LOSSY_BOUND:.9e}，peak={peak:.9}）—— 超出即不是 Vorbis 量化能解释的\
         （查声道折法／增益／采样率／夹具被换）"
    );
    assert_ne!(
        wav_hash, ogg_hash,
        "有损 ⇒ 指纹不可能相同（若相同，说明 OGG 腿根本没解码）"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// ⭐ 判据 ②（**"同一条腿"判据**；⭐ 甲组最强的一条）。
///
/// ## 为什么这一条才是本单真正要的（⭐ 它是一条**恒等式**，⛔ 不是一个"容差内"的读数）
///
/// `#85` 登记的缺口不是"Vorbis 能不能解"（那是 symphonia 的事），而是"**SFZ 样本路径**
/// 到底有没有接上这条通用解码腿"。这条判据把它写成**恒等式**——左右两侧都是**逐位**比较
/// （`f32` 的 `==`），⛔ 不是"近似相等"、⛔ 不是"容差内"：
///
/// ```text
/// read_sample_file(<f>.ogg).pcm  ==  decode_path(<f>.ogg).samples().step_by(channels)
/// ```
///
/// 两件性质合起来：
///
/// 1. **走的是 `decode_path`**（symphonia 那条腿）—— ⛔ 不是 hound（`read_plain_wav_mono_f32`）。
///    hound 根本读不动 Ogg ⇒ 若分派把 `.ogg` 交给它，`read_sample_file` 会**返回错误**，
///    下面的 `.expect` 立刻红；
/// 2. **折单声道那一步是 `step_by(channels)` 取声道 0** —— 与 hound 腿**同一口径**
///    （`src/sfz_load.rs` 的 `decode_with_yeban_decode`）。
///
/// ## 判定量
///
/// * 逐位相等（`==`，⛔ 不是"近似相等"）：**800** 帧逐位相同；
/// * `read_sample_file` 上报的 `sample_hz == 48000`、`frames == 800`、`describe` 含
///   `frames=800` 与 `rate=48000`；
/// * **⛔ 不是 hound 那条腿**：逐位核对 OGG 腿的值**不**等于 `raw × 1/(2²³−1)`
///   （WAV 腿的公式）—— 用**同一个** `!=` 给出两侧证据。
///
/// ## 实测读数（本机 `aarch64/macos`，2026-10-16）
///
/// `frames = 800`、`channels = 1`、交织 `800`、折后 `800`、`sample_hz = 48000`、
/// `fnv1a_leg == fnv1a_decode == 0xb242c829a67e1884`（⭐ 两岸逐位相同）；
/// 标称第 0 帧的两个公式给 `0.25`（hound）／`0.25`（symphonia），而 OGG 腿实得
/// `2.529147e-1` ⇒ 两个 `!=` 都成立（Vorbis 有损）。
///
/// ⚠ **注入（应红）**：把期望样本乘 `1.0000001`（≈ `1 + 分母差`）⇒ 逐位比较**红**
/// （实测：1 红 5 绿，只有本条翻红）。
#[test]
fn the_ogg_leg_is_bit_identical_to_the_shared_decoder_not_the_hound_leg() {
    let dir = scratch("ogg-same-leg");
    std::fs::write(
        dir.join("stair.ogg"),
        fixture(
            STAIR_OGG_HEX,
            "323f46770d6b089768fb49acea0101943adc2e1b2a5ca733d9120132ca75345e",
            5147,
        ),
    )
    .expect("写 OGG 夹具");

    let (sample, frames, describe) =
        read_sample_file(&dir.join("stair.ogg")).expect("真 Ogg/Vorbis 必须可读");
    let asset =
        decode_path(&dir.join("stair.ogg"), &DecodeOptions::default()).expect("直接解码 OGG 夹具");
    let channels = usize::from(asset.channels().max(1));
    let expected: Vec<f32> = asset.samples().iter().step_by(channels).copied().collect();

    println!(
        "[app-sfz-ogg/same-leg] frames={frames} channels={channels} interleaved={} \
         folded={} sample_hz={} describe={describe} fnv1a_leg=0x{:016x} fnv1a_decode=0x{:016x}",
        asset.samples().len(),
        expected.len(),
        sample.sample_hz,
        fnv1a_64(&sample.pcm),
        fnv1a_64(&expected),
    );

    assert_eq!(channels, 1, "单声道夹具：这条判据在 `channels == 1` 上比较");
    assert_eq!(frames, 800, "折单声道后必须是 800 帧");
    assert_eq!(sample.pcm.len(), 800, "PCM 长度也必须折到 800");
    assert_eq!(sample.sample_hz, 48_000.0, "采样率原样上报");
    assert!(
        describe.contains("frames=800") && describe.contains("rate=48000"),
        "描述必须写折后帧数与文件采样率：{describe}"
    );

    // ---- ⭐ 逐位：SFZ 样本腿 ≡ 共享解码器的折声道 0 ----
    assert_eq!(
        sample.pcm.len(),
        expected.len(),
        "折声道 0 之后的长度必须与 `decode_path` 的折法一致"
    );
    assert_eq!(
        sample.pcm.as_slice(),
        expected.as_slice(),
        "⭐ `read_sample_file` 必须**逐位**等于 `decode_path(..).samples().step_by(channels)`\
         （这**就是**「`#85` 那条腿」的定义）"
    );

    // ---- ⛔ 它**不是** hound 腿：逐位核对 OGG 腿的值 ≠ `raw × 1/(2²³−1)` ----
    let nominal = stair_values();
    let raw0 = (I24_SCALE * nominal[0]) as i32;
    let hound_side = raw0 as f32 * (1.0 / 8_388_607.0);
    let symphonia_side = raw0 as f32 * (1.0 / 8_388_608.0);
    assert_ne!(
        sample.pcm[0], hound_side,
        "⛔ OGG 腿**不是** hound 腿：它不可能等于 `raw × 1/(2²³−1)`（实得 {:?}）",
        sample.pcm[0]
    );
    assert_ne!(
        sample.pcm[0], symphonia_side,
        "⚠ Vorbis **有损** ⇒ 它也不等于 `raw × 1/2²³`（那是 FLAC 那条无损腿的公式）；\
         实得 {:?}，两个标称公式分别给 {hound_side:?} / {symphonia_side:?}",
        sample.pcm[0]
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// ⭐ 判据 ③（**§3.3 的"同分母"判据**；乙组：Ogg 容器 ＋ **无损**载荷）。
///
/// ⚠ ⚠ **本条测的是 Ogg 容器 ＋ FLAC 载荷**（无损）；⛔ **不证明 Vorbis 解码**。
/// ⛔ **不许把本条读成"Vorbis 验过了"** —— Vorbis 端到端由**甲组**（判据 ①②④⑤）独立闭合，
/// 本条只是**补充**：它测的是 `is_wav_suffix == false` 那个**同一个分派分支** ＋ 分母口径。
///
/// ## 为什么需要它（⛔ 它不是甲组的替代品）
///
/// 任务单 §3.3 原先要求机械核对"**OGG 腿必须落同一侧**（与 FLAC 腿**同分母**）"。甲组用真
/// Vorbis ⛔ 做不到：Vorbis 有损且解出 `f32`（⛔ 没有 `2²³` 分母）。但**这条性质本身是可判
/// 的** —— 只要载荷无损：`ffmpeg -c:a flac -f ogg` 产出一份**真 Ogg 容器**（417 字节），
/// 后缀 `.ogg` ⇒ 走 `is_wav_suffix == false` 那个**同一个**分支 ⇒ **同一条腿**。
/// 于是 FLAC 先例那条逐位判据可以**原样照搬**，只把 `.flac` 换成 `.ogg`。
///
/// ⚠ **它证明的是腿与分母，⛔ 不证明 Vorbis 解码**（那由甲组的 ①/②/④/⑤ 负责）。
/// ⚠ 它是**合法真 Ogg 文件**（Ogg ＋ FLAC 映射是标准容器映射），⛔ 不是"假夹具"。
///
/// ## 实测读数（本机 `aarch64/macos`）
///
/// | 时间点 | `differing` | `max_abs_diff` | `fnv1a_wav` | `fnv1a_oggflac` |
/// | :--- | :--- | :--- | :--- | :--- |
/// | `#254` 之前（2026-10-16） | `1598` | `5.960464478e-8`（= `0.5 × 分母差`） | `0x162634cbf9fddf99` | `0xb9234f5509b02141` |
/// | **`#254` 之后（2026-10-20）** | **`0`** | **`0.000000000e0`** | **`0x26a43fe5bccb0a75`** | **`0x26a43fe5bccb0a75`** |
///
/// 报告 `samples=1 silent=0 unreadable=0`；`peak = 0.328704327`。
/// 逐位核对八段的公式**全过**（`#254` 之后两条腿是同**一个**公式 `raw × 1/2²³`）。
///
/// ## 夹具
///
/// * `stair.wav` —— `24-bit` 单声道八段阶梯（800 帧）；
/// * `stair_oggflac.ogg` —— **同一份 PCM** 经 `ffmpeg -c:a flac -f ogg` **无损**转出。
///
/// ## 判定量
///
/// * OGG 那次的报告：`decoded_samples == 1`、`unreadable_samples` 空、`silent_region_count == 0`；
/// * **同分母（机械化）**：对八段的 `raw` 值逐位核对
///   `wav_pcm == raw as f32 × (1/8388608)` **且** `ogg_pcm == raw as f32 × (1/8388608)`
///   —— ⭐ `#254` 之前 WAV 腿是 `1/8388607`，那正是两腿相差 `5.960464478e-8` 的全部原因；
/// * **渲染层**：`max_abs_diff == 0` **且** `fnv1a_wav == fnv1a_oggflac`
///   （同分母 ⇒ 无损两面必须逐位相同）。
#[test]
fn an_ogg_container_with_a_lossless_flac_payload_is_bit_identical_to_the_wav_leg() {
    let dir = scratch("ogg-flac-payload");
    write_segments_wav(&dir.join("stair.wav"), &STAIR_SEGMENTS, 48_000);
    std::fs::write(
        dir.join("stair_oggflac.ogg"),
        fixture(
            STAIR_OGGFLAC_HEX,
            "6e8befbb746d3a6fa6988efd9613e3fd4415e4c0a718a8571fd8b1703e7811e4",
            417,
        ),
    )
    .expect("写 Ogg+FLAC 夹具");

    let wav = render_bound(&dir, "wav.sfz", "stair.wav");
    let ogg = render_bound(&dir, "ogg.sfz", "stair_oggflac.ogg");

    assert_eq!(ogg.sfz_decoded.len(), 1, "模型链条必须注入一条 SFZ 摊平表");
    let (_, report) = &ogg.sfz_decoded[0];
    assert_eq!(
        report.decoded_samples, 1,
        "`stair_oggflac.ogg` 必须真的进提供表：{:?}",
        report.unreadable_sample_reasons
    );
    assert!(
        report.unreadable_samples.is_empty(),
        "⭐ `.ogg` 后缀必须走通用解码腿（⛔ 不是 hound）：{:?}",
        report.unreadable_sample_reasons
    );
    assert_eq!(report.silent_region_count, 0, "所有区域都必须有样本");

    assert_eq!(
        wav.samples.len(),
        ogg.samples.len(),
        "两条腿的导出帧数必须相同"
    );
    let mut differing = 0usize;
    let mut max_abs_diff = 0.0_f32;
    for (left, right) in wav.samples.iter().zip(&ogg.samples) {
        if left.to_bits() != right.to_bits() {
            differing += 1;
            max_abs_diff = max_abs_diff.max((left - right).abs());
        }
    }
    let wav_hash = fnv1a_64(&wav.samples);
    let ogg_hash = fnv1a_64(&ogg.samples);
    let peak = wav
        .samples
        .iter()
        .fold(0.0_f32, |acc, sample| acc.max(sample.abs()));

    // ---- ⭐ 同分母（机械化）：两条腿都必须是 `raw × 1/2²³`（`#254` 统一） ----
    let (wav_pcm, wav_rate, _) =
        read_plain_wav_mono_f32(dir.join("stair.wav")).expect("回读 WAV 夹具");
    let asset = decode_path(&dir.join("stair_oggflac.ogg"), &DecodeOptions::default())
        .expect("直接解码 Ogg+FLAC 夹具");
    assert_eq!(wav_rate, 48_000);
    assert_eq!(asset.sample_rate(), 48_000);
    assert_eq!(wav_pcm.len(), asset.samples().len());
    let mut cursor = 0usize;
    for (value, frames) in STAIR_SEGMENTS {
        let raw = (I24_SCALE * value) as i32;
        assert_eq!(
            wav_pcm[cursor],
            raw as f32 * (1.0 / 8_388_608.0),
            "WAV 腿必须**逐位**是 `raw × 1/2²³`（`#254` 起；raw={raw}，段值={value}）"
        );
        assert_eq!(
            asset.samples()[cursor],
            raw as f32 * (1.0 / 8_388_608.0),
            "⭐ `.ogg` 腿必须**逐位**是 `raw × 1/2²³` —— 与 WAV／FLAC 腿**同分母**\
             （raw={raw}，段值={value}）"
        );
        assert_eq!(
            wav_pcm[cursor],
            asset.samples()[cursor],
            "⭐ 同一个 `raw` 的两条腿必须**逐位**相同（`#254`；raw={raw}）"
        );
        cursor += frames;
    }

    println!(
        "[app-sfz-ogg/denominator] wav_samples={} ogg_samples={} differing={differing} \
         max_abs_diff={max_abs_diff:.9e} peak={peak:.9} bound={:.9e} \
         denominator_gap={DENOMINATOR_GAP:.9e} fnv1a_wav=0x{wav_hash:016x} \
         fnv1a_oggflac=0x{ogg_hash:016x} ogg_report={}",
        wav.samples.len(),
        ogg.samples.len(),
        peak * DENOMINATOR_GAP * 2.0,
        report.summary(),
    );

    assert_eq!(
        max_abs_diff,
        0.0,
        "⭐ 两条腿同分母（都是 `2²³`）⇒ 无损两面**必须**逐位相同；若 > 0，说明有一条腿的\
         分母被改回 `2²³ − 1`（实得 {max_abs_diff:.9e}；旧口径的界是 {:.9e}）",
        peak * DENOMINATOR_GAP * 2.0
    );
    assert_eq!(
        wav_hash, ogg_hash,
        "同分母 ⇒ FNV 指纹必须相同（wav=0x{wav_hash:016x} ogg=0x{ogg_hash:016x}）"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// ⭐ 判据 ④（**折单声道口径**；⭐ 注入 ② 的靶子）：立体声 **OGG/Vorbis** 与 WAV **同款**
/// 取声道 0。
///
/// ## 夹具
///
/// * `stereo44.wav` —— 逐帧交替 `(阶梯, −0.5)` 的**立体声** `44.1 kHz` WAV（走既有 hound 腿）；
/// * `stereo44.ogg` —— **同一份立体声 PCM** 的 Vorbis（5867 字节）。
///
/// ## 判定量
///
/// * 夹具自证：`decode_path(stereo44.ogg)` ⇒ `channels == 2`、交织样本 `1600`、
///   且**第 0 与第 1 个样本不同**（⛔ 若夹具退化成单声道，本判据测不出任何东西）；
/// * **WAV 腿内部一致**：立体声 WAV 的导出指纹 == 单声道 WAV 参照（既有口径：取**声道 0**）；
/// * **OGG 腿内部一致**：`read_sample_file(stereo44.ogg)` 折后 `800` 帧（⛔ 不是 `1600`），
///   且**逐位**等于 `decode_path(stereo44.ogg).samples().step_by(2)`。
///
/// ## 实测读数（本机 `aarch64/macos`，2026-10-16）
///
/// `channels = 2`、交织 `1600`、折后 `800`、`sample_hz = 44100`、
/// `s0 = 2.5238612e-1`、`s1 = −5.052742e-1`（左右**不同**）。
///
/// ⚠ **注入 ②**：把 `decode_with_yeban_decode` 里的 `step_by(channels)` 改成 `step_by(1)`
/// （交织直传）⇒ 折后变 `1600` 帧 ⇒ 帧数断言与逐位断言**同时红**。
#[test]
fn a_stereo_ogg_vorbis_is_folded_to_channel_zero_like_the_wav_leg() {
    let dir = scratch("ogg-stereo");
    let left = stair_values();
    let right = vec![-0.5_f32; left.len()];
    write_segments_wav(&dir.join("stair.wav"), &STAIR_SEGMENTS, 48_000);
    write_stereo_wav(&dir.join("stereo.wav"), &left, &right, 48_000);
    std::fs::write(
        dir.join("stereo44.ogg"),
        fixture(
            STEREO44_OGG_HEX,
            "10977ef07aaf409c4117896a7068dd1430cbd268118b69e96988e38c4989a7e9",
            5867,
        ),
    )
    .expect("写立体声 OGG 夹具");

    // ---- 夹具自证：它**真的**是立体声，且左右不同（否则注入 ② 测不出东西）----
    let asset = decode_path(&dir.join("stereo44.ogg"), &DecodeOptions::default())
        .expect("立体声 OGG 夹具必须能解");
    assert_eq!(asset.channels(), 2, "夹具必须是 2 声道");
    assert_eq!(
        asset.samples().len(),
        1_600,
        "夹具必须是 800 帧 × 2 声道的交织"
    );
    assert_ne!(
        asset.samples()[0],
        asset.samples()[1],
        "夹具的左右声道必须不同（⛔ 否则'折单声道'那一步不可判）"
    );

    // ---- WAV 腿的"取声道 0"参照（既有口径，本单⛔ 不动）----
    let mono_wav = render_bound(&dir, "mono-wav.sfz", "stair.wav");
    let stereo_wav = render_bound(&dir, "stereo-wav.sfz", "stereo.wav");

    // ---- OGG 腿：折后帧数与逐位口径 ----
    let (sample, frames, describe) =
        read_sample_file(&dir.join("stereo44.ogg")).expect("立体声 OGG 必须可读");
    let expected: Vec<f32> = asset.samples().iter().step_by(2).copied().collect();
    let ogg = render_bound(&dir, "stereo-ogg.sfz", "stereo44.ogg");
    let (_, report) = &ogg.sfz_decoded[0];
    assert_eq!(
        report.decoded_samples, 1,
        "立体声 OGG 的样本必须真的进提供表：{:?}",
        report.unreadable_sample_reasons
    );

    let mono_wav_hash = fnv1a_64(&mono_wav.samples);
    let stereo_wav_hash = fnv1a_64(&stereo_wav.samples);
    let ogg_hash = fnv1a_64(&ogg.samples);
    println!(
        "[app-sfz-ogg/stereo] wav_leg mono=0x{mono_wav_hash:016x} stereo=0x{stereo_wav_hash:016x} \
         ogg_leg fnv1a=0x{ogg_hash:016x} folded_frames={frames} interleaved_before_fold=1600 \
         channels={} s0={:?} s1={:?} sample_hz={} describe={describe} ogg_nonzero={}",
        asset.channels(),
        asset.samples()[0],
        asset.samples()[1],
        sample.sample_hz,
        nonzero(&ogg.samples),
    );

    assert_eq!(
        frames, 800,
        "折单声道后必须是 800 帧（⛔ 交织直传会是 1600）"
    );
    assert_eq!(sample.pcm.len(), 800, "PCM 长度也必须折到 800");
    assert_eq!(sample.sample_hz, 44_100.0, "采样率原样上报");
    assert!(
        describe.contains("frames=800"),
        "描述必须写折后帧数：{describe}"
    );
    assert!(
        sample.pcm.as_slice() == expected.as_slice(),
        "⭐ OGG 腿必须**逐位**等于 `decode_path(..).samples().step_by(2)`（取声道 0）—— \
         若不等，最可能是把**交织**样本当成了单声道（实得 {} 项 vs 期望 {} 项）",
        sample.pcm.len(),
        expected.len()
    );

    assert!(nonzero(&mono_wav.samples) > 0, "WAV 参照必须真的发声");
    assert_eq!(
        stereo_wav_hash, mono_wav_hash,
        "既有 WAV 腿对立体声也是**取声道 0**（0x{stereo_wav_hash:016x} vs 0x{mono_wav_hash:016x}）"
    );
    assert!(
        nonzero(&ogg.samples) > 0,
        "立体声 OGG 折单声道后必须照常发声"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// ⭐ 判据 ⑤（**采样率不匹配的处理方式**）：`44.1 kHz` 的 **Ogg/Vorbis** **不重采样**，
/// 原样上报文件自己的采样率；播放速率的换算交给引擎既有口径（`RenderRates::new`）。
///
/// ## 为什么是"报数"而不是"重采样"
///
/// 与 `sfz_decoded_formats.rs` 的 FLAC 判据③**同一条理由**：WAV 腿从来把文件采样率原样
/// 放进 `InMemorySample::sample_hz`，由 `yeban_engine::sfz_instrument` 的
/// `RenderRates::new(source.sample_hz, engine_hz)` 把它算进**播放速率**。新腿沿用**同一个**
/// 口径 ⇒ 两条腿对同一份 PCM 给同一个音频结果（⛔ 不引入第二条口径）。
///
/// ## 夹具
///
/// `stereo44.ogg` —— **立体声** `44.1 kHz`（`800` 帧／声道）。本判据只取它的**采样率面**
/// （折单声道那一面由判据 ④ 负责；⭐ 同一份夹具 ⇒ 少一个二进制面）。
///
/// ## 判定量
///
/// * `sample_hz == 44100`（⛔ 不是工程采样率 `48000`）；
/// * `frames == 800`（⛔ 不是 `800 × 48000 / 44100 ≈ 871` ⇒ 读层没重采样）；
/// * `describe` 含 `rate=44100` 且含 `frames=800`；
/// * 端到端仍**出声**（`nonzero > 0`）且 `decoded_samples == 1`。
#[test]
fn a_44k_ogg_vorbis_is_read_at_its_own_rate() {
    let dir = scratch("ogg-44k");
    std::fs::write(
        dir.join("stereo44.ogg"),
        fixture(
            STEREO44_OGG_HEX,
            "10977ef07aaf409c4117896a7068dd1430cbd268118b69e96988e38c4989a7e9",
            5867,
        ),
    )
    .expect("写 44.1 kHz OGG 夹具");

    let asset = decode_path(&dir.join("stereo44.ogg"), &DecodeOptions::default())
        .expect("44.1k OGG 必须能解");
    assert_eq!(asset.sample_rate(), 44_100, "夹具必须是 44.1 kHz");

    let (sample, frames, describe) =
        read_sample_file(&dir.join("stereo44.ogg")).expect("44.1k OGG 必须可读");
    assert_eq!(
        sample.sample_hz, 44_100.0,
        "⭐ 上报**文件自己**的采样率（⛔ 不是工程采样率 48000）"
    );
    assert_eq!(
        frames, 800,
        "⭐ 读层**不重采样**（若见 871 一类数字 ⇒ 有人在这里插了 sinc）"
    );
    assert_eq!(sample.pcm.len(), 800);
    assert!(
        describe.contains("rate=44100") && describe.contains("frames=800"),
        "描述必须写文件采样率与折后帧数：{describe}"
    );

    let out = render_bound(&dir, "m44.sfz", "stereo44.ogg");
    let (_, report) = &out.sfz_decoded[0];
    println!(
        "[app-sfz-ogg/44k] sample_hz={} frames={frames} describe={describe} \
         nonzero={} report={}",
        sample.sample_hz,
        nonzero(&out.samples),
        report.summary()
    );
    assert_eq!(report.decoded_samples, 1, "44.1k OGG 必须真的被解码");
    assert!(
        nonzero(&out.samples) > 0,
        "44.1k 样本必须照常发声（播放速率由引擎按 sample_hz/engine_hz 换算）"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// ⭐ 判据 ⑥（**可判别性**；§5.4）：坏的 OGG 落**哪一档**是**实测**出来的，⛔ 不是猜的；
/// 并且**同一批数据里必须有一个合法 OGG 被正确接受**（⛔ 否则"全都读不到"也能让本判据绿）。
///
/// ## 三份夹具（同一条 SFZ 摊平表里的三条区域）
///
/// | `sample=` | 盘上是什么 | ⭐ 实测档 | ⭐ 实测 `detail` |
/// | :--- | :--- | :--- | :--- |
/// | `good.ogg` | 真 Ogg/Vorbis（5147 字节，判据 ① 那份） | （**读成功**） | — |
/// | `truncated.ogg` | 上面那份**去掉最后一个字节** | [`SampleReadFailure::Unreadable`] | 含 `unexpected end of file` |
/// | `fake.ogg` | 任意文本，后缀 `.ogg` | [`SampleReadFailure::UnsupportedFormat`] | 探测器认不出容器 |
///
/// ⚠ **两条都是实测**（不是从 `classify_decode_failure` 的表格推的）：
/// * 截断 Ogg ⇒ `DecodeError::Io(Custom { kind: UnexpectedEof, .. })` ⇒ `Io` 且 `kind != NotFound`
///   ⇒ `Unreadable`。实测对**从 1 字节到一半**的每一处截断都成立（见报告）；
/// * 假 Ogg ⇒ `DecodeError::UnsupportedFormat` ⇒ `UnsupportedFormat`。
///
/// ## 判定量（单位：条／枚举）
///
/// * `decoded_samples == 1`（`good.ogg` **真的读了**）；
/// * `unreadable_not_found_count == 0`、`unreadable_unsupported_format_count == 1`、
///   `unreadable_other_count == 1`；
/// * `silent_region_count == 2`（引擎口径：缺样本的区域数）。
///
/// ## 实测读数（本机 `aarch64/macos`，2026-10-16）
///
/// 报告行逐字：`regions=3 samples=1 silent=2 missing=["fake.ogg", "truncated.ogg"]
/// unreadable=2 unreadable_kind(not_found=0,unsupported=1,other=1)`；
/// `truncated.ogg` 的 `detail` 尾部 = `` `…/truncated.ogg`: io error: unexpected end of file ``；
/// `fake.ogg` 的 `detail` 尾部 = `` `…/fake.ogg`: unrecognised or unsupported container format ``。
#[test]
fn a_broken_ogg_is_classified_and_a_valid_ogg_in_the_same_batch_is_accepted() {
    let dir = scratch("ogg-reasons");
    let good = fixture(
        STAIR_OGG_HEX,
        "323f46770d6b089768fb49acea0101943adc2e1b2a5ca733d9120132ca75345e",
        5147,
    );
    // ① 合法 Ogg/Vorbis（正向对照：这一条必须**读成功**）。
    std::fs::write(dir.join("good.ogg"), &good).expect("写合法 OGG");
    // ② 截断：去掉最后一个字节（⛔ 不是"随便切一半"，那会连页头都切掉）。
    std::fs::write(dir.join("truncated.ogg"), &good[..good.len() - 1]).expect("写截断 OGG");
    // ③ 假 Ogg：后缀对、内容任意文本。
    std::fs::write(
        dir.join("fake.ogg"),
        b"this is not an ogg container at all\n",
    )
    .expect("写假 OGG");

    let mut text = String::new();
    for (offset, sample) in ["good.ogg", "truncated.ogg", "fake.ogg"].iter().enumerate() {
        let key = 60 + offset;
        text.push_str(&format!(
            "<region> sample={sample} key={key} pitch_keycenter={key}\n"
        ));
    }
    let sfz = dir.join("reasons.sfz");
    std::fs::write(&sfz, text).expect("写 SFZ 夹具");

    let (_plan, report) = decode_sfz_plan(&sfz, 48_000.0).expect("装载必须成功（读不到不是错误）");
    println!("[app-sfz-ogg/reasons] {}", report.summary());
    for entry in &report.unreadable_sample_reasons {
        println!(
            "[app-sfz-ogg/reasons]   {} => {} （{}）",
            entry.path, entry.reason, entry.detail
        );
    }

    assert_eq!(
        report.referenced_sample_paths.len(),
        3,
        "三条区域各指一份样本：{:?}",
        report.referenced_sample_paths
    );
    assert_eq!(
        report.decoded_samples, 1,
        "只有 `good.ogg` 该被读成功（⛔ 否则本判据比的是「全都读不到」）"
    );

    let reason_of = |suffix: &str| -> Option<SampleReadFailure> {
        report
            .unreadable_sample_reasons
            .iter()
            .find(|entry| entry.path.ends_with(suffix))
            .map(|entry| entry.reason)
    };
    assert_eq!(
        reason_of("truncated.ogg"),
        Some(SampleReadFailure::Unreadable),
        "⭐ 截断的真 Ogg ⇒ 容器认得、内容坏 ⇒ `Unreadable`（实测 `UnexpectedEof`）"
    );
    assert_eq!(
        reason_of("fake.ogg"),
        Some(SampleReadFailure::UnsupportedFormat),
        "⭐ 假 Ogg（探测器认不出容器）⇒ `UnsupportedFormat`（实测）"
    );

    let detail_of = |suffix: &str| -> String {
        report
            .unreadable_sample_reasons
            .iter()
            .find(|entry| entry.path.ends_with(suffix))
            .map(|entry| entry.detail.clone())
            .expect("该条必须在读失败表里")
    };
    assert!(
        detail_of("truncated.ogg").contains("unexpected end of file"),
        "⭐ 截断的 `detail` 必须带上读取器原文（实测）：{}",
        detail_of("truncated.ogg")
    );
    assert!(
        !detail_of("fake.ogg").is_empty(),
        "⭐ 假 Ogg 的 `detail` 必须非空：{}",
        detail_of("fake.ogg")
    );

    assert_eq!(report.unreadable_not_found_count(), 0, "文件都存在 0 条");
    assert_eq!(
        report.unreadable_unsupported_format_count(),
        1,
        "格式不受支持 1 条（`fake.ogg`）"
    );
    assert_eq!(
        report.unreadable_other_count(),
        1,
        "容器认得、内容坏 1 条（`truncated.ogg`）"
    );
    assert_eq!(
        report.silent_region_count, 2,
        "缺样本 ⇒ 两个区域静音（`truncated.ogg` ＋ `fake.ogg`）"
    );

    std::fs::remove_dir_all(&dir).ok();
}
