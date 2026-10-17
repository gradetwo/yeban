//! 任务单 `#142` 的判据：**内置演示工程不得引用没有字节的音频素材**，且出厂就能出声。
//!
//! # 这一片修的是什么
//!
//! `--project-sample default` 的 `clip_pool` 里有一条 `ClipContent::Audio`，引用
//! `AssetHash::of_bytes(b"yeban-demo-kick")` —— 那是 13 字节 ASCII 标签的摘要，
//! 全仓没有任何文件与它对应；`project.assets` 里也没有它，归档资产池为空。
//! `EngineHost` 的第 1.3 步（`#136`）于是逐条打印
//! `音频片段注入跳过: asset 1c531c50…：会话资产池里没有这份字节` ⇒ 在任何机器上都静音。
//!
//! 修法：样例在**运行期合成**那段音频（`yeban_model::samples::{demo,filled}_project_assets`），
//! 并把字节送进 `ProjectArchive.assets`（`crate::cli::Sample::assets` 是唯一入口）。
//! ⛔ 不入库二进制、⛔ 不加依赖、⛔ 不把跳过改成静默。
//!
//! # 判据与文件头的对应
//!
//! | 判据 | 内容 |
//! | :--- | :--- |
//! | [`every_builtin_sample_references_audio_with_metadata_and_bytes`] | 判据 3：三个内置样例逐条"引用 ⇒ 有元数据 ＋ 有字节"；缺失数必须为 0 |
//! | [`the_default_sample_injects_one_asset_and_skips_none`] | 判据 1：产品路径 `EngineHost` 注入 1 个素材／2 条摆放／跳过 **0**；反向腿（不接字节来源）⇒ 跳过 **1**，原因逐字可读 |
//! | [`the_default_sample_export_path_is_audible_where_only_the_audio_clip_plays`] | 判据 2：`--export-wav` 的渲染路径（`render_project_f32_with_assets`）非静音 ＋ 峰值；反向腿：只在该音频摆放发声的窗口里，无资产池 ⇒ **逐位静音** |
//!
//! 全部无头：不建窗口、不需要桌面会话。

use std::collections::BTreeMap;

use yeban_app::cli::Sample;
use yeban_app::engine_host::EngineHost;
use yeban_app::export_wav::WavExportOptions;
use yeban_app::render_core::render_project_f32_with_assets;
use yeban_model::{AssetHash, ClipContent};

/// 工程默认采样率（三个内置样例都是 48 kHz）。
const RATE: usize = 48_000;

/// 判据 3：**出厂不空引用** —— 每个内置样例里，每条 `ClipContent::Audio` 的
/// `AssetHash` 都必须在 `project.assets` 里有元数据、且在样例资产池里有字节。
///
/// 数的是（单位）：样例数（3 个）／音频片段数（条）／缺失元数据数（条）／
/// 缺失字节数（条）。通过条件是后两个都是 **0**。
#[test]
fn every_builtin_sample_references_audio_with_metadata_and_bytes() {
    let mut sample_count = 0_usize;
    let mut clip_count = 0_usize;
    let mut missing_metadata = 0_usize;
    let mut missing_bytes = 0_usize;
    let mut summary = Vec::new();
    for sample in [Sample::Default, Sample::Filled, Sample::Empty] {
        sample_count += 1;
        let project = sample.project();
        let pool = sample.assets();
        assert_eq!(
            project.validate(),
            Ok(()),
            "{}: 内置样例必须合法",
            sample.name()
        );
        let mut audio_clips = 0_usize;
        for entry in project.clip_pool.values() {
            let ClipContent::Audio { asset, .. } = &entry.content else {
                continue;
            };
            audio_clips += 1;
            clip_count += 1;
            match project.assets.get(asset) {
                Some(metadata) => assert_eq!(
                    &metadata.hash,
                    asset,
                    "{}: `project.assets` 的内嵌哈希必须等于键",
                    sample.name()
                ),
                None => missing_metadata += 1,
            }
            match pool.iter().find(|(hash, _)| hash == asset) {
                Some((hash, bytes)) => {
                    assert_eq!(
                        &AssetHash::of_bytes(bytes),
                        hash,
                        "{}: 池里字节的 SHA-256 必须等于 clip 引用的哈希",
                        sample.name()
                    );
                    if let Some(metadata) = project.assets.get(asset) {
                        assert_eq!(
                            u64::try_from(bytes.len()).unwrap_or(u64::MAX),
                            metadata.byte_len,
                            "{}: `byte_len` 必须等于真实字节数（⛔ 手抄值）",
                            sample.name()
                        );
                    }
                }
                None => missing_bytes += 1,
            }
        }
        summary.push(format!(
            "{}: audio-clips={audio_clips} asset-metadata={} asset-blobs={}",
            sample.name(),
            project.assets.len(),
            pool.len()
        ));
    }
    println!("[#142/3] {}", summary.join(" | "));
    println!(
        "[#142/3] samples={sample_count} audio-clips={clip_count} \
         missing-metadata={missing_metadata} missing-bytes={missing_bytes}"
    );
    assert_eq!(missing_metadata, 0, "出厂样例不得引用没有元数据的素材");
    assert_eq!(missing_bytes, 0, "出厂样例不得引用没有字节的素材");
    // `default` 1 条（909 鼓组）＋ `filled` 1 条（Kick）＋ `empty` 0 条。
    assert_eq!(sample_count, 3);
    assert_eq!(clip_count, 2);
}

/// 判据 1：产品路径（`EngineHost::reload` 的第 1.3 步）注入 1 个素材／跳过 0；
/// 反向腿不接字节来源 ⇒ 跳过 1（逐字原因可读，⛔ 不静默）。
#[test]
fn the_default_sample_injects_one_asset_and_skips_none() {
    let project = Sample::Default.project();
    let pool: BTreeMap<AssetHash, Vec<u8>> = Sample::Default.assets().into_iter().collect();
    let pool_assets = pool.len();

    let mut host = EngineHost::new();
    host.set_audio_asset_source(Box::new(move |hash| pool.get(hash).cloned()));
    host.reload(&project, 0).expect("内置样例的投影必须成功");
    let report = host.audio_clip_report().clone();
    println!("[#142/1] pool={pool_assets} {}", report.summary());
    assert_eq!(pool_assets, 1, "default 样例必须自带恰好 1 份音频字节");
    assert_eq!(report.injected_assets, 1, "必须注入 1 个素材");
    assert_eq!(report.skipped.len(), 0, "跳过数必须是 0");
    // 演示工程把同一份 909 鼓组摆在两条音频轨上 ⇒ 2 条摆放接上同一份 PCM。
    assert_eq!(report.spans_attached, 2, "2 条摆放必须都接上 PCM");
    let injected = report.assets.first().expect("必须有逐素材读数");
    assert_eq!(injected.frames, 24_000, "0.5 s @ 48 kHz");
    assert_eq!(injected.channels, 2);
    assert_eq!(injected.sample_rate, RATE as u32);
    assert!(!injected.resampled, "48k 工程 + 48k 素材 ⇒ ⛔ 不重采样");
    assert_eq!(
        host.current_snapshot().expect("有快照").audio_pcm_spans(),
        2
    );

    // 反向腿：不接字节来源 ⇒ 与修复前完全相同的读数（跳过 1，原因逐字）。
    let mut silent = EngineHost::new();
    silent.reload(&project, 0).expect("没有来源也必须能建快照");
    let silent_report = silent.audio_clip_report().clone();
    println!("[#142/1r] {}", silent_report.summary());
    assert_eq!(silent_report.injected_assets, 0);
    assert_eq!(silent_report.skipped.len(), 1);
    assert!(
        silent_report.skipped[0]
            .reason
            .contains("没有接上资产字节来源"),
        "跳过原因必须可读: {}",
        silent_report.skipped[0].reason
    );
}

/// 判据 2：`--export-wav` 的渲染路径非静音（非零样本 ＋ 峰值）；音频可闻性用控制臂
/// （只清掉夹具自带的 solo/mute）在**只有该音频摆放发声的窗口**（3.05 s–3.45 s，
/// MIDI 已在 2 s 结束）里证明，反向腿（无资产池）在该窗口逐位静音。
#[test]
fn the_default_sample_export_path_is_audible_where_only_the_audio_clip_plays() {
    let project = Sample::Default.project();
    let assets = Sample::Default.assets();
    let options = WavExportOptions {
        frames: 4 * RATE as u64,
        sample_rate: RATE as u32,
        channels: 2,
        ..WavExportOptions::default()
    };
    let with_assets = render_project_f32_with_assets(&project, options, None, &assets)
        .expect("带资产池的渲染必须成功");

    let (nonzero, peak) = stats(&with_assets.samples);
    println!(
        "[#142/2] 非零样本={nonzero} 峰值={peak:.6} frames={} \
         audio-clips-rendered={} audio-clips-skipped={:?}",
        with_assets.frames, with_assets.audio_clips_rendered, with_assets.audio_clips_skipped
    );
    assert!(nonzero > 0, "演示工程必须出声（非零样本 = {nonzero}）");
    assert!(peak > 0.05, "峰值必须可闻（实测 {peak}）");
    // ⚠ 演示夹具自带 `solo`（0 号轨「鼓」）与 `mute`（5 号轨「打击」）—— 引擎与离线
    // 导出都按 `track_is_audible` 门控（`yeban-engine` 的唯一口径）。音频摆放所在的
    // 1/2 轨因此被 solo 静音；这不是"字节缺失"，跳过原因里会逐字写明。
    for reason in &with_assets.audio_clips_skipped {
        assert!(
            reason.contains("mute/solo"),
            "带资产池时唯一的跳过原因必须是夹具自带的 solo/mute 门控，实测: {reason}"
        );
    }

    // ---- 控制臂：清掉夹具自带的 solo/mute ＋ 只留音频片段（⛔ 不改片段 / 哈希 / 摆放）----
    //
    // 目的：证明**字节真的能进母带**。演示工程 0 号轨是 `solo`（`live_ui_mcp.rs`
    // 明确要求它保持 true）、5 号轨是 `mute` ⇒ 音频摆放所在的 1/2 轨会被引擎的
    // `track_is_audible` 门控。控制臂只做两件事：清门控、移走 MIDI 片段/摆放；
    // 片段本身、资产哈希、两条音频摆放一个字节不改。
    let mut unblocked = project.clone();
    for track in unblocked.tracks.values_mut() {
        track.solo = false;
        track.mute = false;
    }
    let audio_clip_ids: std::collections::BTreeSet<_> = unblocked
        .clip_pool
        .iter()
        .filter(|(_, entry)| matches!(entry.content, ClipContent::Audio { .. }))
        .map(|(id, _)| *id)
        .collect();
    for track in unblocked.tracks.values_mut() {
        track
            .clips
            .retain(|_, placement| audio_clip_ids.contains(&placement.clip_id));
    }
    unblocked
        .clip_pool
        .retain(|id, _| audio_clip_ids.contains(id));
    let audible =
        render_project_f32_with_assets(&unblocked, options, None, &assets).expect("控制臂渲染");
    let missing = render_project_f32_with_assets(&unblocked, options, None, &[])
        .expect("反向腿渲染（逐条跳过）");
    assert_eq!(
        audible.audio_clips_rendered, 2,
        "solo 门控清掉后两条音频摆放都要进母带"
    );
    assert!(audible.audio_clips_skipped.is_empty());
    assert_eq!(missing.audio_clips_rendered, 0);
    assert_eq!(missing.audio_clips_skipped.len(), 2);

    // 窗口：3.05 s–3.45 s。MIDI 片段/摆放已移走，第二个音频摆放从 tick 5760（= 3 s）
    // 开始 ⇒ 这个窗口里**只有**音频片段可能发声。
    let start = (3.05 * RATE as f64) as usize;
    let end = (3.45 * RATE as f64) as usize;
    let (with_nonzero, with_peak) = window_stats(&audible.samples, 2, start, end);
    let (without_nonzero, without_peak) = window_stats(&missing.samples, 2, start, end);
    println!(
        "[#142/2] 3.05–3.45 s 窗口（solo/mute 已清、只留音频）: 有资产池 非零={with_nonzero} \
         峰值={with_peak:.6}；无资产池 非零={without_nonzero} 峰值={without_peak:.6}"
    );
    assert!(with_nonzero > 0, "该窗口只有音频片段，必须发声");
    assert!(with_peak > 0.05);
    assert_eq!(without_nonzero, 0, "反向腿：没有字节 ⇒ 该窗口逐位静音");
    assert_eq!(without_peak, 0.0);
}

/// 交错缓冲的 `(非零样本个数, abs() 峰值)`。
fn stats(samples: &[f32]) -> (usize, f32) {
    let nonzero = samples.iter().filter(|sample| **sample != 0.0).count();
    let peak = samples
        .iter()
        .fold(0.0_f32, |acc, sample| acc.max(sample.abs()));
    (nonzero, peak)
}

/// `[start_frame, end_frame)` 窗口内的 `(非零样本个数, abs() 峰值)`（交错缓冲）。
fn window_stats(samples: &[f32], channels: usize, start: usize, end: usize) -> (usize, f32) {
    let start = start * channels;
    let end = (end * channels).min(samples.len());
    stats(&samples[start.min(end)..end])
}
