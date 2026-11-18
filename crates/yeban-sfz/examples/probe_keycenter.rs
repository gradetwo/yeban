//! One-off probe (drum-fx-layer line): parse the factory Virtuosity basic kit
//! and print the reduced key range / pitch keycenter of its regions, to pin
//! down why rendered drums play ~2 octaves low.

use std::collections::BTreeMap;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let base = std::env::args()
        .nth(1)
        .expect("usage: probe_keycenter <Programs dir>");
    let limits = yeban_sfz::ParseLimits::default();
    let resolver = yeban_sfz::IncludeResolver::new(&base, limits)?;
    let sources = resolver.resolve("01-basic-kit.sfz")?;
    let instrument = yeban_sfz::parse_sources(&sources, &limits)?;
    println!("regions total: {}", instrument.regions().len());
    println!(
        "cc_defaults(72,76,81,86,90,101..106): {:?}",
        [72u16, 76, 81, 86, 90, 101, 102, 103, 104, 105, 106]
            .iter()
            .map(|cc| (*cc, instrument.cc_defaults().get(cc).copied()))
            .collect::<Vec<_>>()
    );
    for region in instrument.regions() {
        if region.sample.contains("kickmic_kick_snon_vl4_rr1") {
            println!(
                "TARGET {} lokey={} hikey={} keycenter={} spelling={:?} declared_shift={} lovel={} hivel={}",
                region.sample,
                region.lokey,
                region.hikey,
                region.pitch_keycenter,
                region.pitch_keycenter_spelling,
                region.declared_note_shift,
                region.lovel,
                region.hivel
            );
        }
    }
    for key in [36i32, 37, 38, 42, 43, 45, 46, 48, 49, 82] {
        let mut centers: BTreeMap<i32, usize> = BTreeMap::new();
        let mut ranges: BTreeMap<(i32, i32), usize> = BTreeMap::new();
        let mut count = 0usize;
        for region in instrument.regions() {
            if region.lokey <= key && key <= region.hikey && region.trigger_by_note {
                count += 1;
                *centers.entry(region.pitch_keycenter).or_default() += 1;
                *ranges.entry((region.lokey, region.hikey)).or_default() += 1;
            }
        }
        println!("key {key}: matching regions={count} keycenters={centers:?} ranges={ranges:?}");
    }
    Ok(())
}
