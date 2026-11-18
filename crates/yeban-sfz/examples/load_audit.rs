//! 全库装载审计（`line/drum-fx-layer`）：对清单登记的每款 SFZ 乐器做
//! **纯解析**转储，逐 region 扫四类异常，产出 Markdown 清单。
//!
//! - 类 1：键区间为单键 K 但 `pitch_keycenter != K`；
//! - 类 2：`pitch_keycenter` 拼写出处为 `Default`（没写也没推导出来）
//!   且键区间宽度 < 12；
//! - 类 3：记法检测（§14.4 同款配方，证据根音读 WAV `smpl` 头；样本
//!   不可读时如实记「证据不足」）给出非零移位；
//! - 类 4（补充，鼓机怪音色的实际落点）：`<control>` 的浮点 `set_ccN`
//!   声明被解析器丢弃（[`yeban_sfz::Warning::MalformedSetCc`]），且有
//!   region 以双极曲线（curve 1）消费同一 CC 的 `tune` 调制 —— 种子
//!   丢失会让该 CC 在装载时落 0，调音被整段拉偏。
//!
//! 用法：`load_audit <targets.tsv> <report.md>`
//! TSV 每行：`id \t root_dir \t sfz_relpath`（root_dir 是乐器根目录，
//! sfz_relpath 是入口 SFZ 相对 root_dir 的路径）。

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use yeban_sfz::modulation::ModTarget;
use yeban_sfz::{IncludeResolver, KeycenterSpelling, NotationEvidence, ParseLimits, Warning};

/// 读 WAV 文件的 `smpl` 头 `dwMIDIUnityNote`（字节偏移 12，u32 LE）。
/// 非 WAV / 无 smpl / 越界 ⇒ None。
fn wav_unity_note(path: &Path) -> Option<i32> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let mut pos = 12usize;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().ok()?) as usize;
        let data = pos + 8;
        if id == b"smpl" && size >= 16 && data + 16 <= bytes.len() {
            let note = u32::from_le_bytes(bytes[data + 12..data + 16].try_into().ok()?);
            return i32::try_from(note).ok().filter(|n| (0..=127).contains(n));
        }
        pos = data + size + (size & 1);
    }
    None
}

struct ClassHit {
    count: usize,
    examples: Vec<String>,
}

impl ClassHit {
    fn new() -> Self {
        Self {
            count: 0,
            examples: Vec::new(),
        }
    }
    fn push(&mut self, example: String) {
        self.count += 1;
        if self.examples.len() < 3 {
            self.examples.push(example);
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let (tsv_path, report_path) = (&args[1], &args[2]);
    let tsv = std::fs::read_to_string(tsv_path)?;
    let limits = ParseLimits::default();
    let mut report = String::new();
    let mut summary_rows: Vec<String> = Vec::new();
    let mut totals = [0usize; 4];
    let mut affected_instruments = [0usize; 4];

    for line in tsv.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        let (id, root, rel) = (cols[0], cols[1], cols[2]);
        let root_path = PathBuf::from(root);
        let entry_abs = root_path.join(rel);
        let _ = writeln!(report, "## {id}\n");
        let _ = writeln!(report, "- 入口：`{rel}`");
        if !entry_abs.exists() {
            let _ = writeln!(
                report,
                "- ⛔ SFZ 入口不在预期位置，跳过（待按来源拉文本）。\n"
            );
            summary_rows.push(format!("| {id} | - | - | - | - | - | 入口缺失 |"));
            continue;
        }
        let base_dir = entry_abs.parent().expect("entry has parent").to_path_buf();
        let entry_name = entry_abs
            .file_name()
            .expect("entry has file name")
            .to_string_lossy()
            .into_owned();
        let resolver = match IncludeResolver::new(&base_dir, limits) {
            Ok(r) => r,
            Err(e) => {
                let _ = writeln!(report, "- ⛔ include 解析器构造失败：{e}\n");
                continue;
            }
        };
        let sources = match resolver.resolve(&entry_name) {
            Ok(s) => s,
            Err(e) => {
                let _ = writeln!(report, "- ⛔ include 展开失败：{e}\n");
                summary_rows.push(format!("| {id} | - | - | - | - | - | 展开失败 |"));
                continue;
            }
        };
        let instrument = match yeban_sfz::parse_sources(&sources, &limits) {
            Ok(i) => i,
            Err(e) => {
                let _ = writeln!(report, "- ⛔ 解析失败：{e}\n");
                summary_rows.push(format!("| {id} | - | - | - | - | - | 解析失败 |"));
                continue;
            }
        };
        let regions = instrument.regions();
        let _ = writeln!(report, "- region 总数：{}", regions.len());

        // 类 4 的丢弃声明（opcode → 原始值），来自解析告警。
        let mut discarded: Vec<(u16, String)> = Vec::new();
        for warning in instrument.warnings() {
            if let Warning::MalformedSetCc { opcode, value, .. } = warning
                && let Some(cc) = opcode
                    .strip_prefix("set_cc")
                    .and_then(|d| d.parse::<u16>().ok())
            {
                discarded.push((cc, value.clone()));
            }
        }

        let mut class1 = ClassHit::new();
        let mut class1_deltas: BTreeMap<i32, usize> = BTreeMap::new();
        let mut class1_const60 = 0usize;
        let mut class2 = ClassHit::new();
        let mut class2_outside = ClassHit::new();
        let mut class4_regions = ClassHit::new();
        let mut evidence: Vec<NotationEvidence> = Vec::with_capacity(regions.len());
        let mut notename_regions = 0usize;
        for region in regions {
            // lokey < 0 的 region 是 CC 触发（如踏板噪声），不由音符发声，
            // 不进类 1 / 类 2（它们的「键」是哨兵，不是音高）。
            if region.trigger_by_note && region.lokey >= 0 {
                if region.lokey == region.hikey && region.pitch_keycenter != region.lokey {
                    *class1_deltas
                        .entry(region.pitch_keycenter - region.lokey)
                        .or_default() += 1;
                    if region.pitch_keycenter == 60 {
                        class1_const60 += 1;
                    }
                    class1.push(format!(
                        "sample={} key={} keycenter={}",
                        region.sample, region.lokey, region.pitch_keycenter
                    ));
                }
                if region.pitch_keycenter_spelling == KeycenterSpelling::Default
                    && region.hikey - region.lokey < 12
                {
                    let example = format!(
                        "sample={} range={}..{} keycenter={} (Default)",
                        region.sample, region.lokey, region.hikey, region.pitch_keycenter
                    );
                    if region.pitch_keycenter < region.lokey
                        || region.pitch_keycenter > region.hikey
                    {
                        class2_outside.push(example);
                    } else {
                        class2.push(example);
                    }
                }
            }
            if region.pitch_keycenter_spelling == KeycenterSpelling::NoteName {
                notename_regions += 1;
            }
            // 类 4：双极曲线 tune 调制消费了被丢弃种子的 CC。
            for m in region.cc_modulations() {
                if m.target == ModTarget::Tune
                    && m.curve == 1
                    && discarded.iter().any(|(cc, _)| *cc == u16::from(m.cc))
                {
                    class4_regions.push(format!(
                        "sample={} cc={} delta={} curve=1",
                        region.sample, m.cc, m.delta
                    ));
                    break;
                }
            }
            // 类 3 证据：样本嵌入根音（仅 WAV smpl 可读）。
            let sample_rel = region.sample_path().replace('\\', "/");
            let root_note = wav_unity_note(&base_dir.join(&sample_rel));
            evidence.push(NotationEvidence::from_region(region, root_note));
        }
        let detection = yeban_sfz::detect_notation_shift(&evidence);
        let class3_hit = detection.applied_shift.is_some();
        let _ = writeln!(
            report,
            "- 类 1（单键但根音≠键）：{} 条；delta(keycenter−key) 分布 {:?}；其中 keycenter 恒 60 形 {} 条{}",
            class1.count,
            class1_deltas,
            class1_const60,
            examples_suffix(&class1)
        );
        let _ = writeln!(
            report,
            "- 类 2（根音出处 Default 且窄区间）：区间内 {} 条{}；区间外（真风险）{} 条{}",
            class2.count,
            examples_suffix(&class2),
            class2_outside.count,
            examples_suffix(&class2_outside)
        );
        let _ = writeln!(
            report,
            "- 类 3（记法检测）：applied={:?} suspected={:?} miss={:?} 证据 {}/{}（音名拼写 region {} 条）",
            detection.applied_shift,
            detection.suspected_shift,
            detection.miss_reason,
            detection.evidence_count,
            detection.sampled_region_count,
            notename_regions
        );
        let _ = writeln!(
            report,
            "- 类 4（浮点 set_cc 被丢弃）：声明 {:?}；受影响 region {} 条{}",
            discarded,
            class4_regions.count,
            examples_suffix(&class4_regions)
        );
        let _ = writeln!(report);
        let counts = [
            class1.count,
            class2.count + class2_outside.count,
            usize::from(class3_hit),
            class4_regions.count,
        ];
        for (i, c) in counts.iter().enumerate() {
            totals[i] += c;
            if *c > 0 {
                affected_instruments[i] += 1;
            }
        }
        summary_rows.push(format!(
            "| {id} | {} | {} | {} | {} | {} |  |",
            regions.len(),
            counts[0],
            counts[1],
            counts[2],
            counts[3]
        ));
    }

    let mut out = String::new();
    let _ = writeln!(out, "# SFZ 全库装载审计\n");
    let _ = writeln!(
        out,
        "汇总：类 1 共 {} 条（{} 款）；类 2 共 {} 条（{} 款）；类 3 命中 {} 款；类 4 受影响 region 共 {} 条（{} 款）。\n",
        totals[0],
        affected_instruments[0],
        totals[1],
        affected_instruments[1],
        totals[2],
        totals[3],
        affected_instruments[3]
    );
    let _ = writeln!(
        out,
        "| 乐器 | region 数 | 类 1 | 类 2 | 类 3 | 类 4 region | 备注 |\n|---|---:|---:|---:|---:|---:|---|"
    );
    for row in &summary_rows {
        let _ = writeln!(out, "{row}");
    }
    let _ = writeln!(out, "\n---\n");
    out.push_str(&report);
    std::fs::write(report_path, out)?;
    println!("wrote {report_path}");
    Ok(())
}

fn examples_suffix(hit: &ClassHit) -> String {
    if hit.examples.is_empty() {
        String::new()
    } else {
        format!("；例：{}", hit.examples.join("；"))
    }
}
