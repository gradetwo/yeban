//! T-FIX-951：首启 demo 页的下载入口与进度/取消/失败回执。
//!
//! 判据分两层：
//! ① **纯逻辑**（无需 Slint 平台）：`gather` 在本地一首 demo 都没有时仍产出
//!    官方 8 首下载卡片；进度文案在总量未知时明说"总量未知"，不编假数；
//! ② **接线红对照**（源码级）：host / dialog / app 三处必须同时具备下载回调、
//!    后台线程与排空路径；删掉任一处后检查函数必须当场点名该缺口（改前红、
//!    改后绿），避免"只有一个文件写了接线"的假绿。

use std::path::PathBuf;

use yeban_app::settings::Settings;
use yeban_app::startup_chooser::{DemoDownloadState, gather};
use yeban_app::templates::TemplateRoots;

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-t951-{}-{}",
            std::process::id(),
            yeban_model::ids::EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }

    fn roots(&self) -> TemplateRoots {
        TemplateRoots {
            factory: self.dir.join("factory"),
            user: self.dir.join("user"),
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn gather_lists_all_official_demos_with_download_entry() {
    let scratch = Scratch::new();
    // 两个模板根目录都空 ⇒ 本地演示清单为空（不设置 YEBAN_DEMOS_DIR）。
    let data = gather(&Settings::default(), &scratch.roots());
    let official: Vec<_> = data
        .demos
        .iter()
        .filter(|entry| entry.download.is_some())
        .collect();
    assert_eq!(
        official.len(),
        yeban_app::demo_download::DEMOS.len(),
        "本地无 demo 时也必须列出全部官方 demo（下载入口），实际 {:?}",
        data.demos
    );
    let root = yeban_app::demo_download::cache_root();
    for entry in official {
        let yeban_app::startup_chooser::DemoSource::File(path) = &entry.source else {
            panic!(
                "官方 demo 必须指向下载落地的工程文件，实际 {:?}",
                entry.source
            );
        };
        assert!(
            path.starts_with(&root),
            "下载入口的落点必须在缓存根下（{root:?}），实际 {path:?}"
        );
        assert!(
            matches!(
                &entry.download,
                Some(DemoDownloadState::NotDownloaded) | Some(DemoDownloadState::Downloaded)
            ),
            "清单里的官方 demo 只能是未下载或已下载，实际 {:?}",
            entry.download
        );
    }
}

#[test]
fn progress_text_never_fabricates_unknown_total() {
    let unknown = DemoDownloadState::Downloading {
        received: 4096,
        total: None,
    };
    let text = unknown.progress_text();
    assert!(text.contains("总量未知"), "总量未知必须明说，实际 {text:?}");
    assert!(
        !text.contains("0 B /"),
        "不得把未知总量写成 0，实际 {text:?}"
    );

    let known = DemoDownloadState::Downloading {
        received: 4096,
        total: Some(8192),
    };
    let text = known.progress_text();
    assert!(
        text.contains("4.0 KB") && text.contains("8.0 KB"),
        "已收/总量必须可读，实际 {text:?}"
    );

    let failed = DemoDownloadState::Failed("请求失败: 超时".to_owned());
    assert_eq!(failed.progress_text(), "请求失败: 超时");
    assert_eq!(failed.code(), 3, "失败态必须可被界面识别为可重试");
}

/// 下载接线的必备读数（每个字符串都是一处独立接线）。
const REQUIRED_WIRING: [(&str, &str); 9] = [
    ("host.rs", "fn apply_demo_download_event("),
    ("host.rs", "ui.on_startup_demo_download_drain_requested("),
    ("host.rs", "ui.on_startup_demo_download(move |index|"),
    ("host.rs", "ui.on_startup_demo_cancel_download(move |index|"),
    ("host.rs", "std::thread::spawn(move ||"),
    (
        "ui/dialogs/startup_chooser.slint",
        "callback demo-download(int);",
    ),
    (
        "ui/dialogs/startup_chooser.slint",
        "callback demo-cancel-download(int);",
    ),
    (
        "ui/dialogs/startup_chooser.slint",
        "accessible-id: \"startup-demo-download-bar\"",
    ),
    ("ui/tokens.slint", "download-state: int,"),
];

fn missing_wiring(sources: &[(&str, String)]) -> Vec<String> {
    REQUIRED_WIRING
        .iter()
        .filter(|(file, needle)| {
            sources
                .iter()
                .find(|(name, _)| name == file)
                .is_none_or(|(_, text)| !text.contains(needle))
        })
        .map(|(file, needle)| format!("{file}: 缺少 {needle}"))
        .collect()
}

fn sources() -> Vec<(&'static str, String)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    vec![
        (
            "host.rs",
            std::fs::read_to_string(root.join("src/host.rs")).expect("读 host.rs"),
        ),
        (
            "ui/dialogs/startup_chooser.slint",
            std::fs::read_to_string(root.join("ui/dialogs/startup_chooser.slint"))
                .expect("读 chooser.slint"),
        ),
        (
            "ui/tokens.slint",
            std::fs::read_to_string(root.join("ui/tokens.slint")).expect("读 tokens.slint"),
        ),
        (
            "ui/app.slint",
            std::fs::read_to_string(root.join("ui/app.slint")).expect("读 app.slint"),
        ),
    ]
}

#[test]
fn download_wiring_is_complete_and_has_red_contrast() {
    let src = sources();
    let missing = missing_wiring(&src);
    assert!(missing.is_empty(), "下载接线不全: {missing:?}");
    assert!(
        src[3].1.contains("callback startup-demo-download(int);"),
        "MainWindow 必须把下载回调转给对话框"
    );

    // 红对照：删掉 tokens.slint 的下载态字段后，同一个检查必须当场点名缺口。
    let mut mutated = src.clone();
    mutated[2].1 = mutated[2]
        .1
        .replace("download-state: int,", "REMOVED-BY-MUTATION");
    let red = missing_wiring(&mutated);
    assert!(
        red.iter().any(|item| item.contains("ui/tokens.slint")),
        "变异后判据必须红在点名的缺口上，实际 {red:?}"
    );
}
