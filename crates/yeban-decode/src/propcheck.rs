//! 属性测试（`proptest`）：给纯逻辑层的判据加一层"随机输入"覆盖 [AGENTS.md §3 DoD 2]。
//!
//! 为什么把属性测试单独放一个文件：本 crate 的 [`crate::limits`] 与 [`crate::duration`]
//! 是**零第三方依赖**的，因此可以在本机用 `rustc --edition 2024 --test` 单独编译执行
//! （见 notes §7）。若把 `proptest` 写进那两个文件的 `#[cfg(test)]` 块，它们就再也
//! 无法脱离依赖树编译，本机可验证的面积会**变小**。所以属性测试住在这里，只由
//! `cargo test` 拉起。
//!
//! 四类属性分别对应四条硬约束：
//! 1. 尺寸算术：`frames × channels` 要么精确，要么明确溢出 —— **绝不回绕**；
//! 2. 长度契约：任何采样率比例下区间都自洽，且"恰好越界"必须被拒（判据不能是空的）；
//! 3. 时长对账：对称、只把相等判为 `Exact`、`is_reconciled` 等价于"差在容差内"；
//! 4. **不可信输入零 panic**：生产入口 `decode_bytes` 面对任意字节只允许"类型化错误或
//!    自洽资产"两种结果；同一份字节两次解码的 `pcm_hash` 必须相同。
//!
//! 第 4 条**经过** `symphonia`。这不影响 `crate::limits` / `crate::duration` / `crate::testfix`
//! 的"可脱离依赖树单独编译"性质：那三个模块不引用本文件。

use proptest::prelude::*;

use crate::decode::{self, DecodeOptions};
use crate::duration::{self, Reconciliation};
use crate::limits;
use crate::testfix::{FlacSpec, WavFormat, WavSpec, encode_int_samples, flac_constant, wav};

/// 变异基底：一个**合法**的 16-bit 单声道 WAV（8 帧 @8 kHz，44 + 16 = 60 字节）。
///
/// 用合法字节做基底，是为了让属性测试真的走进 `fmt` / `data` 块解析与解码器。
/// 若基底本身就是垃圾，全部用例都会在探测器那一层被拒，Ok 分支的判据就成了空判据。
fn valid_wav_fixture() -> Vec<u8> {
    let spec = WavSpec {
        channels: 1,
        sample_rate: 8_000,
        bits: 16,
        format: WavFormat::Integer,
    };
    let values: Vec<i32> = (0..8).map(|step| step * 500 - 2_000).collect();
    wav(&spec, &encode_int_samples(16, &values))
}

/// 第二个变异基底：一个**合法**的 FLAC 流（单声道 16-bit，2 个 CONSTANT 帧）。
///
/// `ROAD-M-1-004` 点名的两个格式是 WAV 与 FLAC。只变异 WAV 会漏掉另一条被点名的
/// 解析路径，因此这里也放一个 FLAC 基底。
fn valid_flac_fixture() -> Vec<u8> {
    flac_constant(&FlacSpec::default(), 2, 0)
}

/// 四类不可信输入，合起来让"错误分支"与"Ok 分支"都被执行。
///
/// - 魔数 + 随机尾巴：走探测与解封装的最外层；
/// - 合法 WAV 夹具**整段**变异：头字段会被打坏，走到块解析的内部路径；
/// - 合法 WAV 夹具**只**变异 `data` 区：16-bit PCM 的样本字节不可能非法，因此这一类
///   几乎总是解出资产 —— 它保证 Ok 分支的不变量判据不是空判据；
/// - 合法 FLAC 夹具整段变异：`ROAD-M-1-004` 点名的另一个格式。
fn untrusted_input() -> impl Strategy<Value = Vec<u8>> {
    let magic_and_tail = (
        prop_oneof![
            Just(b"".to_vec()),
            Just(b"RIFF".to_vec()),
            Just(b"RIFF\xff\xff\xff\xffWAVE".to_vec()),
            Just(b"fLaC".to_vec()),
            Just(b"OggS".to_vec()),
            prop::collection::vec(any::<u8>(), 1..=12),
        ],
        prop::collection::vec(any::<u8>(), 0..=256),
    )
        .prop_map(|(mut magic, tail)| {
            magic.extend_from_slice(&tail);
            magic
        });

    let anywhere = valid_wav_fixture();
    let anywhere_len = anywhere.len();
    let mutate_anywhere =
        prop::collection::vec((0..anywhere_len, any::<u8>()), 0..=8).prop_map(move |edits| {
            let mut bytes = anywhere.clone();
            for (index, byte) in edits {
                bytes[index] = byte;
            }
            bytes
        });

    let data_only = valid_wav_fixture();
    let data_only_len = data_only.len();
    let mutate_data_only =
        prop::collection::vec((44..data_only_len, any::<u8>()), 0..=8).prop_map(move |edits| {
            let mut bytes = data_only.clone();
            for (index, byte) in edits {
                bytes[index] = byte;
            }
            bytes
        });

    let flac = valid_flac_fixture();
    let flac_len = flac.len();
    let mutate_flac =
        prop::collection::vec((0..flac_len, any::<u8>()), 0..=8).prop_map(move |edits| {
            let mut bytes = flac.clone();
            for (index, byte) in edits {
                bytes[index] = byte;
            }
            bytes
        });

    prop_oneof![
        magic_and_tail,
        mutate_anywhere,
        mutate_data_only,
        mutate_flac
    ]
}

/// 三个规范采样率 + 任意合理采样率，用来覆盖"规范路径"与"任意路径"。
fn any_rate() -> impl Strategy<Value = u32> {
    prop_oneof![
        Just(44_100u32),
        Just(48_000u32),
        Just(96_000u32),
        1u32..=limits::DEFAULT_MAX_SAMPLE_RATE,
    ]
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

    #[test]
    fn interleaved_sample_count_is_exact_unless_it_overflows(
        frames in any::<u64>(),
        channels in any::<u16>(),
    ) {
        let exact = u128::from(frames) * u128::from(channels);
        match limits::interleaved_samples(frames, channels) {
            Ok(value) => prop_assert_eq!(u128::from(value), exact),
            Err(_) => prop_assert!(exact > u128::from(u64::MAX)),
        }
    }

    #[test]
    fn layout_budget_admits_exactly_the_documented_envelope(
        channels in any::<u16>(),
        sample_rate in any::<u32>(),
        frames in 0u64..=40_000_000u64,
    ) {
        let budget = limits::PcmBudget::default();
        let outcome = limits::check_layout(channels, sample_rate, frames, &budget);
        // 四道闸门的合取；`&&` 的短路求值保证后面的除法/乘法只在采样率非 0 时求值。
        let expected_ok = (1..=budget.max_channels).contains(&channels)
            && (1..=budget.max_sample_rate).contains(&sample_rate)
            && u128::from(frames) <= u128::from(budget.max_duration_secs) * u128::from(sample_rate)
            && u128::from(frames) * u128::from(channels)
                <= u128::from(budget.interleaved_samples_limit());
        prop_assert_eq!(outcome.is_ok(), expected_ok);
        // 判据 (内存上界): 预算通过 ⇒ PCM 字节数 ≤ `max_pcm_bytes`。
        // 这是"解码缓冲不会超过预算"的机械上界（与输入文件长度无关）。
        if outcome.is_ok() {
            prop_assert!(
                u128::from(frames) * u128::from(channels) * 4 <= u128::from(budget.max_pcm_bytes)
            );
        }
    }

    #[test]
    fn pcm_size_conversion_is_exact_or_refused_never_wrapped(
        seconds in any::<u64>(),
        sample_rate in any::<u32>(),
        channels in any::<u16>(),
    ) {
        let exact = u128::from(seconds) * u128::from(sample_rate) * u128::from(channels) * 4;
        match limits::pcm_bytes_for(seconds, sample_rate, channels) {
            Some(bytes) => prop_assert_eq!(u128::from(bytes), exact),
            None => prop_assert!(exact == 0 || exact > u128::from(u64::MAX)),
        }
    }

    #[test]
    fn length_contract_is_self_consistent_and_not_vacuous(
        input_frames in 0u64..=5_000_000u64,
        in_rate in any_rate(),
        out_rate in any_rate(),
    ) {
        let contract = limits::resample_len_contract(input_frames, out_rate, in_rate)
            .expect("non-zero rates always yield a contract");
        prop_assert!(contract.min <= contract.ideal_floor);
        prop_assert!(contract.ideal_ceil <= contract.max);
        prop_assert!(contract.min <= contract.max);
        prop_assert!(
            limits::check_resampled_len(input_frames, out_rate, in_rate, contract.ideal_floor)
                .is_ok()
        );
        prop_assert!(
            limits::check_resampled_len(input_frames, out_rate, in_rate, contract.ideal_ceil)
                .is_ok()
        );
        // 区间之外必须被拒绝 —— 否则这条判据是空的。
        if contract.min > 0 {
            prop_assert!(
                limits::check_resampled_len(input_frames, out_rate, in_rate, contract.min - 1)
                    .is_err()
            );
        }
        if contract.max < u64::MAX {
            prop_assert!(
                limits::check_resampled_len(input_frames, out_rate, in_rate, contract.max + 1)
                    .is_err()
            );
        }
    }

    #[test]
    fn reconciliation_is_symmetric_and_only_exact_on_equality(
        declared in 1u64..=10_000_000u64,
        decoded in 0u64..=10_000_000u64,
        tolerance in 0u64..=1_000u64,
    ) {
        let forward = duration::reconcile(Some(declared), decoded, tolerance);
        let backward = duration::reconcile(Some(decoded), declared, tolerance);
        prop_assert_eq!(forward.is_reconciled(), backward.is_reconciled());
        prop_assert_eq!(
            forward.is_reconciled(),
            declared.abs_diff(decoded) <= tolerance
        );
        prop_assert_eq!(
            matches!(forward, Reconciliation::Exact),
            declared == decoded
        );
        prop_assert_eq!(
            duration::reconcile(None, decoded, tolerance),
            Reconciliation::DeclaredUnknown
        );
    }

    /// 不可信输入边界零 panic —— 生产入口 `decode_bytes` 的总性判据。
    ///
    /// 出处：`lib.rs` 的"不可信输入边界零 panic"。本 crate 用 `MUST-GATE-011`
    /// （"格式解析零崩溃"）作规范锚点 —— 该门禁的正文点名的是 SFZ 词法解析器与
    /// `.yeban` JSON（`YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md:390`），本 crate
    /// 按同一精神自持，映射见 `decode-core-notes.md` §1。
    ///
    /// 量词形状：**任意**字节序列只能得到两种结果 ——
    /// (a) 一个类型化 `DecodeError`，或
    /// (b) 一个自洽的 `DecodedAsset`（不变量见下）。
    /// panic 不是允许的结果。同一条判据把 [ARCH-DET-001] 的"同输入 → 同输出"从一条
    /// 固定夹具推广到任意输入。
    ///
    /// 为什么需要它：`decode.rs` 的 5 条固定垃圾输入与 7 个截断点都是**手写**的形状。
    /// `decode_bytes` 是 `yeban-mcp`（`yeban_import_audio` / `yeban_render_master`）
    /// 唯一使用的解码入口，而它面对的是用户给的任意字节。
    #[test]
    fn untrusted_bytes_never_panic_and_ok_results_are_self_consistent(
        bytes in untrusted_input(),
    ) {
        let options = DecodeOptions::default();
        let first = decode::decode_bytes(&bytes, &options);
        let second = decode::decode_bytes(&bytes, &options);
        // 同输入 → 同输出：成功/失败一致，成功时内容摘要一致。
        prop_assert_eq!(first.is_ok(), second.is_ok());
        if let (Ok(a), Ok(b)) = (&first, &second) {
            prop_assert_eq!(a.pcm_hash(), b.pcm_hash());
        }

        if let Ok(asset) = &first {
            let channels = u64::from(asset.channels());
            let samples = u64::try_from(asset.samples().len()).unwrap();
            prop_assert!(channels > 0);
            prop_assert!(asset.sample_rate() > 0);
            prop_assert_eq!(samples % channels, 0, "交织长度必须是整帧");
            prop_assert_eq!(asset.frame_count(), samples / channels);
            prop_assert!(asset.frame_count() > 0);
            prop_assert!(
                samples <= options.budget.interleaved_samples_limit(),
                "被接受的资产必须落在 PCM 预算之内"
            );
            prop_assert!(
                asset.duration_is_reconciled(),
                "默认开启时长校验, 因此被接受的资产时长必须已对账"
            );
        }
    }
}
