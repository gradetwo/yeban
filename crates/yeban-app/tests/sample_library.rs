//! ⭐ 任务单 `#196` 的判据：**采样库按需下载到程序预设路径**，解析器与"装乐器"谓词都认它。
//!
//! ## 判据读数（每条都有单位 / 逐字原文）
//!
//! | 工单 §2 | 断言 | 证什么 |
//! | :--- | :--- | :--- |
//! | ① 库根是预设路径 | [`preset_library_root`] 三平台逐字；`YEBAN_SAMPLE_LIBRARY` 覆盖后 [`library_root`] 与 [`DownloadOptions::for_app().dest`] **同值** | 证"一处实现 ＋ 可覆盖" |
//! | ② 解析器认库根 | 样本放库根 ⇒ `resolve_sampler_path("assets/samples/<乐器>/…")` 解析到 `<库根>/<乐器>/…`；"试过"列表逐字按 `库根 ⇒ 仓库根 ⇒ cwd` | 证"不用工程目录 / 仓根" |
//! | ③ 谓词 / 清单认库根 | 空库根 ⇒ `instrument_names().len() == 0`；放入 cowsynth 的 SFZ ＋ 全部样本 ⇒ `sfz_samples_present == Ok` 且清单 **0 → 2** | 证"下载完按钮不再是空的" |
//! | ④ 未安装读数 | 控件树 `sample-library-status` == `"采样库未安装，点此下载（9.4 GiB）"`；`sample-library-download-button` 在场 | 证"逐字 ＋ 按钮在场" |
//! | ⑤ 下载流程（假源） | 本地假清单 ＋ `file://` 假源 ⇒ `download 3 / verified 3 / mismatch 0`；落点文件逐字节相同 | 证"走的是同一份取回脚本" |
//! | ⑥ 完整性 | 故意把一项 sha256 写错 ⇒ 退出码 `1`、`mismatch 1`、**不写安装戳**、旧戳被删 | 证"⛔ 不静默算成功" |
//! | ⑦ 不联网 | [`DownloadOutcome::command`] 含 `--source-root file://…`，⛔ 不含 `raw.githubusercontent.com` | 证"跑的是假源" |
//! | ⑩ 无头可判 | 全部判据走 Tier-1 控件树 / 纯函数，无需显示器 | —— |
//!
//! ## 反向腿（注入 ⇒ 哪条红）
//!
//! - **注入甲**：删掉 [`yeban_app::sfz_load::resolve_sampler_path`] 里的库根候选 ⇒
//!   [`the_parser_reads_a_relative_path_from_the_library_root`] 与
//!   [`the_presence_predicate_and_the_install_list_follow_the_library`] 红；
//! - **注入乙**：让 `ViewState::from_project` 把 `sample_library_notice` 恒置空 ⇒
//!   [`the_uninstalled_library_is_words_plus_a_button`] 红（提示消失）。
//!
//! ## ⚠ 它**不**证什么
//!
//! ⛔ 不证真实 9.4 GiB 下载（网络 ＋ 体积；人类命令见报告 §8）；
//! ⛔ 不证"下载完成后 GUI 那条异步回执"真的落屏（Tier-1 没有事件循环；本文件测的是
//! 同一段 `run_download`，宿主接线以行号留证）；
//! ⛔ 不证人耳听到声音（判据只读样本解码读数与控件树）。

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use yeban_app::bridge::{BUILTIN_INSTRUMENTS, ViewState, instrument_names, sfz_samples_present};
use yeban_app::host;
use yeban_app::samples::{
    DOWNLOAD_SCRIPT_RELATIVE, DownloadOptions, LibraryPlatform, SAMPLE_LIBRARY_ENV,
    SAMPLE_LIBRARY_NOT_INSTALLED_HINT, SAMPLE_LIBRARY_STAMP, library_root, preset_library_root,
    run_download,
};
use yeban_app::scene::DemoScene;
use yeban_app::sfz_load::{SfzRoots, repo_root, resolve_sampler_path};
use yeban_app::ui::MainWindow;
use yeban_model::EntityId;
use yeban_model::samples::filled_project;
use yeban_render::dither::PcmBuffer;
use yeban_render::rf64::PcmFormat;
use yeban_render::wav::write_plain_wav;
use yeban_ui_test_port::Size;
use yeban_ui_test_port::inspect::tree_from_element_root;
use yeban_ui_test_port::port::Permission;
use yeban_ui_test_port::render::{LivePort, report_line};
use yeban_ui_test_port::tree::ControlTree;

/// 环境变量是**进程级**的：本文件的判据全部串行持锁（每个测试二进制是独立进程，
/// ⛔ 不影响别的测试文件）。
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// 窗口尺寸：与 `instrument_install.rs` 同款（Tab 3 面板需要高度余量）。
const WINDOW: Size = Size::new(1440, 1000);

/// 假清单里三项的固定字节（sha256 由 `shasum -a 256` 现算后写死；⛔ 不在判据里联网）。
const FAKE_SFZ: &[u8] = b"<region> sample=a.wav key=60\n";
const FAKE_SFZ_SHA: &str = "c476de8e67ea20296eab90591c700270d3d9d50cd3d5dffaaba229256042d571";
const FAKE_WAV: &[u8] = b"RIFFfake";
const FAKE_WAV_SHA: &str = "8263c59cf788f2a1b6646fe893f91b2fe1d637ef24f9859d69a198e749e12fa5";
const FAKE_TXT: &[u8] = b"notes\n";
const FAKE_TXT_SHA: &str = "444e0fffbd825e9610ff5b199485707a0c895339ae80c15cc8a8aee41b106fda";

/// 一次性临时目录（`Drop` 删除自己 —— ⛔ 工作树里不留文件）。
struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(tag: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "yeban-196-{tag}-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&path).expect("建临时目录");
        Self { path }
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// `YEBAN_SAMPLE_LIBRARY` 的 RAII 守卫（`Drop` 时恢复原值）。
///
/// SAFETY（Rust 2024 的 `set_var` 是 `unsafe`）：本文件的全部判据都先拿 [`ENV_LOCK`]，
/// 且本测试二进制里没有其它线程读这个变量；`Drop` 在同一把锁内恢复。
struct LibraryEnv {
    previous: Option<std::ffi::OsString>,
}

impl LibraryEnv {
    /// ⚠ 调用方**必须**先持有 [`ENV_LOCK`]（本函数不再锁一次：那是同一把非重入锁）。
    fn set(dir: &Path) -> Self {
        let previous = std::env::var_os(SAMPLE_LIBRARY_ENV);
        // SAFETY: 调用方持有 ENV_LOCK，且本进程内没有别的线程读 `YEBAN_SAMPLE_LIBRARY`
        // （见结构体文档；这是测试专用的一次性进程）。
        unsafe {
            std::env::set_var(SAMPLE_LIBRARY_ENV, dir);
        }
        Self { previous }
    }
}

impl Drop for LibraryEnv {
    fn drop(&mut self) {
        // SAFETY: 守卫比调用方的 ENV_LOCK 先析构（同一测试内后声明者先 drop），
        // 且恢复的是进入时读到的原值。
        unsafe {
            match &self.previous {
                Some(value) => std::env::set_var(SAMPLE_LIBRARY_ENV, value),
                None => std::env::remove_var(SAMPLE_LIBRARY_ENV),
            }
        }
    }
}

/// 判据 ①：三平台路径公式逐字 ＋ 环境变量覆盖生效（下载落点也用它）。
#[test]
fn the_library_root_is_a_preset_platform_path_and_the_environment_wins() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let home = Path::new("/Users/crow");
    let xdg = Path::new("/home/crow/.local/share");
    let appdata = Path::new(r"C:\Users\crow\AppData\Roaming");
    let macos = preset_library_root(LibraryPlatform::MacOs, Some(home), Some(xdg), Some(appdata));
    let linux_xdg =
        preset_library_root(LibraryPlatform::Linux, Some(home), Some(xdg), Some(appdata));
    let linux_default =
        preset_library_root(LibraryPlatform::Linux, Some(home), None, Some(appdata));
    let windows = preset_library_root(
        LibraryPlatform::Windows,
        Some(home),
        Some(xdg),
        Some(appdata),
    );
    assert_eq!(
        macos,
        PathBuf::from("/Users/crow/Library/Application Support/dev.yeban.app/Samples")
    );
    assert_eq!(
        linux_xdg,
        PathBuf::from("/home/crow/.local/share/yeban/samples")
    );
    assert_eq!(
        linux_default,
        PathBuf::from("/Users/crow/.local/share/yeban/samples")
    );
    assert_eq!(
        windows,
        PathBuf::from(r"C:\Users\crow\AppData\Roaming").join("yeban/samples")
    );

    let temp = TempDir::new("env");
    let _env = LibraryEnv::set(&temp.path);
    assert_eq!(library_root(), temp.path, "覆盖必须生效（且原样，不改写）");
    assert_eq!(
        DownloadOptions::for_app().dest,
        temp.path,
        "下载落点必须**就是** library_root()（⛔ 界面 / 脚本各写一份）"
    );
    report_line(&format!(
        "[#196 ①] macOS={}；Linux(xdg)={}；Linux(默认)={}；Windows={}；\
         覆盖={}；DownloadOptions::for_app().dest={}",
        macos.display(),
        linux_xdg.display(),
        linux_default.display(),
        windows.display(),
        library_root().display(),
        DownloadOptions::for_app().dest.display(),
    ));
}

/// 判据 ②：样本放库根 ⇒ **相对路径**（逐字是仓库内形态）从库根解析成功；
/// 找不到时"试过"列表逐字是 `库根 ⇒ 仓库根 ⇒ cwd`。
#[test]
fn the_parser_reads_a_relative_path_from_the_library_root() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = TempDir::new("parser");
    let library = temp.path.join("library");
    let relative = "assets/samples/yeban196-fixture/tone.sfz";
    let target = library.join("yeban196-fixture/tone.sfz");
    std::fs::create_dir_all(target.parent().expect("有父目录")).expect("建库内目录");
    std::fs::write(&target, "<region> sample=tone.wav key=60\n").expect("写库内 SFZ");
    let _env = LibraryEnv::set(&library);

    let resolved =
        resolve_sampler_path(relative, SfzRoots::none()).expect("库根里的相对路径必须解析得到");
    assert_eq!(resolved, target, "解析结果必须落在库根（⛔ 仓库内）");
    assert!(
        !resolved.starts_with(repo_root()),
        "⛔ 不能解析到仓库内路径：{}",
        resolved.display()
    );

    // 找不到 ⇒ 逐字列出试过的根（工程目录未给 ⇒ 库根 / 仓库根 / cwd 三条）。
    let missing = "assets/samples/yeban196-fixture/missing.sfz";
    let error = resolve_sampler_path(missing, SfzRoots::none()).expect_err("找不到必须报错");
    let cwd = std::env::current_dir().expect("进程 cwd");
    let expected = [
        library.join("yeban196-fixture/missing.sfz"),
        repo_root().join(missing),
        cwd.join(missing),
    ];
    assert_eq!(
        error.tried(),
        expected.as_slice(),
        "候选顺序必须是 采样库根 ⇒ 仓库根 ⇒ cwd"
    );
    report_line(&format!(
        "[#196 ②] path={relative} ⇒ resolved={}；not_in_repo={}；tried={}；原文={}",
        resolved.display(),
        !resolved.starts_with(repo_root()),
        error.tried().len(),
        error
    ));
}

/// 判据 ③：`sfz_samples_present` 认库根 ⇒ "装乐器"清单 **0 → 2**。
#[test]
fn the_presence_predicate_and_the_install_list_follow_the_library() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = TempDir::new("predicate");
    let empty = temp.path.join("empty-library");
    std::fs::create_dir_all(&empty).expect("建空库根");
    let _env = LibraryEnv::set(&empty);
    let before = instrument_names();
    assert_eq!(
        before.len(),
        0,
        "空库根 ⇒ 可装清单必须为空（实测 {before:?}）"
    );
    assert_eq!(
        sfz_samples_present(BUILTIN_INSTRUMENTS[0].path),
        Err(yeban_app::bridge::INSTRUMENT_INSTALL_NO_SAMPLES_REASON),
        "空库根 ⇒ 谓词必须说'没有样本'"
    );

    // 已下载的库：把仓内两份 cowsynth SFZ 拷进库根，并按 `sample=` 逐条造 WAV。
    let library = temp.path.join("library");
    let instrument_dir = library.join("karoryfer-cowsynth");
    std::fs::create_dir_all(&instrument_dir).expect("建库内乐器目录");
    for candidate in BUILTIN_INSTRUMENTS {
        let source = repo_root().join(candidate.path);
        let text = std::fs::read_to_string(&source).expect("读仓内 SFZ");
        let file_name = source.file_name().expect("有文件名");
        std::fs::write(instrument_dir.join(file_name), &text).expect("写库内 SFZ");
        for sample in sample_references(&text) {
            let target = instrument_dir.join(sample);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).expect("建样本子目录");
            }
            write_plain_wav(
                &target,
                &PcmFormat::integer(1, 48_000, 16),
                &PcmBuffer::Int16(vec![0_i16; 16]),
            )
            .expect("写夹具 WAV");
        }
    }
    let _env = LibraryEnv::set(&library);

    for candidate in BUILTIN_INSTRUMENTS {
        assert_eq!(
            sfz_samples_present(candidate.path),
            Ok(()),
            "库根里的 `{}` 必须被同一条谓词判为有样本",
            candidate.path
        );
    }
    let after = instrument_names();
    assert_eq!(
        after,
        vec!["Shifty Pluck".to_owned(), "Lonely Organ".to_owned()],
        "样本进库根后可装清单必须从空变非空（前 {} / 后 {}）",
        before.len(),
        after.len()
    );
    report_line(&format!(
        "[#196 ③] 空库根 names={}；库根={} 装好后 names={:?}（{} 条）；predicate(Shifty)={:?}",
        before.len(),
        library.display(),
        after,
        after.len(),
        sfz_samples_present(BUILTIN_INSTRUMENTS[0].path),
    ));
}

/// 从 SFZ 文本里取全部 `sample=` 的**原文**（不做路径规范化 —— 与解析器同口径）。
fn sample_references(text: &str) -> Vec<String> {
    text.split_whitespace()
        .filter_map(|token| token.strip_prefix("sample="))
        .map(str::to_owned)
        .collect()
}

/// 判据 ④：未安装时控件树里逐字有提示 ＋ 下载按钮在场（无头可判）。
#[test]
fn the_uninstalled_library_is_words_plus_a_button() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = TempDir::new("ui");
    let _env = LibraryEnv::set(&temp.path);
    let project = filled_project();
    let view = ViewState::from_project(&project).expect("投影");
    assert!(!view.sample_library_installed);
    assert_eq!(
        view.sample_library_notice,
        SAMPLE_LIBRARY_NOT_INSTALLED_HINT
    );
    assert!(view.sample_library_download_visible);
    assert_eq!(
        view.sample_library_root,
        temp.path.display().to_string(),
        "投影里的库根必须逐字是 library_root()"
    );

    let scene = DemoScene::from_view(&view);
    let live = LivePort::new(WINDOW, Permission::Interactive, None, || {
        host::build_main_window_with_console_tab(&view, &scene, 2)
    })
    .expect("Tier-1 平台 + 生产窗口构造入口");
    let tree = runtime_tree(live.ui());
    let status = tree
        .find_by_id("sample-library-status")
        .expect("未安装时提示必须在树里");
    assert_eq!(status.label, SAMPLE_LIBRARY_NOT_INSTALLED_HINT);
    let button = tree
        .find_by_id("sample-library-download-button")
        .expect("未安装时下载按钮必须在场");
    assert_eq!(button.label, "下载采样库");
    assert!(
        tree.find_by_id("instrument-install-reason").is_none(),
        "提示与'清单为空'原因互斥可见（⛔ 两句话叠在同一格）"
    );
    report_line(&format!(
        "[#196 ④] notice={:?}；button={:?}；installed={}；root={}",
        status.label, button.label, view.sample_library_installed, view.sample_library_root,
    ));
}

/// 运行时控件树（`[UI-DoD §3.6]` 的唯一载体）。
fn runtime_tree(window: &MainWindow) -> ControlTree {
    let json = tree_from_element_root(window, None)
        .expect("运行时控件树")
        .dump_json();
    ControlTree::from_json(&json).expect("JSON 必须能被同一套校验读回")
}

/// 写一份**本地假清单 ＋ 假源**（三项；`corrupt` ⇒ 把第 N 项的 sha256 写成全 0）。
fn fake_mirror(root: &Path, corrupt: Option<usize>) -> (PathBuf, PathBuf) {
    let source = root.join("source");
    std::fs::create_dir_all(&source).expect("建假源");
    std::fs::write(source.join("a.sfz"), FAKE_SFZ).expect("写假源 SFZ");
    std::fs::write(source.join("b.wav"), FAKE_WAV).expect("写假源 WAV");
    std::fs::write(source.join("c.txt"), FAKE_TXT).expect("写假源 TXT");
    let items = [
        ("a.sfz", FAKE_SFZ.len(), FAKE_SFZ_SHA),
        ("b.wav", FAKE_WAV.len(), FAKE_WAV_SHA),
        ("c.txt", FAKE_TXT.len(), FAKE_TXT_SHA),
    ];
    let mut entries = Vec::new();
    for (index, (name, size, sha)) in items.iter().enumerate() {
        let sha = if corrupt == Some(index) {
            "0".repeat(64)
        } else {
            (*sha).to_owned()
        };
        entries.push(serde_json::json!({
            "id": format!("fake/{name}"),
            "instrument": "fake",
            "relative_path": format!("assets/samples/fake/{name}"),
            "sha256": sha,
            "size_bytes": size,
        }));
    }
    let manifest = root.join("manifest.json");
    let document = serde_json::json!({
        "instruments": [{
            "id": "fake",
            "repo": "example/fake",
            "pin": "0000000000000000000000000000000000000000",
            "relative_root": "assets/samples/fake",
        }],
        "items": entries,
    });
    std::fs::write(
        &manifest,
        serde_json::to_vec_pretty(&document).expect("序列化假清单"),
    )
    .expect("写假清单");
    (manifest, source)
}

/// 本地目录的 `file://` URL（Windows 需要 `file:///C:/…`）。
fn file_url(path: &Path) -> String {
    let text = path.display().to_string().replace('\\', "/");
    if text.starts_with('/') {
        format!("file://{text}")
    } else {
        format!("file:///{text}")
    }
}

/// 判据 ⑤ ＋ ⑦：假清单 ＋ `file://` 假源 ⇒ 下载 3 / 校验通过 3 / 失败 0，⛔ 不联网。
#[test]
fn a_fake_manifest_and_file_source_download_and_verify() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = TempDir::new("download");
    let (manifest, source) = fake_mirror(&temp.path, None);
    let dest = temp.path.join("library");
    let options = DownloadOptions {
        manifest,
        dest: dest.clone(),
        source_root: Some(PathBuf::from(file_url(&source))),
    };
    let outcome = run_download(&options).expect("取回脚本必须能跑");
    assert_eq!(outcome.exit_code, 0);
    assert_eq!(
        (
            outcome.fetched,
            outcome.verified,
            outcome.total,
            outcome.mismatched
        ),
        (3, 3, 3, 0),
        "读数：下载 / 校验通过 / 总项 / 不符；stdout={}",
        outcome.stdout
    );
    assert!(outcome.ok(), "全通过必须为 ok");
    for (name, bytes) in [
        ("a.sfz", FAKE_SFZ),
        ("b.wav", FAKE_WAV),
        ("c.txt", FAKE_TXT),
    ] {
        let landed = std::fs::read(dest.join("fake").join(name)).expect("落点文件必须在");
        assert_eq!(landed, bytes, "`{name}` 必须逐字节相同");
    }
    assert!(
        dest.join(SAMPLE_LIBRARY_STAMP).is_file(),
        "全部通过 ⇒ 必须写安装戳"
    );
    assert!(
        outcome.command.contains("--source-root file://"),
        "命令行必须带本地假源：{}",
        outcome.command
    );
    assert!(
        !outcome.command.contains("raw.githubusercontent.com"),
        "⛔ 判据不许走上游：{}",
        outcome.command
    );
    assert!(
        outcome.stdout.contains("[fetch-samples] 源根: file://"),
        "脚本自己必须打印'源根'证据：{}",
        outcome.stdout
    );
    report_line(&format!(
        "[#196 ⑤⑦] 下载 {}｜校验通过 {}/{}｜失败 {}｜安装戳={}；command={}；summary={}",
        outcome.fetched,
        outcome.verified,
        outcome.total,
        outcome.mismatched,
        dest.join(SAMPLE_LIBRARY_STAMP).is_file(),
        outcome.command,
        outcome.summary(),
    ));
}

/// 判据 ⑥：一个文件的 sha256 不符 ⇒ 退出码 `1`、失败数 `1`、**不写安装戳**（旧戳被删）。
#[test]
fn a_byte_mismatch_is_a_failure_and_never_leaves_an_installed_stamp() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let temp = TempDir::new("mismatch");
    let (manifest, source) = fake_mirror(&temp.path, Some(1));
    let dest = temp.path.join("library");
    // 先放一个**旧**安装戳：失败必须把它删掉（⛔ 让旧的"已安装"继续骗人）。
    std::fs::create_dir_all(&dest).expect("建落点");
    std::fs::write(dest.join(SAMPLE_LIBRARY_STAMP), b"{\"verified\":3}").expect("写旧戳");
    let options = DownloadOptions {
        manifest,
        dest: dest.clone(),
        source_root: Some(PathBuf::from(file_url(&source))),
    };
    let outcome = run_download(&options).expect("取回脚本必须能跑");
    assert_eq!(outcome.exit_code, 1, "字节不符 ⇒ 退出码 1（脚本口径）");
    assert_eq!(
        (
            outcome.fetched,
            outcome.verified,
            outcome.total,
            outcome.mismatched
        ),
        (3, 2, 3, 1),
        "读数：下载 3 / 通过 2 / 总 3 / 不符 1；stdout={}",
        outcome.stdout
    );
    assert!(!outcome.ok(), "⛔ 字节不符不得算成功");
    assert!(
        !outcome.summary().contains("下载完成"),
        "{}",
        outcome.summary()
    );
    assert!(
        !dest.join(SAMPLE_LIBRARY_STAMP).exists(),
        "失败 ⇒ ⛔ 不得留下安装戳（旧戳也必须删）"
    );
    report_line(&format!(
        "[#196 ⑥] exit_code={}；下载 {}｜校验通过 {}/{}｜失败 {}；stamp_exists={}；summary={}",
        outcome.exit_code,
        outcome.fetched,
        outcome.verified,
        outcome.total,
        outcome.mismatched,
        dest.join(SAMPLE_LIBRARY_STAMP).exists(),
        outcome.summary(),
    ));
    // 证据腿：脚本确实存在（不是"没跑所以没红"）。
    assert!(repo_root().join(DOWNLOAD_SCRIPT_RELATIVE).is_file());
}
