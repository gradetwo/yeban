//! **黄金表枚举的"无通配符 `match`"穷举探针**（裁决 R48）。
//!
//! ## 为什么需要它（R48: 黄金表数的是**表**, 读不到**枚举**）
//!
//! `contract_tests::every_diagnostic_message_is_pinned_in_one_golden_table` 用
//! `assert_eq!(cases.len(), 45)` 与"按枚举 18/4/12/3/8"守住**表**。但给一个错误枚举
//! **加一个变体**、同时补上它的 `Display` 实现、却**忘了**给黄金表加一行时 ——
//! 表还是 45 行、四个计数全对 ⇒ 全绿。本文件补上那个缺口:
//!
//! 下面 5 个 `*_label` 是**无通配符**的 `match`（一条 `_ =>` 都不许有）。枚举加一个变体
//! ⇒ 这些函数**编译失败** ⇒ 门禁红。它们每一个都被**真的调用**过（对每个变体各一次）,
//! 因此不是死代码。
//!
//! ## 与黄金表的关系
//!
//! 本文件**不改**黄金表, 也不重复它的文案断言; 它只钉"变体集合 == 45 个标签"这件事。
//! 另一半（"表里的每一行都真的被 `Display` 兑现"）由 `src/lib.rs` 的黄金表判据负责。
//!
//! ## 为什么它自己是一个 cargo 目标
//!
//! 这样"证明它会红"可以做成**可逆的一步**: 把本文件临时移出门禁（接线前）跑一次注入,
//! 再移回来跑同一条注入 —— 见报告。放进 `src/lib.rs` 就没法做这个前后对照。
//!
//! ## 为什么用 `yeban_model::EntityId`
//!
//! `RenderError` 的四个变体带 `EntityId`。集成测试可以按名字使用本 crate 的**普通依赖**
//! （Cargo 对 test 目标同样传 `--extern`）, 因此这里直接用 `yeban_model::EntityId`。

use std::collections::BTreeSet;

use yeban_model::EntityId;
use yeban_render::mastering::MasterExportError;
use yeban_render::pdc::PdcError;
use yeban_render::render::RenderError;
use yeban_render::rf64::Rf64Error;
use yeban_render::wav::WavError;

/// 一个固定的 ULID, 供带 `EntityId` 的变体使用。
fn ulid(index: u32) -> EntityId {
    use std::str::FromStr;
    const CROCKFORD: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let mut text = [b'0'; 26];
    for position in 0..4 {
        text[25 - position] = CROCKFORD[((index >> (5 * position)) & 0x1F) as usize];
    }
    EntityId::from_str(core::str::from_utf8(&text).expect("ASCII")).expect("合法 ULID")
}

fn f_label(error: &Rf64Error) -> &'static str {
    match error {
        Rf64Error::Io(_) => "Rf64::Io",
        Rf64Error::NotWaveContainer => "Rf64::NotWaveContainer",
        Rf64Error::Truncated { .. } => "Rf64::Truncated",
        Rf64Error::BadDs64Len(_) => "Rf64::BadDs64Len",
        Rf64Error::MissingDs64 => "Rf64::MissingDs64",
        Rf64Error::MissingFmt => "Rf64::MissingFmt",
        Rf64Error::MissingData => "Rf64::MissingData",
        Rf64Error::BadFmtLen(_) => "Rf64::BadFmtLen",
        Rf64Error::UnsupportedFormatTag(_) => "Rf64::UnsupportedFormatTag",
        Rf64Error::ZeroChannels => "Rf64::ZeroChannels",
        Rf64Error::ZeroSampleRate => "Rf64::ZeroSampleRate",
        Rf64Error::ZeroBitsPerSample => "Rf64::ZeroBitsPerSample",
        Rf64Error::DataSizeMismatch { .. } => "Rf64::DataSizeMismatch",
        Rf64Error::UnrepresentableBlockAlign { .. } => "Rf64::UnrepresentableBlockAlign",
        Rf64Error::UnsupportedBextVersion(_) => "Rf64::UnsupportedBextVersion",
        Rf64Error::BextLoudnessVersionMismatch { .. } => "Rf64::BextLoudnessVersionMismatch",
        Rf64Error::UnrepresentableBextField { .. } => "Rf64::UnrepresentableBextField",
        Rf64Error::BadStartTimecode(_) => "Rf64::BadStartTimecode",
    }
}

fn w_label(error: &WavError) -> &'static str {
    match error {
        WavError::Hound(_) => "Wav::Hound",
        WavError::FormatMismatch { .. } => "Wav::FormatMismatch",
        WavError::UnsupportedDepth(_) => "Wav::UnsupportedDepth",
        WavError::RejectedFormat { .. } => "Wav::RejectedFormat",
    }
}

fn r_label(error: &RenderError) -> &'static str {
    match error {
        RenderError::InvalidGraph(_) => "Render::InvalidGraph",
        RenderError::MasterNotInGraph(_) => "Render::MasterNotInGraph",
        RenderError::Pdc(_) => "Render::Pdc",
        RenderError::ZeroChannels => "Render::ZeroChannels",
        RenderError::ZeroBlockSize => "Render::ZeroBlockSize",
        RenderError::ZeroFrames => "Render::ZeroFrames",
        RenderError::SizeOverflow { .. } => "Render::SizeOverflow",
        RenderError::MissingSource(_) => "Render::MissingSource",
        RenderError::SourceOnBusNode(_) => "Render::SourceOnBusNode",
        RenderError::UnknownNode(_) => "Render::UnknownNode",
        RenderError::ThreadPool(_) => "Render::ThreadPool",
        RenderError::Source { .. } => "Render::Source",
    }
}

fn p_label(error: &PdcError) -> &'static str {
    match error {
        PdcError::Cycle { .. } => "Pdc::Cycle",
        PdcError::UnknownNode { .. } => "Pdc::UnknownNode",
        PdcError::MasterNotInGraph { .. } => "Pdc::MasterNotInGraph",
    }
}

fn m_label(error: &MasterExportError) -> &'static str {
    match error {
        MasterExportError::UnsupportedSampleRate(_) => "Mst::UnsupportedSampleRate",
        MasterExportError::NotStereo(_) => "Mst::NotStereo",
        MasterExportError::RaggedInterleavedBuffer(_) => "Mst::RaggedInterleavedBuffer",
        MasterExportError::UnsupportedBextVersion(_) => "Mst::UnsupportedBextVersion",
        MasterExportError::UnrepresentableBextField { .. } => "Mst::UnrepresentableBextField",
        MasterExportError::BextCannotCarryLoudness(_) => "Mst::BextCannotCarryLoudness",
        MasterExportError::NonFiniteSamples { .. } => "Mst::NonFiniteSamples",
        MasterExportError::Container(_) => "Mst::Container",
    }
}

/// 判据（R48）: 5 个黄金表枚举的变体集合 == 45 个标签, 且每个变体都能被**穷举 `match`**
/// 取到标签。枚举加一个变体而没改 `*_label` ⇒ **编译失败**（这是本判据的主要机制）。
#[test]
fn every_golden_table_enum_variant_has_an_exhaustive_label() {
    let f_cases: Vec<(&str, _)> = vec![
        ("Rf64::Io", Rf64Error::Io("磁盘满了".to_owned())),
        ("Rf64::NotWaveContainer", Rf64Error::NotWaveContainer),
        (
            "Rf64::Truncated",
            Rf64Error::Truncated {
                what: "data 负载",
                got: 7,
            },
        ),
        ("Rf64::BadDs64Len", Rf64Error::BadDs64Len(12)),
        ("Rf64::MissingDs64", Rf64Error::MissingDs64),
        ("Rf64::MissingFmt", Rf64Error::MissingFmt),
        ("Rf64::MissingData", Rf64Error::MissingData),
        ("Rf64::BadFmtLen", Rf64Error::BadFmtLen(40)),
        (
            "Rf64::UnsupportedFormatTag",
            Rf64Error::UnsupportedFormatTag(0x0002),
        ),
        ("Rf64::ZeroChannels", Rf64Error::ZeroChannels),
        ("Rf64::ZeroSampleRate", Rf64Error::ZeroSampleRate),
        ("Rf64::ZeroBitsPerSample", Rf64Error::ZeroBitsPerSample),
        (
            "Rf64::DataSizeMismatch",
            Rf64Error::DataSizeMismatch {
                declared: 8,
                actual: 4,
            },
        ),
        (
            "Rf64::UnrepresentableBlockAlign",
            Rf64Error::UnrepresentableBlockAlign {
                channels: 21_846,
                bytes_per_sample: 3,
            },
        ),
        (
            "Rf64::UnsupportedBextVersion",
            Rf64Error::UnsupportedBextVersion(3),
        ),
        (
            "Rf64::BextLoudnessVersionMismatch",
            Rf64Error::BextLoudnessVersionMismatch {
                version: 2,
                has_loudness: false,
            },
        ),
        (
            "Rf64::UnrepresentableBextField",
            Rf64Error::UnrepresentableBextField {
                field: "Description",
            },
        ),
        (
            "Rf64::BadStartTimecode",
            Rf64Error::BadStartTimecode("25:00:00".to_owned()),
        ),
    ];
    let w_cases: Vec<(&str, _)> = vec![
        ("Wav::Hound", WavError::Hound("hound 说不行".to_owned())),
        (
            "Wav::FormatMismatch",
            WavError::FormatMismatch {
                expected: "Float32 (32 位)".to_owned(),
                got: "16 位, 整数".to_owned(),
            },
        ),
        ("Wav::UnsupportedDepth", WavError::UnsupportedDepth(8)),
        (
            "Wav::RejectedFormat",
            WavError::RejectedFormat {
                field: "channels",
                detail: "0".to_owned(),
            },
        ),
    ];
    let r_cases: Vec<(&str, _)> = vec![
        (
            "Render::InvalidGraph",
            RenderError::InvalidGraph("环".to_owned()),
        ),
        (
            "Render::MasterNotInGraph",
            RenderError::MasterNotInGraph(ulid(0xFFFF)),
        ),
        ("Render::Pdc", RenderError::Pdc("有环".to_owned())),
        ("Render::ZeroChannels", RenderError::ZeroChannels),
        ("Render::ZeroBlockSize", RenderError::ZeroBlockSize),
        ("Render::ZeroFrames", RenderError::ZeroFrames),
        (
            "Render::SizeOverflow",
            RenderError::SizeOverflow {
                what: "block_size * channels",
            },
        ),
        (
            "Render::MissingSource",
            RenderError::MissingSource(ulid(0xFFFF)),
        ),
        (
            "Render::SourceOnBusNode",
            RenderError::SourceOnBusNode(ulid(0xFFFF)),
        ),
        (
            "Render::UnknownNode",
            RenderError::UnknownNode(ulid(0xFFFF)),
        ),
        (
            "Render::ThreadPool",
            RenderError::ThreadPool("线程池炸了".to_owned()),
        ),
        (
            "Render::Source",
            RenderError::Source {
                node: ulid(0xFFFF),
                message: "音源炸了".to_owned(),
            },
        ),
    ];
    let p_cases: Vec<(&str, _)> = vec![
        (
            "Pdc::Cycle",
            PdcError::Cycle {
                remaining: vec!["a".to_owned(), "b".to_owned()],
            },
        ),
        (
            "Pdc::UnknownNode",
            PdcError::UnknownNode {
                node: "ghost".to_owned(),
            },
        ),
        (
            "Pdc::MasterNotInGraph",
            PdcError::MasterNotInGraph {
                master: "nowhere".to_owned(),
            },
        ),
    ];
    let m_cases: Vec<(&str, _)> = vec![
        (
            "Mst::UnsupportedSampleRate",
            MasterExportError::UnsupportedSampleRate(22_050),
        ),
        ("Mst::NotStereo", MasterExportError::NotStereo(4)),
        (
            "Mst::RaggedInterleavedBuffer",
            MasterExportError::RaggedInterleavedBuffer(7),
        ),
        (
            "Mst::UnsupportedBextVersion",
            MasterExportError::UnsupportedBextVersion(3),
        ),
        (
            "Mst::UnrepresentableBextField",
            MasterExportError::UnrepresentableBextField {
                field: "Originator",
            },
        ),
        (
            "Mst::BextCannotCarryLoudness",
            MasterExportError::BextCannotCarryLoudness(1),
        ),
        (
            "Mst::NonFiniteSamples",
            MasterExportError::NonFiniteSamples {
                index: 3,
                value: f32::NAN,
            },
        ),
        (
            "Mst::Container",
            MasterExportError::Container(Rf64Error::ZeroChannels),
        ),
    ];
    // 逐变体调用穷举 match, 并核对标签。
    for (label, value) in &f_cases {
        assert_eq!(f_label(value), *label, "Rf64Error 的标签");
    }
    for (label, value) in &w_cases {
        assert_eq!(w_label(value), *label, "WavError 的标签");
    }
    for (label, value) in &r_cases {
        assert_eq!(r_label(value), *label, "RenderError 的标签");
    }
    for (label, value) in &p_cases {
        assert_eq!(p_label(value), *label, "PdcError 的标签");
    }
    for (label, value) in &m_cases {
        assert_eq!(m_label(value), *label, "MasterExportError 的标签");
    }

    // 计数与唯一性: 与黄金表的"按枚举 18/4/12/3/8"对齐。
    assert_eq!(f_cases.len(), 18, "Rf64Error 的变体数");
    assert_eq!(w_cases.len(), 4, "WavError 的变体数");
    assert_eq!(r_cases.len(), 12, "RenderError 的变体数");
    assert_eq!(p_cases.len(), 3, "PdcError 的变体数");
    assert_eq!(m_cases.len(), 8, "MasterExportError 的变体数");
    let mut all: Vec<&str> = Vec::with_capacity(45);
    all.extend(f_cases.iter().map(|(label, _)| *label));
    all.extend(w_cases.iter().map(|(label, _)| *label));
    all.extend(r_cases.iter().map(|(label, _)| *label));
    all.extend(p_cases.iter().map(|(label, _)| *label));
    all.extend(m_cases.iter().map(|(label, _)| *label));
    assert_eq!(all.len(), 45, "5 个枚举的变体总数");
    let unique: BTreeSet<&str> = all.iter().copied().collect();
    assert_eq!(unique.len(), 45, "一行一变体: 标签不得重复");
    // 非空证明: 5 个枚举的规模互不相同, 因此"把某一族的计数写错"不会恰好被另一族掩盖。
    assert_ne!(f_cases.len(), r_cases.len());
    assert_ne!(r_cases.len(), m_cases.len());
}
