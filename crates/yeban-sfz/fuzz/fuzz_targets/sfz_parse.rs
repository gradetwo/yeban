//! cargo-fuzz 目标 `sfz_parse` —— `MUST-GATE-011`「格式解析零崩溃」。
//!
//! 规范来源 (Normative): `docs/YEBAN_ENGINEERING_IMPLEMENTATION_ROADMAP.md` MUST-GATE-011：
//! 「SFZ 词法解析器与 `.yeban` JSON 反序列化经历千万次随机变异输入注入，零崩溃」。
//!
//! 目标性质：**任意字节序列都不得 panic / abort / OOM / 栈溢出**。解析器返回
//! `Ok(Instrument)` 或 `Err(SfzError)` 都算通过；崩溃即判据变红。
//!
//! 本机纪律：**不在 M2 上跑 fuzz**（重 CPU），只在 CI 手动档执行。运行方法见
//! `docs/ledger/sfz-core-notes.md`「如何在 CI 手动档跑 fuzz」。
//!
//! 无 `#![no_main]` 之外的入口；`libfuzzer-sys` 提供 `main`。

#![no_main]

use libfuzzer_sys::fuzz_target;
use yeban_sfz::{ParseLimits, SfzSource, parse_sources, parse_text};

fuzz_target!(|data: &[u8]| {
    let limits = ParseLimits::default();

    // 1) 纯文本路径：任意字节先做 lossy 解码（真实世界里有 GBK / Latin-1 音色库）。
    let text = String::from_utf8_lossy(data);
    let _ = parse_text(&text, &limits);

    // 2) 源片段路径：与 IncludeResolver 展开后的形态一致（多片段 + 行号偏移）。
    //    这里刻意切成可变数量的片段，覆盖「region 跨片段延续」的状态机分支。
    let owned = text.into_owned();
    let sources: Vec<SfzSource> = owned
        .split_inclusive('\n')
        .enumerate()
        .map(|(index, line)| SfzSource {
            path: "fuzz.sfz".to_string(),
            text: line.to_string(),
            first_line: index + 1,
        })
        .collect();
    let _ = parse_sources(&sources, &limits);

    // 3) 上限收紧后的路径：确认「显式上限」不会因为边界值而 panic。
    let tight = ParseLimits {
        max_line_bytes: 16,
        max_regions: 4,
        max_opcodes_per_header: 4,
        max_defines: 2,
        max_macro_expansions_per_line: 2,
        ..ParseLimits::default()
    };
    let _ = parse_text(&text, &tight);
});
