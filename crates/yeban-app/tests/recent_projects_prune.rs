//! ⭐ T-FIX-312：**最近工程清单不再养死行**。
//!
//! 修前：`settings::RECENT_PROJECTS_MAX = 10` 只按上限从尾部截断；
//! `startup_chooser::gather` 逐条 `exists: path.is_file()` 但不筛选也不回写；
//! 删掉工程 / 移动工程后旧路径仍占一格，10 格满了就再也进不了新工程，
//! 点击死条目只报 `recent-missing`，用户没有任何移除 / 清空入口。
//!
//! 修后（三期同走一条纯裁决）：
//! 1. [`settings::prune_recent_projects`] 按 `path.is_file()` 剔除失效条目；
//!    `settings::load()` 在启动时做一次 prune 并**写回 settings.json**；
//! 2. [`startup_chooser::gather`] 再 filter 一次 ⇒ 即使调用方直接传未经
//!    `load()` 的 Settings，死行也不进选择器；
//! 3. [`settings::forget_recent_project`] / [`settings::clear_recent_projects`]
//!    提供单条移除 / 清空；点击失效条目的宿主臂顺手移除并写回。
//!
//! ## 平台约束
//!
//! `SETTINGS_PATH_ENV` 是**进程级**环境变量 ⇒ 本文件只有一个 `#[test]`，
//! 避免并行测试互相改路径（与活窗口单测同款纪律）。
//! ⚠ 本文件只判设置层 + 汇集层的纯逻辑与持久化；真启动器窗口 / 菜单项
//! 的外观不在范围内。

use std::path::PathBuf;

use yeban_app::settings::{RECENT_PROJECTS_MAX, RecentProject, SETTINGS_PATH_ENV, Settings};

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-recent-prune-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn entry(path: &std::path::Path, at: u64) -> RecentProject {
    RecentProject {
        path: path.to_path_buf(),
        opened_at_unix_ms: at,
    }
}

#[test]
fn dead_recent_entries_are_pruned_persisted_and_never_come_back() {
    let scratch = Scratch::new();
    let settings_file = scratch.dir.join("settings.json");
    // SAFETY: 本文件只有一个 #[test]，进程内没有其它线程读 `SETTINGS_PATH_ENV`；
    // 测试末尾恢复原值，⛔ 不把这次覆盖泄漏给同进程其它判据。
    unsafe {
        std::env::set_var(SETTINGS_PATH_ENV, &settings_file);
    }

    let alive = scratch.dir.join("alive.yeban");
    std::fs::write(&alive, b"original").expect("夹具：活条目有真文件");
    let ghost = scratch.dir.join("deleted.yeban"); // 从不创建 ⇒ 已删除
    let moved = scratch.dir.join("moved.yeban"); // 模拟移动：旧路径失效
    let subdir = scratch.dir.join("a-directory"); // 目录不是工程文件
    std::fs::create_dir(&subdir).expect("夹具：目录");

    // ---- ① 纯 prune：只留 is_file 的条目，顺序不变 ----
    let raw = vec![entry(&alive, 3), entry(&ghost, 2), entry(&subdir, 1)];
    assert_eq!(
        yeban_app::settings::prune_recent_projects(&raw),
        vec![entry(&alive, 3)],
        "文件 / 目录 / 已删除三种输入必须只有文件留下，顺序不变"
    );
    assert!(
        yeban_app::settings::prune_recent_projects(&[]).is_empty(),
        "空清单 prune 仍是空"
    );

    // ---- ② load() 在启动时 prune 并写回（load_from 仍读原始文件）----
    let mut settings = Settings {
        recent_projects: vec![entry(&alive, 3), entry(&ghost, 2), entry(&moved, 1)],
        ..Settings::default()
    };
    yeban_app::settings::save(&settings).expect("夹具：写设置");
    assert_eq!(
        yeban_app::settings::load_from(&settings_file)
            .recent_projects
            .len(),
        3,
        "load_from 是纯读，原始文件仍保有那 3 条"
    );
    let loaded = yeban_app::settings::load();
    assert_eq!(loaded.recent_projects, vec![entry(&alive, 3)]);
    assert_eq!(
        yeban_app::settings::load_from(&settings_file).recent_projects,
        vec![entry(&alive, 3)],
        "清理后的清单必须真的写回 settings.json ⇒ 下次启动不再复活死路径"
    );

    // ---- ③ 清单满 10 条时，剔除死行后新开的一条仍能进入 ----
    let mut ten: Vec<RecentProject> = (0..RECENT_PROJECTS_MAX)
        .map(|i| entry(&alive, 100 - i as u64))
        .collect();
    ten[RECENT_PROJECTS_MAX - 1] = entry(&ghost, 1);
    let pruned = yeban_app::settings::prune_recent_projects(&ten);
    assert_eq!(pruned.len(), RECENT_PROJECTS_MAX - 1);
    let fresh = scratch.dir.join("brand-new.yeban");
    std::fs::write(&fresh, b"new").expect("夹具：新工程文件在盘上");
    let with_new = yeban_app::settings::touch_recent_project(&pruned, &fresh, 999);
    assert_eq!(with_new.len(), RECENT_PROJECTS_MAX);
    assert_eq!(with_new[0], entry(&fresh, 999), "新打开的一条排最前");
    assert!(
        !with_new.iter().any(|e| e.path == ghost),
        "死路径不能借 truncate 重新占位"
    );

    // ---- ④ 单条移除 / 清空都写回空清单，不带死路径 ----
    settings.recent_projects = vec![entry(&alive, 2), entry(&fresh, 1)];
    yeban_app::settings::save(&settings).expect("夹具：两条活记录");
    yeban_app::settings::forget_recent_project(&alive);
    assert_eq!(
        yeban_app::settings::load_from(&settings_file).recent_projects,
        vec![entry(&fresh, 1)],
        "单条移除按路径逐字节匹配，别的字段与条目不受影响"
    );
    yeban_app::settings::clear_recent_projects();
    assert!(
        yeban_app::settings::load_from(&settings_file)
            .recent_projects
            .is_empty(),
        "清空后 settings.json 必须是空数组"
    );

    // 收尾：环境变量只在本单进程内生效，⛔ 不污染同进程后续判据。
    // SAFETY: 同上 —— 单判据进程、无并发读者，恢复默认路径。
    unsafe {
        std::env::remove_var(SETTINGS_PATH_ENV);
    }
}
