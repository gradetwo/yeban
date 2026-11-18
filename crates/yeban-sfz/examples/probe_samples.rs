//! One-off probe: dump the unique sample paths an SFZ entry references.
//! Usage: probe_samples <Programs dir> <entry file name>
use std::collections::BTreeSet;
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = PathBuf::from(&args[1]);
    let entry = &args[2];
    let limits = yeban_sfz::ParseLimits::default();
    let resolver = yeban_sfz::IncludeResolver::new(&dir, limits).expect("resolver");
    let sources = resolver.resolve(entry).expect("resolve includes");
    let instrument = yeban_sfz::parse_sources(&sources, &limits).expect("parse");
    let mut set = BTreeSet::new();
    for region in instrument.regions() {
        set.insert(region.sample.replace('\\', "/"));
    }
    for sample in &set {
        println!("{sample}");
    }
    eprintln!("unique samples: {}", set.len());
}
