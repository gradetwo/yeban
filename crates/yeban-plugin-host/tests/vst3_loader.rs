//! 宿主链路集成测试：现场编译夹具插件（`tests/fixtures/fixture_plugin.rs`）
//! 再由宿主加载——扫描、加载失败隔离、离线渲染出声、状态往返全覆盖。
//!
//! 夹具源码独立手写（不 import 宿主代码），见夹具文件头注记。
//! 编译器取 `RUSTC` 环境变量，未设时用 `rustc`（CI 各腿均有）。

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

use yeban_plugin_host::{Host, HostError, MidiEvent, MidiEventKind, PluginKind, PluginState};

/// 平台动态库扩展名（与宿主 `host.rs` 同款口径）。
#[cfg(target_os = "windows")]
const DYLIB_EXT: &str = "dll";
/// 平台动态库扩展名（与宿主 `host.rs` 同款口径）。
#[cfg(target_os = "macos")]
const DYLIB_EXT: &str = "dylib";
/// 平台动态库扩展名（与宿主 `host.rs` 同款口径）。
#[cfg(all(unix, not(target_os = "macos")))]
const DYLIB_EXT: &str = "so";

/// 夹具类标识的十六进制（夹具 `fixture_plugin.rs` 的自造 CID 大端串联）。
const COMBINED_CID_HEX: &str = "11111111222222223333333344444444";
const SEPARATE_CID_HEX: &str = "55555555666666667777777788888888";
const EFFECT_CID_HEX: &str = "ddddddddeeeeeeeeffffffff12121212";

static TEST_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// 本测试专用的临时目录（并行测试互不干扰：pid＋单调计数）。
fn test_dir() -> PathBuf {
    let id = TEST_COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("yeban-plughost-test-{}-{id}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("建测试临时目录");
    dir
}

/// 用 `rustc` 把夹具源码现场编译成 cdylib，返回产物路径。
fn compile_fixture(dir: &Path) -> PathBuf {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fixture_plugin.rs");
    assert!(source.is_file(), "夹具源码不在 {source:?}");
    let out = dir.join(format!("fixture_plugin.{DYLIB_EXT}"));
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
    let output = Command::new(&rustc)
        .args([
            "--crate-type",
            "cdylib",
            "--edition",
            "2021",
            "-o",
            out.to_str().expect("产物路径可转 UTF-8"),
            source.to_str().expect("夹具路径可转 UTF-8"),
        ])
        .output()
        .unwrap_or_else(|error| panic!("启动 {rustc} 失败: {error}"));
    assert!(
        output.status.success(),
        "夹具编译失败（{}）：\n{}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(out.is_file(), "夹具产物不在 {out:?}");
    out
}

fn note_on(offset: u32, pitch: u8, velocity: f32, note_id: i32) -> MidiEvent {
    MidiEvent {
        sample_offset: offset,
        kind: MidiEventKind::NoteOn {
            channel: 0,
            pitch,
            velocity,
            note_id,
        },
    }
}

fn note_off(offset: u32, pitch: u8, note_id: i32) -> MidiEvent {
    MidiEvent {
        sample_offset: offset,
        kind: MidiEventKind::NoteOff {
            channel: 0,
            pitch,
            velocity: 0.0,
            note_id,
        },
    }
}

fn pitch_bend(offset: u32, channel: u8, value: f64) -> MidiEvent {
    MidiEvent {
        sample_offset: offset,
        kind: MidiEventKind::PitchBend { channel, value },
    }
}

#[test]
fn fixture_loads_by_name_and_reports_gain_parameter() {
    let dir = test_dir();
    let plugin = compile_fixture(&dir);
    let mut instance =
        Host::load(plugin.to_str().unwrap(), Some("TestSine")).expect("夹具应能加载");
    assert_eq!(instance.name(), "TestSine");
    let params = instance.parameters().expect("参数枚举");
    assert_eq!(params.len(), 1, "夹具应恰好 1 个参数");
    assert_eq!(params[0].id, 0);
    assert_eq!(params[0].title, "Gain");
    assert!((params[0].default_normalized - 0.8).abs() < 1e-9);
    assert!(params[0].can_automate());
    assert_eq!(instance.parameter_value(0), Some(f64::from(0.8_f32)));
    instance.set_parameter(0, 0.5).expect("直设参数");
    assert_eq!(instance.parameter_value(0), Some(f64::from(0.5_f32)));
}

#[test]
fn fixture_load_without_id_picks_first_instrument_class() {
    let dir = test_dir();
    let plugin = compile_fixture(&dir);
    let instance = Host::load(plugin.to_str().unwrap(), None).expect("不指名应能加载");
    assert_eq!(instance.name(), "TestSine", "首个乐器类应为 TestSine");
}

#[test]
fn fixture_load_by_cid_hex_picks_separate_class() {
    let dir = test_dir();
    let plugin = compile_fixture(&dir);
    let instance = Host::load(plugin.to_str().unwrap(), Some(SEPARATE_CID_HEX))
        .expect("CID 十六进制指名应能加载");
    assert_eq!(instance.name(), "TestSineSeparate");
    // 组合式 CID 也应能指名（覆盖另一条指名分支）。
    let instance = Host::load(plugin.to_str().unwrap(), Some(COMBINED_CID_HEX))
        .expect("组合式 CID 指名应能加载");
    assert_eq!(instance.name(), "TestSine");
}

#[test]
fn fixture_loads_by_bundle_path() {
    let dir = test_dir();
    let plugin = compile_fixture(&dir);
    // 包布局（按平台子目录，与扫描测试同款口径）。
    #[cfg(target_os = "linux")]
    let bundle_sub = "Contents/x86_64-linux";
    #[cfg(target_os = "macos")]
    let bundle_sub = "Contents/MacOS";
    #[cfg(target_os = "windows")]
    let bundle_sub = "Contents/x86_64-win";
    let bundle_dir = dir.join("Test.vst3").join(bundle_sub);
    std::fs::create_dir_all(&bundle_dir).expect("建包目录");
    #[cfg(target_os = "windows")]
    let bundle_file = bundle_dir.join("plugin.vst3");
    #[cfg(not(target_os = "windows"))]
    let bundle_file = bundle_dir.join(format!("fixture_plugin.{DYLIB_EXT}"));
    std::fs::copy(&plugin, &bundle_file).expect("拷包内文件");
    // 传包路径（不是包内动态库路径）应能加载（加载侧与扫描侧同一解析口径）。
    let instance =
        Host::load(dir.join("Test.vst3").to_str().unwrap(), None).expect("包路径应能加载");
    assert_eq!(instance.name(), "TestSine");
}

#[test]
fn fixture_renders_sine_with_sample_accurate_offsets() {
    let dir = test_dir();
    let plugin = compile_fixture(&dir);
    let mut instance =
        Host::load(plugin.to_str().unwrap(), Some("TestSine")).expect("夹具应能加载");
    instance.set_parameter(0, 1.0).expect("增益置 1");
    instance.activate(48_000.0, 1024).expect("激活");
    // 块 1：偏移 0 的 A4（440 Hz）开音，64 帧。
    let mut out_l = vec![0.0_f32; 64];
    let mut out_r = vec![0.0_f32; 64];
    instance
        .process(&[note_on(0, 69, 1.0, 0)], 64, &mut out_l, &mut out_r)
        .expect("渲染");
    assert_eq!(out_l[0], 0.0, "正弦初相为 0");
    let expected_step = (2.0 * std::f64::consts::PI * 440.0 / 48_000.0).sin() as f32;
    assert!(
        (out_l[1] - expected_step).abs() < 1e-4,
        "第 2 采样应 ≈ sin(2π·440/48000)＝{expected_step}，实为 {}",
        out_l[1]
    );
    assert_eq!(out_l, out_r, "夹具双声道写同一内容");
    assert!(
        out_l.iter().any(|sample| sample.abs() > 0.5),
        "64 帧内应有大幅值采样"
    );
    // 块 2：偏移 32 的关音——前 32 帧仍响，后 32 帧静默。
    let mut out_l = vec![0.0_f32; 64];
    let mut out_r = vec![0.0_f32; 64];
    instance
        .process(&[note_off(32, 69, 0)], 64, &mut out_l, &mut out_r)
        .expect("渲染");
    assert!(
        out_l[..32].iter().any(|sample| sample.abs() > 0.1),
        "关音前应仍响"
    );
    assert!(
        out_l[32..].iter().all(|sample| *sample == 0.0),
        "关音后应全静默"
    );
}

#[test]
fn separate_controller_class_renders_with_shared_gain() {
    let dir = test_dir();
    let plugin = compile_fixture(&dir);
    let mut instance =
        Host::load(plugin.to_str().unwrap(), Some("TestSineSeparate")).expect("分离式类应能加载");
    assert_eq!(instance.name(), "TestSineSeparate");
    instance
        .set_parameter(0, 0.25)
        .expect("分离式控制器直设参数");
    assert_eq!(instance.parameter_value(0), Some(0.25));
    instance.activate(48_000.0, 1024).expect("激活");
    let mut out_l = vec![0.0_f32; 64];
    let mut out_r = vec![0.0_f32; 64];
    instance
        .process(&[note_on(0, 69, 1.0, 0)], 64, &mut out_l, &mut out_r)
        .expect("渲染");
    let expected = (2.0 * std::f64::consts::PI * 440.0 / 48_000.0).sin() as f32 * 0.25;
    assert!(
        (out_l[1] - expected).abs() < 1e-4,
        "分离式增益 0.25 应反映在渲染里：期望 {expected}，实为 {}",
        out_l[1]
    );
}

#[test]
fn fixture_state_roundtrips_through_container_bytes() {
    let dir = test_dir();
    let plugin = compile_fixture(&dir);
    let mut first = Host::load(plugin.to_str().unwrap(), Some("TestSine")).expect("夹具应能加载");
    first.set_parameter(0, 0.3).expect("置增益 0.3");
    let state = first.save_state().expect("保存状态");
    // 容器字节往返（encode → decode）后恢复进全新实例。
    let encoded = state.encode();
    let decoded = PluginState::decode(&encoded).expect("容器解码");
    assert_eq!(decoded, state);
    let mut second =
        Host::load(plugin.to_str().unwrap(), Some("TestSine")).expect("夹具应能二次加载");
    second.restore_state(&decoded).expect("恢复状态");
    assert_eq!(
        second.parameter_value(0),
        Some(f64::from(0.3_f32)),
        "恢复后增益应为 0.3"
    );
    // 损坏容器必须响亮失败（⛔ 不静默接受）。
    assert!(matches!(
        PluginState::decode(b"garbage"),
        Err(HostError::StateCorrupt(_))
    ));
    let mut truncated = encoded.clone();
    truncated.truncate(10);
    assert!(matches!(
        PluginState::decode(&truncated),
        Err(HostError::StateCorrupt(_))
    ));
}

#[test]
fn process_before_activate_and_bad_slices_fail_loudly() {
    let dir = test_dir();
    let plugin = compile_fixture(&dir);
    let mut instance =
        Host::load(plugin.to_str().unwrap(), Some("TestSine")).expect("夹具应能加载");
    let mut out_l = vec![0.0_f32; 16];
    let mut out_r = vec![0.0_f32; 16];
    assert!(matches!(
        instance.process(&[], 16, &mut out_l, &mut out_r),
        Err(HostError::NotActivated)
    ));
    instance.activate(48_000.0, 1024).expect("激活");
    let mut short_r = vec![0.0_f32; 8];
    assert!(
        instance.process(&[], 16, &mut out_l, &mut short_r).is_err(),
        "片长 ≠ 帧数必须失败"
    );
}

#[test]
fn fixture_effect_loads_and_processes_audio() {
    let dir = test_dir();
    let plugin = compile_fixture(&dir);
    let mut instance =
        Host::load_effect(plugin.to_str().unwrap(), Some("TestGain")).expect("效果器应能加载");
    assert_eq!(instance.name(), "TestGain");
    let params = instance.parameters().expect("参数枚举");
    assert_eq!(params.len(), 1);
    assert_eq!(params[0].title, "Gain");
    assert!((params[0].default_normalized - 1.0).abs() < 1e-9);
    assert_eq!(instance.parameter_value(0), Some(f64::from(1.0_f32)));
    instance.activate(48_000.0, 1024).expect("激活");
    // 缺省增益 1.0 ⇒ 直通（逐位不变）。
    let in_l: Vec<f32> = (0..16).map(|i| i as f32 / 16.0).collect();
    let in_r: Vec<f32> = (0..16).map(|i| 1.0 - i as f32 / 16.0).collect();
    let mut out_l = vec![0.0_f32; 16];
    let mut out_r = vec![0.0_f32; 16];
    instance
        .process_audio(&[], &in_l, &in_r, &mut out_l, &mut out_r)
        .expect("直通渲染");
    assert_eq!(out_l, in_l, "缺省增益 1.0 应逐位直通");
    assert_eq!(out_r, in_r);
    // 增益 0.5 ⇒ 输出减半（直设只在停用态可用：先停用、直设、再激活）。
    instance.deactivate();
    instance.set_parameter(0, 0.5).expect("直设参数");
    instance.activate(48_000.0, 1024).expect("重激活");
    instance
        .process_audio(&[], &in_l, &in_r, &mut out_l, &mut out_r)
        .expect("减半渲染");
    for index in 0..16 {
        assert!(
            (out_l[index] - in_l[index] * 0.5).abs() < 1e-6,
            "左声道第 {index} 采样应减半"
        );
        assert!(
            (out_r[index] - in_r[index] * 0.5).abs() < 1e-6,
            "右声道第 {index} 采样应减半"
        );
    }
    // 效果器实例走乐器路径必须响亮失败。
    let mut sink_l = vec![0.0_f32; 16];
    let mut sink_r = vec![0.0_f32; 16];
    assert!(
        instance.process(&[], 16, &mut sink_l, &mut sink_r).is_err(),
        "效果器走乐器 process 必须失败"
    );
}

#[test]
fn fixture_load_effect_without_id_picks_first_effect_class() {
    let dir = test_dir();
    let plugin = compile_fixture(&dir);
    let instance = Host::load_effect(plugin.to_str().unwrap(), None).expect("不指名应能加载");
    assert_eq!(instance.name(), "TestGain", "首个效果器类应为 TestGain");
    let instance = Host::load_effect(plugin.to_str().unwrap(), Some(EFFECT_CID_HEX))
        .expect("CID 十六进制指名应能加载");
    assert_eq!(instance.name(), "TestGain");
}

#[test]
fn load_effect_rejects_wrong_kind() {
    let dir = test_dir();
    let plugin = compile_fixture(&dir);
    assert!(matches!(
        Host::load_effect(plugin.to_str().unwrap(), Some("TestSine")),
        Err(HostError::NotAnEffect(_))
    ));
    assert!(matches!(
        Host::load(plugin.to_str().unwrap(), Some("TestGain")),
        Err(HostError::NotAnInstrument(_))
    ));
    assert!(matches!(
        Host::load_effect("stub:sine", None),
        Err(HostError::NotAnEffect(_))
    ));
    assert!(matches!(
        Host::load("stub:gain", None),
        Err(HostError::NotAnInstrument(_))
    ));
}

#[test]
fn stub_gain_processes_audio_and_roundtrips_state() {
    let mut instance = Host::load_effect("stub:gain", None).expect("桩效果器应能加载");
    assert_eq!(instance.name(), "stub:gain");
    assert_eq!(instance.parameter_value(0), Some(1.0));
    instance.activate(48_000.0, 1024).expect("激活");
    let in_l = vec![0.25_f32; 16];
    let in_r = vec![-0.5_f32; 16];
    let mut out_l = vec![0.0_f32; 16];
    let mut out_r = vec![0.0_f32; 16];
    instance
        .process_audio(&[], &in_l, &in_r, &mut out_l, &mut out_r)
        .expect("直通渲染");
    assert_eq!(out_l, in_l);
    assert_eq!(out_r, in_r);
    // 增益 0.25 ⇒ 输出衰减（直设只在停用态可用：先停用、直设、再激活）。
    instance.deactivate();
    instance.set_parameter(0, 0.25).expect("增益置 0.25");
    instance.activate(48_000.0, 1024).expect("重激活");
    instance
        .process_audio(&[], &in_l, &in_r, &mut out_l, &mut out_r)
        .expect("衰减渲染");
    assert!(out_l.iter().all(|sample| (*sample - 0.0625).abs() < 1e-6));
    assert!(out_r.iter().all(|sample| (*sample + 0.125).abs() < 1e-6));
    let state = instance.save_state().expect("保存桩状态");
    let mut second = Host::load_effect("stub:gain", None).expect("桩应能二次加载");
    second.restore_state(&state).expect("恢复桩状态");
    assert_eq!(second.parameter_value(0), Some(0.25));
    // 桩效果器走乐器路径必须响亮失败。
    let mut sink_l = vec![0.0_f32; 16];
    let mut sink_r = vec![0.0_f32; 16];
    assert!(
        second.process(&[], 16, &mut sink_l, &mut sink_r).is_err(),
        "桩效果器走乐器 process 必须失败"
    );
}

#[test]
fn effect_process_audio_before_activate_and_bad_slices_fail_loudly() {
    let mut instance = Host::load_effect("stub:gain", None).expect("桩效果器应能加载");
    let in_l = vec![0.0_f32; 16];
    let in_r = vec![0.0_f32; 16];
    let mut out_l = vec![0.0_f32; 16];
    let mut out_r = vec![0.0_f32; 16];
    assert!(matches!(
        instance.process_audio(&[], &in_l, &in_r, &mut out_l, &mut out_r),
        Err(HostError::NotActivated)
    ));
    instance.activate(48_000.0, 1024).expect("激活");
    let mut short_r = vec![0.0_f32; 8];
    assert!(
        instance
            .process_audio(&[], &in_l, &in_r, &mut out_l, &mut short_r)
            .is_err(),
        "片长不对必须失败"
    );
}

#[test]
fn stub_sine_renders_and_roundtrips_state() {
    let mut instance = Host::load("stub:sine", None).expect("桩应能加载");
    assert_eq!(instance.name(), "stub:sine");
    instance.set_parameter(0, 1.0).expect("增益置 1");
    instance.activate(48_000.0, 1024).expect("激活");
    let mut out_l = vec![0.0_f32; 64];
    let mut out_r = vec![0.0_f32; 64];
    instance
        .process(&[note_on(0, 69, 1.0, 0)], 64, &mut out_l, &mut out_r)
        .expect("渲染");
    assert_eq!(out_l[0], 0.0);
    assert!(out_l.iter().any(|sample| sample.abs() > 0.5));
    let state = instance.save_state().expect("保存桩状态");
    let mut second = Host::load("stub:sine", None).expect("桩应能二次加载");
    second.restore_state(&state).expect("恢复桩状态");
    assert_eq!(second.parameter_value(0), Some(1.0));
}

#[test]
fn fixture_activate_rejects_f32_when_plugin_refuses() {
    let dir = test_dir();
    let plugin = compile_fixture(&dir);
    // 拒 f32 类：`open` 成功（open 不问样本格式），激活须报 `SampleSizeUnsupported`。
    // 结果码负向覆盖：夹具 `canProcessSampleSize` 恒返 1（`kResultFalse`），
    // 宿主须认 1 为拒绝、0 为接受；旧判定 `code != 1` 会把 1 当接受而放行。
    // 正向（0 通过）由其余全部夹具激活测试覆盖。
    assert!(matches!(
        Host::load(plugin.to_str().unwrap(), Some("TestRejectF32")),
        Err(HostError::SampleSizeUnsupported)
    ));
}

#[test]
fn load_failures_are_loud() {
    assert!(matches!(
        Host::load("/definitely/not/a/plugin.so", None),
        Err(HostError::LibraryLoad(_))
    ));
    assert!(matches!(
        Host::load("stub:nope", None),
        Err(HostError::InvalidPluginPath(_))
    ));
    let dir = test_dir();
    let plugin = compile_fixture(&dir);
    assert!(matches!(
        Host::load(plugin.to_str().unwrap(), Some("NoSuchClass")),
        Err(HostError::ClassNotFound(_))
    ));
}

#[test]
fn scan_lists_usable_plugins_and_isolates_bad_files() {
    let dir = test_dir();
    let plugin = compile_fixture(&dir);
    // 散文件。
    let loose = dir.join(format!("loose.{DYLIB_EXT}"));
    std::fs::copy(&plugin, &loose).expect("拷散文件");
    // 包布局（按平台子目录）。
    #[cfg(target_os = "linux")]
    let bundle_sub = "Contents/x86_64-linux";
    #[cfg(target_os = "macos")]
    let bundle_sub = "Contents/MacOS";
    #[cfg(target_os = "windows")]
    let bundle_sub = "Contents/x86_64-win";
    let bundle_dir = dir.join("Test.vst3").join(bundle_sub);
    std::fs::create_dir_all(&bundle_dir).expect("建包目录");
    #[cfg(target_os = "windows")]
    let bundle_file = bundle_dir.join("plugin.vst3");
    #[cfg(not(target_os = "windows"))]
    let bundle_file = bundle_dir.join(format!("fixture_plugin.{DYLIB_EXT}"));
    std::fs::copy(&plugin, &bundle_file).expect("拷包内文件");
    // 坏文件（不是动态库）＋无关文件（应被忽略）。
    std::fs::write(
        dir.join(format!("bad.{DYLIB_EXT}")),
        b"not a library at all",
    )
    .expect("写坏文件");
    std::fs::write(dir.join("notes.txt"), b"hello").expect("写无关文件");

    let found = Host::scan(std::slice::from_ref(&dir));
    // 按路径排序。
    let paths: Vec<&str> = found.iter().map(|entry| entry.path.as_str()).collect();
    let mut sorted = paths.clone();
    sorted.sort();
    assert_eq!(paths, sorted, "扫描结果应按路径排序");
    // 坏文件在列但不可用（失败隔离：不中断扫描）。
    let bad = found
        .iter()
        .find(|entry| entry.path.ends_with(&format!("bad.{DYLIB_EXT}")))
        .expect("坏文件应在扫描结果里");
    assert!(!bad.usable());
    assert!(bad.error.is_some());
    // 散文件可用：6 个类（4 乐器＋1 效果器＋1 控制器）。
    let loose_entry = found
        .iter()
        .find(|entry| entry.path == loose.to_string_lossy())
        .expect("散文件应在扫描结果里");
    assert!(loose_entry.usable());
    assert_eq!(loose_entry.classes.len(), 6);
    assert!(
        loose_entry.classes[..2]
            .iter()
            .all(|class| class.error.is_none()),
        "夹具组件类探针应全通过"
    );
    assert_eq!(loose_entry.classes[0].name, "TestSine");
    assert_eq!(loose_entry.classes[0].kind, PluginKind::Instrument);
    assert_eq!(loose_entry.classes[1].name, "TestSineSeparate");
    assert_eq!(loose_entry.classes[1].kind, PluginKind::Instrument);
    assert_eq!(loose_entry.classes[2].name, "TestGain");
    assert_eq!(loose_entry.classes[2].kind, PluginKind::Effect);
    // 控制器类不作组件探针：单列说明，不记 createInstance 失败，模块仍可用。
    let controller = &loose_entry.classes[3];
    assert_eq!(controller.name, "TestSineController");
    assert_eq!(controller.kind, PluginKind::Unknown);
    assert_eq!(
        controller.error.as_deref(),
        Some("控制器／兼容类：不作组件实例化（VST3 正常现象）")
    );
    // 拒 f32 类与无弯音映射类：组件探针通过（激活／弯音时才显现各自行为）。
    let reject_f32 = &loose_entry.classes[4];
    assert_eq!(reject_f32.name, "TestRejectF32");
    assert_eq!(reject_f32.kind, PluginKind::Instrument);
    assert!(reject_f32.error.is_none());
    let no_midi_map = &loose_entry.classes[5];
    assert_eq!(no_midi_map.name, "TestNoMidiMap");
    assert_eq!(no_midi_map.kind, PluginKind::Instrument);
    assert!(no_midi_map.error.is_none());
    // 包内文件可用（路径指向包内动态库，不是包目录本身）。
    let bundle_entry = found
        .iter()
        .find(|entry| entry.path == bundle_file.to_string_lossy())
        .expect("包内文件应在扫描结果里");
    assert!(bundle_entry.usable());
    // 无关文件不在结果里。
    assert!(
        found.iter().all(|entry| !entry.path.ends_with("notes.txt")),
        "无关文件不应成为候选"
    );
    // 不存在的目录静默跳过（不是失败）。
    let missing = Host::scan(&[dir.join("does-not-exist")]);
    assert!(missing.is_empty());
}

/// 过零率估 F0（仅适用于夹具的纯正弦；GS1 类复音色须用自相关法，
/// 见 `gs1_real_plugin.rs` 的 `estimate_f0`）。
fn estimate_f0_zero_crossing(samples: &[f32], sample_rate: f64) -> f64 {
    if samples.len() < 2 {
        return 0.0;
    }
    let mut crossings = 0_usize;
    for pair in samples.windows(2) {
        if (pair[0] < 0.0) != (pair[1] < 0.0) {
            crossings += 1;
        }
    }
    let seconds = samples.len() as f64 / sample_rate;
    crossings as f64 / 2.0 / seconds
}

#[test]
fn pitch_bend_reaches_instrument_via_parameter_channel() {
    let dir = test_dir();
    let plugin = compile_fixture(&dir);
    let mut instance =
        Host::load(plugin.to_str().unwrap(), Some("TestSine")).expect("夹具应能加载");
    instance.activate(48_000.0, 4096).expect("激活");
    // A4（69）＋满上弯音（1.0＝＋2 半音）同块送达：F0 应 ≈493.88 Hz。
    let events = vec![note_on(0, 69, 0.9, 0), pitch_bend(0, 0, 1.0)];
    let mut out_l = vec![0.0_f32; 4096];
    let mut out_r = vec![0.0_f32; 4096];
    instance
        .process(&events, 4096, &mut out_l, &mut out_r)
        .expect("弯音渲染");
    assert!(
        out_l.iter().any(|sample| sample.abs() > 0.01),
        "弯音渲染须非静音"
    );
    // 跳过前 256 采样（起音瞬态），窗内过零率估 F0。
    let f0 = estimate_f0_zero_crossing(&out_l[256..], 48_000.0);
    eprintln!("弯音 1.0 时 F0 估计: {f0:.1} Hz（期望 ≈493.88 Hz）");
    assert!(
        (470.0..520.0).contains(&f0),
        "满上弯音的 F0 应 ≈493.88 Hz，实为 {f0:.1} Hz"
    );
}

#[test]
fn pitch_bend_center_returns_to_440() {
    let dir = test_dir();
    let plugin = compile_fixture(&dir);
    let mut instance =
        Host::load(plugin.to_str().unwrap(), Some("TestSine")).expect("夹具应能加载");
    instance.activate(48_000.0, 4096).expect("激活");
    // 中心弯音（0.5）＝无弯音：F0 应 ≈440 Hz。
    let events = vec![note_on(0, 69, 0.9, 0), pitch_bend(0, 0, 0.5)];
    let mut out_l = vec![0.0_f32; 4096];
    let mut out_r = vec![0.0_f32; 4096];
    instance
        .process(&events, 4096, &mut out_l, &mut out_r)
        .expect("中心弯音渲染");
    let f0 = estimate_f0_zero_crossing(&out_l[256..], 48_000.0);
    eprintln!("弯音 0.5 时 F0 估计: {f0:.1} Hz（期望 ≈440 Hz）");
    assert!(
        (420.0..460.0).contains(&f0),
        "中心弯音的 F0 应 ≈440 Hz，实为 {f0:.1} Hz"
    );
}

#[test]
fn pitch_bend_applies_at_sample_offset() {
    let dir = test_dir();
    let plugin = compile_fixture(&dir);
    let mut instance =
        Host::load(plugin.to_str().unwrap(), Some("TestSine")).expect("夹具应能加载");
    instance.activate(48_000.0, 4096).expect("激活");
    // 单块 4096 帧、偏移 2048 处满上弯音：前半 440 Hz、后半 ≈493.88 Hz。
    let events = vec![note_on(0, 69, 0.9, 0), pitch_bend(2048, 0, 1.0)];
    let mut out_l = vec![0.0_f32; 4096];
    let mut out_r = vec![0.0_f32; 4096];
    instance
        .process(&events, 4096, &mut out_l, &mut out_r)
        .expect("偏移弯音渲染");
    let f0_first = estimate_f0_zero_crossing(&out_l[256..2048], 48_000.0);
    let f0_second = estimate_f0_zero_crossing(&out_l[2060..], 48_000.0);
    eprintln!("偏移弯音：前半 F0 {f0_first:.1} Hz，后半 F0 {f0_second:.1} Hz");
    assert!(
        (420.0..460.0).contains(&f0_first),
        "弯音前半 F0 应 ≈440 Hz，实为 {f0_first:.1} Hz"
    );
    assert!(
        (470.0..520.0).contains(&f0_second),
        "弯音后半 F0 应 ≈493.88 Hz，实为 {f0_second:.1} Hz"
    );
}

#[test]
fn pitch_bend_skipped_without_midi_mapping() {
    let dir = test_dir();
    let plugin = compile_fixture(&dir);
    let mut instance =
        Host::load(plugin.to_str().unwrap(), Some("TestNoMidiMap")).expect("无映射夹具应能加载");
    instance.activate(48_000.0, 4096).expect("激活");
    // 无 IMidiMapping 时弯音被跳过：process 成功、音符照常出声、F0 仍 ≈440 Hz。
    let events = vec![note_on(0, 69, 0.9, 0), pitch_bend(0, 0, 1.0)];
    let mut out_l = vec![0.0_f32; 4096];
    let mut out_r = vec![0.0_f32; 4096];
    instance
        .process(&events, 4096, &mut out_l, &mut out_r)
        .expect("无映射时 process 须成功");
    assert!(
        out_l.iter().any(|sample| sample.abs() > 0.01),
        "无映射时音符仍须出声"
    );
    let f0 = estimate_f0_zero_crossing(&out_l[256..], 48_000.0);
    eprintln!("无映射＋弯音 1.0 时 F0 估计: {f0:.1} Hz（期望 ≈440 Hz）");
    assert!(
        (420.0..460.0).contains(&f0),
        "无映射时弯音须不生效、F0 仍 ≈440 Hz，实为 {f0:.1} Hz"
    );
}

#[test]
fn set_parameter_rejected_while_active() {
    // 夹具：激活态直设被拒，停用后直设恢复。
    let dir = test_dir();
    let plugin = compile_fixture(&dir);
    let mut instance =
        Host::load(plugin.to_str().unwrap(), Some("TestSine")).expect("夹具应能加载");
    instance.activate(48_000.0, 1024).expect("激活");
    assert!(
        matches!(
            instance.set_parameter(0, 0.5),
            Err(HostError::SetParameterWhileActive)
        ),
        "激活态直设必须报 SetParameterWhileActive"
    );
    instance.deactivate();
    instance.set_parameter(0, 0.5).expect("停用后直设应成功");
    assert_eq!(instance.parameter_value(0), Some(f64::from(0.5_f32)));
    // 桩：同款契约。
    let mut stub = Host::load("stub:sine", None).expect("桩应能加载");
    stub.activate(48_000.0, 1024).expect("桩激活");
    assert!(
        matches!(
            stub.set_parameter(0, 0.5),
            Err(HostError::SetParameterWhileActive)
        ),
        "桩激活态直设必须报 SetParameterWhileActive"
    );
    stub.deactivate();
    stub.set_parameter(0, 0.5).expect("桩停用后直设应成功");
}

#[test]
fn effect_bend_reaches_effect_via_parameter_channel() {
    let dir = test_dir();
    let plugin = compile_fixture(&dir);
    let mut instance =
        Host::load_effect(plugin.to_str().unwrap(), Some("TestGain")).expect("效果器应能加载");
    instance.activate(48_000.0, 4096).expect("激活");
    // 缺省增益 1.0、恒定输入 1.0：通道 0 满上弯音 ⇒ 左声道 ×1.5，
    // 通道 1 无弯音 ⇒ 右声道 ×1.0（夹具弯音→增益因子的可观测手段）。
    let in_l = vec![1.0_f32; 4096];
    let in_r = vec![1.0_f32; 4096];
    let mut out_l = vec![0.0_f32; 4096];
    let mut out_r = vec![0.0_f32; 4096];
    instance
        .process_audio(
            &[pitch_bend(0, 0, 1.0)],
            &in_l,
            &in_r,
            &mut out_l,
            &mut out_r,
        )
        .expect("效果器弯音渲染");
    let mean_l = out_l.iter().sum::<f32>() / out_l.len() as f32;
    let mean_r = out_r.iter().sum::<f32>() / out_r.len() as f32;
    eprintln!("效果器弯音：左均值 {mean_l:.4}（期望 1.5），右均值 {mean_r:.4}（期望 1.0）");
    assert!(
        (mean_l - 1.5).abs() < 1e-4,
        "通道 0 弯音应使左声道增益因子为 1.5，实为 {mean_l}"
    );
    assert!(
        (mean_r - 1.0).abs() < 1e-4,
        "无弯音的右声道应保持 1.0，实为 {mean_r}"
    );
}

#[test]
fn effect_bend_applies_at_sample_offset() {
    let dir = test_dir();
    let plugin = compile_fixture(&dir);
    let mut instance =
        Host::load_effect(plugin.to_str().unwrap(), Some("TestGain")).expect("效果器应能加载");
    instance.activate(48_000.0, 4096).expect("激活");
    // 单块 4096 帧、偏移 2048 处通道 0 满上弯音：左声道前半 ×1.0、后半 ×1.5。
    let in_l = vec![1.0_f32; 4096];
    let in_r = vec![1.0_f32; 4096];
    let mut out_l = vec![0.0_f32; 4096];
    let mut out_r = vec![0.0_f32; 4096];
    instance
        .process_audio(
            &[pitch_bend(2048, 0, 1.0)],
            &in_l,
            &in_r,
            &mut out_l,
            &mut out_r,
        )
        .expect("效果器偏移弯音渲染");
    let mean_first = out_l[..2048].iter().sum::<f32>() / 2048.0;
    let mean_second = out_l[2048..].iter().sum::<f32>() / 2048.0;
    let mean_r = out_r.iter().sum::<f32>() / out_r.len() as f32;
    eprintln!("效果器偏移弯音：左前半 {mean_first:.4}，左后半 {mean_second:.4}，右 {mean_r:.4}");
    assert!(
        (mean_first - 1.0).abs() < 1e-4,
        "弯音前半左声道应为 1.0，实为 {mean_first}"
    );
    assert!(
        (mean_second - 1.5).abs() < 1e-4,
        "弯音后半左声道应为 1.5，实为 {mean_second}"
    );
    assert!(
        (mean_r - 1.0).abs() < 1e-4,
        "右声道应全程 1.0，实为 {mean_r}"
    );
}
