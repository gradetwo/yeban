//! 控制台面板命名的**唯一权威**判据（aether UI 第四轮实测反馈）。
//!
//! 同一块底部面板曾经有多套名字：页签叫「设备堆叠 / 设备机架 / 混音控制台 /
//! 钢琴卷帘」，菜单叫「混合平铺视图 (Stacked) / DSP 效果器机架 / 混音台」，
//! 命令面板、键位表、`scene::CONSOLE_TABS`、元素注册表又各有一套。用户在菜单里
//! 点了「混合平铺视图」，页签上却找不到对应项 —— 这是可发现性问题，不是文案偏好。
//!
//! 本文件把「以底部页签文案为唯一权威名」钉死在源码层（只读 `.slint` / `.rs`
//! 文本，⛔ 不编译 Slint）：任一处被改回旧别名、或菜单条目换了名字而页签没跟，
//! 对应判据立即变红。
//!
//! | dock tab id | 权威名 |
//! | :--- | :--- |
//! | `pianoroll` | 钢琴卷帘 |
//! | `mixer` | 混音控制台 |
//! | `devices` | 设备机架 |
//! | `stacked` | 设备堆叠 |

#![allow(unused)]

const DOCK: &str = include_str!("../ui/lower_dock.slint");
const APP: &str = include_str!("../ui/app.slint");
const MODALS: &str = include_str!("../ui/modals.slint");
const CONSOLE_TABS_SLINT: &str = include_str!("../ui/console/console_tabs.slint");
const CLI: &str = include_str!("../src/cli.rs");
const SCENE: &str = include_str!("../src/scene.rs");
const ELEMENTS: &str = include_str!("../src/elements.rs");
const TRANSPORT: &str = include_str!("../ui/transport_bar.slint");

/// 滤掉整行注释后剩下的源码行（注释里解释"为什么统一"必然提到旧别名）。
fn code_lines(source: &str) -> String {
    source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 四个面板的权威名与 dock tab id（= 底部页签文案）。
const CANONICAL: [(&str, &str); 4] = [
    ("pianoroll", "钢琴卷帘"),
    ("mixer", "混音控制台"),
    ("devices", "设备机架"),
    ("stacked", "设备堆叠"),
];

/// 曾经出现过的别名；任一用户可见面再出现即说明命名又分叉了。
const LEGACY_ALIASES: [&str; 16] = [
    "混合平铺视图",
    "混音台面板",
    "钢琴卷帘编辑器",
    "DSP 效果器机架",
    "设备与混音堆叠",
    "内联 DSP 设备机架",
    "MixConsole 混音控制台",
    "MPE 钢琴卷帘编辑器",
    "多轨调音台总控",
    "设备效果链",
    "打开底部 混音台",
    "(调音台/卷帘)",
    "打开底部混音台",
    "混音台最大化",
    "卷帘/混音台",
    "调音台流",
];

/// **判据 ①**：底部页签的四个 `text:` 与 `select-tab(id)` 都用权威名。
#[test]
fn dock_tabs_use_the_canonical_names() {
    let d = code_lines(DOCK);
    for (id, name) in CANONICAL {
        assert!(
            d.contains(&format!("text: \"{name}\"")),
            "底部页签缺权威名「{name}」"
        );
        assert!(
            d.contains(&format!("select-tab(\"{id}\")")),
            "底部页签缺 {id} 的点击落点"
        );
    }
}

/// **判据 ②**：菜单里每一条「切到某页签」的 MenuItem 都用该页签的权威名。
#[test]
fn every_menu_tab_command_uses_the_canonical_name() {
    let app = code_lines(APP);
    for (id, name) in CANONICAL {
        let cmd = format!("command-execute(\"tab-{id}\")");
        let lines: Vec<&str> = app
            .lines()
            .filter(|l| l.contains("MenuItem") && l.contains(&cmd))
            .collect();
        assert!(!lines.is_empty(), "菜单缺 tab-{id} 条目");
        for line in lines {
            assert!(
                line.contains(name),
                "tab-{id} 菜单条目未用权威名「{name}」：{line}"
            );
        }
    }
}

/// **判据 ③**：命令面板四个「切换底栏」条目都用权威名。
#[test]
fn command_palette_uses_the_canonical_names() {
    let m = code_lines(MODALS);
    for (id, name) in CANONICAL {
        let lines: Vec<&str> = m
            .lines()
            .filter(|l| l.contains(&format!("id: \"tab-{id}\"")))
            .collect();
        assert!(!lines.is_empty(), "命令面板缺 tab-{id} 条目");
        for line in lines {
            assert!(
                line.contains(name),
                "命令面板 tab-{id} 未用权威名「{name}」：{line}"
            );
        }
    }
}

/// **判据 ④**：`--print-shortcuts` 的 F7/F8/F9 标签与页签同名。
#[test]
fn shortcut_table_labels_match_the_dock_tabs() {
    let c = code_lines(CLI);
    assert!(c.contains("F7 → 控制台「钢琴卷帘」"));
    assert!(c.contains("F8 → 控制台「混音控制台」"));
    assert!(c.contains("F9 → 控制台「设备机架」"));
    assert!(c.contains("控制台页签：钢琴卷帘 / 混音控制台 / 设备机架"));
    assert!(c.contains("#   X               打开底部混音控制台"));
    assert!(c.contains("#   F3              混音控制台最大化 / 还原"));
    assert!(!c.contains("打开底部混音台"));
    assert!(!c.contains("混音台最大化"));
}

/// **判据 ⑤**：规范页签表与元素注册表也用权威名（`--dump-elements` / 屏读器可见）。
#[test]
fn canonical_names_reach_the_spec_table_and_element_registry() {
    let s = code_lines(SCENE);
    assert!(s.contains("label: \"钢琴卷帘\""));
    assert!(s.contains("label: \"混音控制台\""));
    assert!(s.contains("label: \"设备机架\""));
    let e = code_lines(ELEMENTS);
    assert!(!e.contains("\"多轨调音台总控\""));
    assert!(!e.contains("\"设备效果链\""));
}

/// **判据 ⑥**：用户可见面的代码行里不再出现任何旧别名。
#[test]
fn no_legacy_panel_aliases_remain_in_user_facing_surfaces() {
    let surfaces: [(&str, &str); 6] = [
        ("ui/lower_dock.slint", DOCK),
        ("ui/app.slint", APP),
        ("ui/modals.slint", MODALS),
        ("ui/console/console_tabs.slint", CONSOLE_TABS_SLINT),
        ("src/cli.rs", CLI),
        ("ui/transport_bar.slint", TRANSPORT),
    ];
    for (label, src) in surfaces {
        let code = code_lines(src);
        for alias in LEGACY_ALIASES {
            assert!(
                !code.contains(alias),
                "{label} 又出现旧别名「{alias}」，面板命名再次分叉"
            );
        }
    }
}
