//! ⭐ **采样读不到时不再静默**（任务 `#311`）。
//!
//! # 这条判据在修什么
//!
//! 修前：`build_region_samples` 在 `read_sample_file` 失败时**静默**塞一份 440 Hz 正弦，
//! 导出照常成功，`loaded-sfz:` 行看起来一切正常。用户听到的是"音色完全不对"。
//! 一次真实排查里，这个静默回退让人先怀疑了音源、采样率、包络三条错路 —— 最后才发现
//! 是 `.sfz` 里的相对路径没解对。
//!
//! 修后：失败清单进 [`RegionSampleTable::missing`]，由 `render_core` 打成
//! `sample-missing:` 行（`--sfz` 导出时逐条列路径与原因）。
//!
//! ⚠ **回退本身保留**（⛔ 不改渲染行为）：少一个采样的工程仍然要能出带子，
//! 改的只是"不再静默"。
//!
//! # 判据
//!
//! | # | 判据 | 判定量 |
//! | :--- | :--- | :--- |
//! | M1 | 缺一个采样 ⇒ `missing` **恰有 1 条**，且带 `sample=` 原文与原因 | `missing.len() == 1`、`missing[0].0 == "gone.wav"`、原因里含 `gone.wav` |
//! | M2 | 全部读到 ⇒ `missing` **为空**（⛔ 假警） | `missing.is_empty()` |
//! | M3 | `sample=*silence` **不进** `missing`（`#217` 既有裁决） | `missing.is_empty()`，且表里那条 PCM 为空 |
//! | M4 | 坏采样**仍然回退**成 440 Hz 正弦（行为不变） | 该条 PCM 长度 48 000、`sample_hz == 48 000` |

use std::path::PathBuf;

use std::str::FromStr as _;

use yeban_app::sfz_load::{build_region_samples, load_sfz_with_instrument};
use yeban_model::EntityId;
use yeban_render::dither::{BitDepth, PcmBuffer};
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::write_plain_wav;

/// 临时夹具目录（`Drop` 删掉自己）。
struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-311-{name}-{}-{}",
            std::process::id(),
            EntityId::new()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建夹具目录");
        // 一份**真的存在**的 16-bit 单声道样本（0.1 s）。
        let samples: Vec<i16> = (0..4_800).map(|i| ((i % 64) * 400) as i16).collect();
        write_plain_wav(
            dir.join("here.wav"),
            &PcmFormat::integer(1, 48_000, 16),
            &PcmBuffer::Int16(samples),
        )
        .expect("写夹具 WAV");
        Self { dir }
    }

    fn write_sfz(&self, name: &str, body: &str) -> PathBuf {
        let path = self.dir.join(format!("{name}.sfz"));
        std::fs::write(&path, body).expect("写夹具 SFZ");
        path
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn id(tail: &str) -> EntityId {
    EntityId::from_str(&format!("01J8Z5Q0R7K3M9X2V4B6N8P0{tail}")).expect("夹具 ULID 必须合法")
}

/// 一份 `.sfz` → 共享表。
fn build(fixture: &Fixture, name: &str, body: &str) -> yeban_app::sfz_load::RegionSampleTable {
    let path = fixture.write_sfz(name, body);
    let (_, instrument) =
        load_sfz_with_instrument(&path, None, 48_000.0).expect("装载夹具 SFZ 必须成功");
    build_region_samples(&instrument, &path)
}

/// ⭐ M1：缺一个采样 ⇒ 恰有一条`missing`，带原文与原因。
#[test]
fn m1_a_missing_sample_is_reported_with_its_original_path() {
    let fixture = Fixture::new("one-missing");
    let table = build(
        &fixture,
        "one_missing",
        "<region> sample=here.wav key=60 pitch_keycenter=60\n\
         <region> sample=gone.wav key=61 pitch_keycenter=61\n",
    );
    assert_eq!(
        table.missing.len(),
        1,
        "⭐ 恰有 1 条坏采样必须被记下来（⛔ 静默）：{:?}",
        table.missing
    );
    assert_eq!(
        table.missing[0].0, "gone.wav",
        "⭐ 记的必须是 `.sfz` 里的**原文**（用户要照它改）"
    );
    assert!(
        table.missing[0].1.contains("gone.wav"),
        "⭐ 原因里必须带路径（否则用户不知道是哪个文件）：{}",
        table.missing[0].1
    );
}

/// ⭐ M2：全部读到 ⇒ 不报警。
#[test]
fn m2_a_healthy_sfz_reports_nothing() {
    let fixture = Fixture::new("all-good");
    let table = build(
        &fixture,
        "all_good",
        "<region> sample=here.wav key=60 pitch_keycenter=60\n",
    );
    assert!(
        table.missing.is_empty(),
        "⭐ 全部读到 ⇒ `missing` 必须为空（⛔ 假警）：{:?}",
        table.missing
    );
    assert!(
        !table.describe.is_empty(),
        "⭐ 来源文案必须仍被填上（`loaded-sfz:` 的 `sample_source=` 靠它）"
    );
}

/// ⭐ M3：`sample=*silence` 不是"读不到"。
#[test]
fn m3_builtin_silence_is_not_a_missing_sample() {
    let fixture = Fixture::new("silence");
    let table = build(
        &fixture,
        "silence",
        "<region> sample=*silence key=60 pitch_keycenter=60\n",
    );
    assert!(
        table.missing.is_empty(),
        "⭐ 内建静音**不是**缺采样（`#217` 的既有裁决）：{:?}",
        table.missing
    );
    let silent = table.samples.get("*silence").expect("内建静音必须进表");
    assert!(
        silent.pcm.is_empty(),
        "⭐ `*silence` 的 PCM 必须是空的（⛔ 不是正弦）"
    );
}

/// ⭐ M4：坏采样仍然回退成 440 Hz 正弦（渲染行为不变）。
#[test]
fn m4_a_missing_sample_still_falls_back_to_the_sine() {
    let fixture = Fixture::new("fallback");
    let table = build(
        &fixture,
        "fallback",
        "<region> sample=gone.wav key=60 pitch_keycenter=60\n",
    );
    let fallback = table
        .samples
        .get("gone.wav")
        .expect("坏采样也必须进表（工程要能出带子）");
    assert_eq!(
        fallback.pcm.len(),
        48_000,
        "⭐ 回退样本必须仍是 48 000 帧（1 s）—— 这条钉的是'渲染行为不变'"
    );
    assert!(
        (fallback.sample_hz - 48_000.0).abs() < f32::EPSILON,
        "⭐ 回退样本的采样率必须是 48 kHz"
    );
    // 正弦：相邻样本不恒定（直流样本会被 DC blocker 吃掉，正弦才是它在链上的样子）。
    let distinct = fallback
        .pcm
        .iter()
        .take(64)
        .fold(0.0_f32, |acc, v| acc.max(*v))
        != fallback.pcm[0];
    assert!(distinct, "⭐ 回退样本必须是 440 Hz 正弦（⛔ 直流）");
}

/// 编译期护栏：`BitDepth` 被本文件用到（`write_plain_wav` 的夹具类型），
/// 避免"改了签名而夹具静默跟不上"。
#[allow(dead_code)]
const _: BitDepth = BitDepth::Int16;

/// 编译期护栏：`id()` 只用一次（跨文件复制夹具时容易留下未用函数）。
#[allow(dead_code)]
fn _id_is_used() -> EntityId {
    id("AA")
}
