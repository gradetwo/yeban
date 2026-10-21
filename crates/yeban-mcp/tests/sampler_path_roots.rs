//! ⭐ 任务单 `#271` 的判据：MCP 的采样器路径解析补齐 app 的**四档**。
//!
//! ## 缺陷（`#270` 勘察，逐字）
//!
//! MCP 的 `domain::render::resolve_sampler_path` 修前只试两档「工程目录 ⇒ 进程 cwd」，
//! 而 app 的 `sfz_load::resolve_sampler_path` 试四档
//! 「工程目录 ⇒ 采样库根 ⇒ 仓库根 ⇒ cwd」⇒ **装在采样库里的内置 SFZ** 在 MCP 导出上
//! 报 `samplerUnreadable`，CLI 却能加载。
//!
//! ## 判据读数
//!
//! | # | 断言 | 证什么 |
//! | :--- | :--- | :--- |
//! | 1 | 库根里的内置形态路径（`assets/samples/<乐器>/<文件>.sfz`）修前 ⇒ `reason=samplerUnreadable`，`tried` 只有工程目录 / cwd | 复现缺陷（本文件先于修复落地并实测红） |
//! | 2 | 同一路径修后 ⇒ 加载成功、`resolvedPath` 落在**库根**、`label` 以 `sfz(` 开头、`missing=[]` | 修好了 |
//! | 3 | 四档逐档命中：工程目录 / 库根 / 仓库根 / cwd 各有一个只在该档存在的夹具 ⇒ `resolvedPath` 逐字等于该档 | 顺序与覆盖 |
//! | 4 | 正对照：`near.sfz` 在工程目录与库根**同名**（库根那份是坏文件）⇒ 修前修后都取工程目录那份，`sha256` 相同 | 不改变本来就能解析的路径 |
//! | 5 | 四档都找不到 ⇒ 仍 `samplerUnreadable`，`tried` 四条按序 | 不变成静默 |
//! | 6 | 同一 `YEBAN_SAMPLE_LIBRARY` 下 MCP 的库根候选逐字等于该环境变量值（app 的 `--probe-instruments` 打印的 `lib:` 同值） | 与 app 同口径 |
//!
//! ## 路径比较（⭐ `#281`：Windows 腿的判决）
//!
//! 判据比较的是**路径**（[`PathBuf`]，按组件相等），⛔ 不是 `display()` 字符串：
//! Windows 的 `PathBuf::join("karoryfer-cowsynth/<文件>")` 会**保留**相对段里的 `/`，
//! 而期望值由 `join("karoryfer-cowsynth").join("<文件>")` 拼成 `\` —— **同一份文件**的
//! 两种拼写。`#281` 的 CI 判决逐字是 `5 passed; 1 failed`，两条字符串的差异只有
//! 第 118 个字节的 `/` vs `\`（左 = `resolvedPath`，右 = 判据的 `target`）。
//! `std` 的 `Path::eq` 按**组件**比较（`library/std/src/path.rs`：
//! `self.components() == other.components()`），Windows 的分隔符是 `\` **和** `/`
//! （`library/std/src/sys/path/windows.rs:12` 的 `path_separator_bytes!(b'\\', b'/')`）
//! ⇒ 同一个判据在三个平台上都成立。这条语义本身在 Windows 腿上由
//! [`path_equality_treats_both_windows_separators_as_the_same_file`] 钉住。
//!
//! ## 反向腿（注入 ⇒ 哪条红）
//!
//! - **注入甲**：删掉库根候选 ⇒ 判据 1 仍是红（修复方向），判据 2/3-库根/6 红；
//! - **注入乙**：把库根候选挪到工程目录之前 ⇒ 判据 4 红（库根那份坏文件胜出）；
//! - **注入丙**：把仓库根候选删掉 ⇒ 判据 3-仓库根 红；
//! - **注入丁**：找不到时不报错、回退正弦 ⇒ 判据 5 红。
//!
//! ## 临时目录 / 环境变量纪律
//!
//! 本文件是**独立测试进程**：`YEBAN_SAMPLE_LIBRARY` 与进程 cwd 都是进程级状态，
//! 全部判据串行持 [`ENV_LOCK`]，且用 RAII 守卫恢复原值；夹具落在
//! `std::env::temp_dir()/<唯一子目录>`，[`Scratch`] 的 `Drop` 删除 —— ⛔ 不污染仓库。

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{Value, json};

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
use yeban_model::samples::filled_project;
use yeban_model::{
    ClipPlacement, EntityId, RoutingEdge, RoutingKind, SamplerBinding, YebanProjectV1,
};

/// 进程级环境状态的串行锁（`YEBAN_SAMPLE_LIBRARY` ＋ cwd）。
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// 与 app 的 `yeban_app::samples::SAMPLE_LIBRARY_ENV` 逐字相同。
const SAMPLE_LIBRARY_ENV: &str = "YEBAN_SAMPLE_LIBRARY";

/// 内置清单里的第一条（`bridge::BUILTIN_INSTRUMENTS[0].path`，逐字）：
/// 它只相对**仓库根**存在（`assets/samples/...` 的 SFZ 文本随仓库分发）。
const BUILTIN_RELATIVE: &str = "assets/samples/karoryfer-cowsynth/cowsynth_shifty_pluck.sfz";

/// 一个临时目录，`Drop` 时删除自己。
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "yeban-mcp-271-{}-{tag}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// 设置 `YEBAN_SAMPLE_LIBRARY` 的 RAII 守卫（`Drop` 恢复原值）。
struct LibraryEnv {
    previous: Option<OsString>,
}

impl LibraryEnv {
    /// ⚠ 调用方必须持有 [`ENV_LOCK`]。
    fn set(dir: &Path) -> Self {
        let previous = std::env::var_os(SAMPLE_LIBRARY_ENV);
        // SAFETY: 调用方持有 ENV_LOCK；本测试进程内没有其它线程读这个变量。
        unsafe {
            std::env::set_var(SAMPLE_LIBRARY_ENV, dir);
        }
        Self { previous }
    }
}

impl Drop for LibraryEnv {
    fn drop(&mut self) {
        // SAFETY: 与 `set` 同一条理由；恢复的是进入时读到的原值。
        unsafe {
            match &self.previous {
                Some(value) => std::env::set_var(SAMPLE_LIBRARY_ENV, value),
                None => std::env::remove_var(SAMPLE_LIBRARY_ENV),
            }
        }
    }
}

/// 切换进程 cwd 的 RAII 守卫（`Drop` 恢复）。⚠ 调用方必须持有 [`ENV_LOCK`]。
struct Cwd {
    previous: PathBuf,
}

impl Cwd {
    fn set(dir: &Path) -> Self {
        let previous = std::env::current_dir().expect("进程 cwd");
        std::env::set_current_dir(dir).expect("切换 cwd");
        Self { previous }
    }
}

impl Drop for Cwd {
    fn drop(&mut self) {
        std::env::set_current_dir(&self.previous).expect("恢复 cwd");
    }
}

/// **手写**一份常量 `0.25` 的 24-bit 单声道 48 kHz WAV（⛔ 不用被测写入器自证）。
fn write_constant_wav(path: &Path) {
    const VALUE: f64 = 0.25;
    const FRAMES: usize = 480;
    const RATE: u32 = 48_000;
    let scaled = (VALUE * ((1_i32 << 23) - 1) as f64).round() as i32;
    let mut data = Vec::with_capacity(FRAMES * 3);
    for _ in 0..FRAMES {
        data.extend_from_slice(&scaled.to_le_bytes()[..3]);
    }
    let mut out = Vec::with_capacity(44 + data.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&u32::try_from(36 + data.len()).expect("小").to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16_u32.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1_u16.to_le_bytes()); // 单声道
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * 3).to_le_bytes());
    out.extend_from_slice(&3_u16.to_le_bytes());
    out.extend_from_slice(&24_u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&u32::try_from(data.len()).expect("小").to_le_bytes());
    out.extend_from_slice(&data);
    std::fs::write(path, &out).expect("写常量样本夹具");
}

/// 在 `<dir>/<name>.sfz` 写一份可发声的 SFZ ＋ 同目录 `tone.wav`，返回 SFZ 路径。
fn write_sfz_with_sample(dir: &Path, name: &str) -> PathBuf {
    std::fs::create_dir_all(dir).expect("建 SFZ 目录");
    write_constant_wav(&dir.join("tone.wav"));
    let path = dir.join(format!("{name}.sfz"));
    std::fs::write(&path, "<region> sample=tone.wav\n").expect("写 SFZ 夹具");
    path
}

/// `filled_project()` 的 Lead 轨绑定一条采样器路径（其余字段原样）。
fn project_with_sampler(raw: &str) -> YebanProjectV1 {
    let mut project = filled_project();
    let lead = project
        .tracks
        .iter()
        .find(|(_, track)| track.name == "Lead")
        .map(|(id, _)| *id)
        .expect("filled_project 必须有 Lead 轨");
    project.tracks.get_mut(&lead).expect("Lead").sampler = Some(SamplerBinding {
        path: raw.to_owned(),
    });
    project.validate().expect("夹具工程必须合法");
    project
}

/// 渲染一次，返回**工具结果**（成功或 `error` 都在里面）。
fn render(project_path: &Path, raw_sampler: &str, out: &Path) -> Value {
    render_project(project_with_sampler(raw_sampler), project_path, out)
}

/// 渲染**给定的**工程（⭐ `#276` 的两轨夹具要自己装配路由）。
fn render_project(project: YebanProjectV1, project_path: &Path, out: &Path) -> Value {
    let token = BearerToken::generate().token;
    let auth = format!("Bearer {}", token.expose());
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    dispatcher
        .domain_mut()
        .open_in_memory(project_path.to_path_buf(), project, false)
        .expect("注入夹具工程");
    let line = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "yeban_render_master",
            "arguments": {
                "format": "wav",
                "sampleRate": 48000,
                "path": out.display().to_string(),
            },
        },
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(&auth), &line);
    outcome
        .response
        .expect("必须有响应")
        .result
        .expect("工具结果")
}

/// 成功渲染的 `data.sources[0].sampler` 读数；失败时把整条响应打出来再 panic。
fn sampler_report(result: &Value) -> &Value {
    assert_eq!(
        result["status"], "success",
        "渲染必须成功（失败时看 error）: {result}"
    );
    &result["data"]["sources"][0]["sampler"]
}

/// ⭐ `#281`：把 JSON 里的 `resolvedPath` 读成 [`PathBuf`] 再比较（⛔ 不比 `display()` 字符串）。
///
/// 理由见文件头「路径比较」：Windows 上同一份文件的两种分隔符拼写在显示字符串上不等，
/// 但 `Path` 的 `PartialEq` 按**组件**比较 ⇒ 三平台读的是同一个判据。
fn resolved_path(sampler: &Value) -> PathBuf {
    PathBuf::from(
        sampler["resolvedPath"]
            .as_str()
            .expect("成功渲染必须报 resolvedPath 字符串"),
    )
}

/// ⭐ `#281`：钉住 [`resolved_path`] 依赖的**平台语义**本身 —— Windows 上 `/` 与 `\`
/// 是同一个分隔符，所以同一份文件的两种拼写按 `Path` 比较相等。
///
/// 只在 Windows 上编译：macOS / Linux 上 `\` 是**普通字符**，把它们当分隔符反而是错的。
/// 这条让"⭐ 为什么 Windows 上会绿"从论证变成下一轮 CI 上的一条读数。
#[cfg(windows)]
#[test]
fn path_equality_treats_both_windows_separators_as_the_same_file() {
    let slash = Path::new("C:\\library\\karoryfer-cowsynth/cowsynth_shifty_pluck.sfz");
    let backslash = Path::new("C:\\library\\karoryfer-cowsynth\\cowsynth_shifty_pluck.sfz");
    assert_eq!(
        slash, backslash,
        "Windows 上 `/` 与 `\\` 必须是同一个分隔符（`#281` 的判据依赖这条）"
    );
}

/// 判据 2 / 3-库根 / 6：`assets/samples/<乐器>/…` 只存在于**采样库根** ⇒ 必须命中库根。
#[test]
fn the_library_root_leg_finds_the_builtin_path() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let scratch = Scratch::new("library");
    let library = scratch.join("library");
    // 库内布局：`assets/samples/` 前缀被剥掉 ⇒ `<库根>/<乐器>/…`。
    let target =
        write_sfz_with_sample(&library.join("karoryfer-cowsynth"), "cowsynth_shifty_pluck");
    let _env = LibraryEnv::set(&library);

    let project_path = scratch.join("project").join("song.yeban");
    std::fs::create_dir_all(project_path.parent().expect("有父目录")).expect("建工程目录");
    let out = scratch.join("library-leg.wav");
    let result = render(&project_path, BUILTIN_RELATIVE, &out);

    let sampler = sampler_report(&result);
    assert_eq!(
        resolved_path(sampler),
        target,
        "必须解析到库根那份（⛔ 仓库根那份 / ⛔ 工程旁）: {result}"
    );
    assert!(
        sampler["label"]
            .as_str()
            .expect("label")
            .starts_with("sfz("),
        "label 必须含 `sfz(...)` 读数: {result}"
    );
    assert_eq!(
        sampler["sampleFailures"],
        json!([]),
        "库内样本必须真的读到（missing=[]）: {result}"
    );
    assert!(
        sampler["label"]
            .as_str()
            .expect("label")
            .contains("missing=[]"),
        "label 里的 missing 必须为空: {result}"
    );
    assert_eq!(
        sampler["notesUsingFileSample"], 4,
        "四个音符都必须用文件样本发声: {result}"
    );
    assert!(
        sampler.get("notesUsingSineFallback").is_none(),
        "⭐ `#275` 起采样器轨报告**没有**这个键（引擎无正弦回退；\
         回退只在 `yeban_render_sfz_wav` 旁路）: {result}"
    );
    println!(
        "### [#271 判据2/3-库根] resolvedPath={} label={}",
        sampler["resolvedPath"], sampler["label"]
    );
    // 判据 6：同一环境变量下 MCP 的库根候选 = 环境变量值本身（app 打印的 `lib:` 同值）。
    let missing = "assets/samples/karoryfer-cowsynth/not-there.sfz";
    let failed = render(&project_path, missing, &scratch.join("missing-lib.wav"));
    let tried = failed["error"]["data"]["tried"]
        .as_array()
        .expect("tried 必须是数组");
    let expected_library_candidate = library.join("karoryfer-cowsynth/not-there.sfz");
    let actual_library_candidate = PathBuf::from(tried[1].as_str().expect("tried[1] 必须是字符串"));
    assert_eq!(
        actual_library_candidate, expected_library_candidate,
        "tried[1] 必须是 `<YEBAN_SAMPLE_LIBRARY>/<乐器>/<文件>`（按路径比较）：{failed}"
    );
    println!(
        "### [#271 判据6] YEBAN_SAMPLE_LIBRARY={} ⇒ 库根候选={}",
        library.display(),
        tried[1]
    );
}

/// 判据 4：工程目录第一 ⇒ 同名坏文件放在库根也**不改**解析结果、不改 `sha256`。
#[test]
fn the_project_directory_still_wins_and_the_sha256_does_not_move() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let scratch = Scratch::new("project-first");
    let project_dir = scratch.join("project");
    std::fs::create_dir_all(&project_dir).expect("建工程目录");
    let project_sfz = write_sfz_with_sample(&project_dir, "near");
    // 库根同名但**坏**：若库根先试，区域解析会读到一份没有样本的 SFZ ⇒ 判据读数不同。
    let library = scratch.join("library");
    std::fs::create_dir_all(&library).expect("建库根");
    std::fs::write(library.join("near.sfz"), "<region> sample=absent.wav\n").expect("写坏 SFZ");
    let _env = LibraryEnv::set(&library);

    let project_path = project_dir.join("song.yeban");
    let out = scratch.join("project-first.wav");
    let result = render(&project_path, "near.sfz", &out);
    let sampler = sampler_report(&result);
    assert_eq!(
        resolved_path(sampler),
        project_sfz,
        "相对路径必须先按工程目录解析: {result}"
    );
    assert_eq!(
        sampler["sampleFailures"],
        json!([]),
        "取工程旁那份（有样本）⇒ 不能有缺样本: {result}"
    );
    println!(
        "### [#271 判据4] resolvedPath={} sha256={}",
        sampler["resolvedPath"], result["data"]["sha256"]
    );
}

/// 判据 3-仓库根：内置清单路径只相对**仓库根**存在（库根设为空 ⇒ 库根档必然落空）。
#[test]
fn the_repository_root_leg_finds_the_builtin_sfz() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let scratch = Scratch::new("repo");
    let _env = LibraryEnv::set(&scratch.join("empty-library"));
    let project_path = scratch.join("project").join("song.yeban");
    std::fs::create_dir_all(project_path.parent().expect("有父目录")).expect("建工程目录");
    let out = scratch.join("repo-leg.wav");
    let result = render(&project_path, BUILTIN_RELATIVE, &out);
    let sampler = sampler_report(&result);
    let resolved = PathBuf::from(sampler["resolvedPath"].as_str().expect("resolvedPath"));
    assert!(
        resolved.ends_with(BUILTIN_RELATIVE),
        "必须解析到仓库根那条相对路径: {result}"
    );
    assert!(
        resolved.is_file(),
        "解析结果必须是仓库里的真实文件: {result}"
    );
    assert!(
        !scratch
            .join("empty-library/karoryfer-cowsynth/cowsynth_shifty_pluck.sfz")
            .exists(),
        "空库根里没有这份文件 ⇒ 本次命中的只能是仓库根档"
    );
    println!(
        "### [#271 判据3-仓库根] resolvedPath={} label={}",
        sampler["resolvedPath"], sampler["label"]
    );
}

/// 判据 3-cwd：文件只在**进程 cwd** 里 ⇒ 仍然命中（最后一档）。
///
/// ⚠ cwd 档的 `resolvedPath` 保持 `#221` 的**原样**形状（相对 cwd），⛔ 不改成绝对路径 ——
/// 那样会改变"本来就能解析"的路径的读数（判据 4 的同一条纪律）。
#[test]
fn the_cwd_leg_is_still_the_last_resort() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let scratch = Scratch::new("cwd");
    let cwd_dir = scratch.join("cwd");
    let target = write_sfz_with_sample(&cwd_dir, "cwd-only");
    assert!(target.is_file(), "夹具必须真的落在 cwd 里");
    let _env = LibraryEnv::set(&scratch.join("empty-library"));
    let project_path = scratch.join("project").join("song.yeban");
    std::fs::create_dir_all(project_path.parent().expect("有父目录")).expect("建工程目录");
    let _cwd = Cwd::set(&cwd_dir);

    let out = scratch.join("cwd-leg.wav");
    let result = render(&project_path, "cwd-only.sfz", &out);
    let sampler = sampler_report(&result);
    assert_eq!(
        sampler["resolvedPath"],
        "cwd-only.sfz",
        "cwd 是第四档：命中后保留 #221 的原样读数（按 cwd 解析就是 {}）: {result}",
        target.display()
    );
    assert_eq!(
        sampler["sampleFailures"],
        json!([]),
        "cwd 里的样本必须真的读到: {result}"
    );
    println!(
        "### [#271 判据3-cwd] resolvedPath={} cwd={}",
        sampler["resolvedPath"],
        cwd_dir.display()
    );
}

/// 判据 5：四档都找不到 ⇒ 仍然 `samplerUnreadable`，`tried` 四条按序（⛔ 不静默）。
#[test]
fn a_path_missing_from_all_four_roots_is_still_loud() {
    let _guard = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let scratch = Scratch::new("missing");
    let library = scratch.join("empty-library");
    let _env = LibraryEnv::set(&library);
    let project_path = scratch.join("project").join("song.yeban");
    std::fs::create_dir_all(project_path.parent().expect("有父目录")).expect("建工程目录");
    let out = scratch.join("missing.wav");
    let result = render(&project_path, "nowhere.sfz", &out);

    assert_eq!(result["status"], "error", "必须响亮失败: {result}");
    assert_eq!(
        result["error"]["data"]["reason"], "samplerUnreadable",
        "必须点名采样器读不到: {result}"
    );
    let tried: Vec<String> = result["error"]["data"]["tried"]
        .as_array()
        .expect("tried 必须是数组")
        .iter()
        .map(|value| value.as_str().expect("字符串").to_owned())
        .collect();
    let cwd = std::env::current_dir().expect("进程 cwd");
    let repo_raw = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let repo = repo_raw.canonicalize().unwrap_or(repo_raw);
    let expected = [
        project_path
            .parent()
            .expect("有父目录")
            .join("nowhere.sfz")
            .display()
            .to_string(),
        library.join("nowhere.sfz").display().to_string(),
        repo.join("nowhere.sfz").display().to_string(),
        // cwd 档保留 `#221` 的**原样**形状（相对 cwd 解析）：见判据 3-cwd 的说明。
        "nowhere.sfz".to_owned(),
    ];
    assert_eq!(
        tried, expected,
        "候选顺序必须是 工程目录 ⇒ 库根 ⇒ 仓库根 ⇒ cwd: {result}"
    );
    assert!(
        !cwd.join("nowhere.sfz").exists(),
        "cwd 档也不能存在（否则这条判据测的不是'四档都找不到'）"
    );
    assert!(!out.exists(), "失败时不许留下半成品");
    println!("### [#271 判据5] tried({})={:?}", tried.len(), tried);
}

/// ⭐ `#276` 判据 6 的**发声那一半**：**同一份样本被两条轨引用** ⇒ 两条轨都正常发声。
///
/// 夹具：`filled_project()` 的 Lead 轨克隆出一条 `Lead B`（同一条 MIDI 片段、同一个
/// SFZ 绑定、同样直连主总线）；正对照是"只有 Lead 绑定采样器"。
///
/// 读数：
/// * 两条轨的 `sources[].sampler` 各报 `decodedSamples=1`、`notesUsingFileSample=4`、
///   `sampleFailures=[]` ⇒ 两条轨都拿到了同一份文件样本；
/// * 双轨母带峰值 > 单轨 × 1.5 ⇒ **第二条轨真的在叠加声音**（⛔ 不是只解了一次却静音）。
///
/// ⚠ "**解码只发生一次**"那一半由 `crates/yeban-mcp/src/domain/sfz.rs` 的
/// `a_shared_store_decodes_a_sample_once_and_a_second_track_reads_the_pool` 钉住
/// （读完第一条轨后把文件删掉，第二条轨仍拿到同一份 PCM）；本判据只钉发声面。
#[test]
fn two_tracks_sharing_one_sample_file_both_sound() {
    let scratch = Scratch::new("shared-sample");
    let project_dir = scratch.join("project");
    std::fs::create_dir_all(&project_dir).expect("建工程目录");
    let sfz = write_sfz_with_sample(&project_dir, "shared");
    let project_path = project_dir.join("song.yeban");

    // ---- 正对照：只有 Lead 绑定采样器 ----
    let mut single = filled_project();
    let lead = single
        .tracks
        .iter()
        .find(|(_, track)| track.name == "Lead")
        .map(|(id, _)| *id)
        .expect("filled_project 必须有 Lead 轨");
    single.tracks.get_mut(&lead).expect("Lead").sampler = Some(SamplerBinding {
        path: sfz.display().to_string(),
    });
    single.validate().expect("单轨夹具必须合法");
    let single_result = render_project(single.clone(), &project_path, &scratch.join("single.wav"));
    assert_eq!(
        single_result["status"], "success",
        "单轨正对照必须渲染成功: {single_result}"
    );
    let single_peak = single_result["data"]["peak"]["before"]
        .as_f64()
        .expect("peak.before");
    assert!(single_peak > 0.0, "单轨正对照不许是静音: {single_result}");

    // ---- 两轨：Lead ＋ 克隆出来的 Lead B，**同一份** SFZ ----
    let mut two = single;
    let mut second = two.tracks.get(&lead).expect("Lead").clone();
    let second_id = EntityId::new();
    second.id = second_id;
    second.name = "Lead B".to_owned();
    // 设备 / 宏 / 自动化泳道带的是**身份**，克隆会撞 id ⇒ 清空（本判据不测它们）。
    second.devices.clear();
    second.macros.clear();
    second.automation_lanes.clear();
    // 摆放也带身份 ⇒ 换新 id；片段池里的 clip **共享**（同一条 MIDI 内容）。
    let placements: Vec<ClipPlacement> = second.clips.values().cloned().collect();
    second.clips.clear();
    for mut placement in placements {
        let id = EntityId::new();
        placement.id = id;
        second.clips.insert(id, placement);
    }
    two.tracks.insert(second_id, second);
    two.routing_graph.nodes.push(second_id);
    let master = two.master_bus_track_id;
    let edge_id = EntityId::new();
    two.routing_graph.edges.insert(
        edge_id,
        RoutingEdge {
            id: edge_id,
            source_node: second_id,
            destination_node: master,
            kind: RoutingKind::TrackToBus,
            gain_db: None,
        },
    );
    two.validate().expect("两轨夹具必须合法");

    let two_result = render_project(two, &project_path, &scratch.join("two.wav"));
    assert_eq!(
        two_result["status"], "success",
        "两轨必须渲染成功: {two_result}"
    );
    let sampler_sources: Vec<&Value> = two_result["data"]["sources"]
        .as_array()
        .expect("sources")
        .iter()
        .filter(|source| !source["sampler"].is_null())
        .collect();
    assert_eq!(
        sampler_sources.len(),
        2,
        "两条轨都必须有采样器读数: {two_result}"
    );
    for source in &sampler_sources {
        let sampler = &source["sampler"];
        assert_eq!(
            sampler["decodedSamples"], 1,
            "同一份文件 ⇒ 每条轨各解出一份（共享的是**解码**、⛔ 不是提供表）: {two_result}"
        );
        assert_eq!(
            sampler["notesUsingFileSample"], 4,
            "四个音符都必须落在文件样本上: {two_result}"
        );
        assert_eq!(
            sampler["sampleFailures"],
            serde_json::json!([]),
            "样本必须真的读到: {two_result}"
        );
    }
    let two_peak = two_result["data"]["peak"]["before"]
        .as_f64()
        .expect("peak.before");
    assert!(
        two_peak > single_peak * 1.5,
        "两条轨必须**都发声**（双轨峰值应约为单轨两倍）: 单轨={single_peak} 双轨={two_peak}"
    );
    println!(
        "### [#276 判据6] 单轨峰值={single_peak:.6} 双轨峰值={two_peak:.6} 比值={:.4} \
         两轨 sampler: {:?}",
        two_peak / single_peak,
        sampler_sources
            .iter()
            .map(|source| (
                source["sampler"]["decodedSamples"].clone(),
                source["sampler"]["notesUsingFileSample"].clone()
            ))
            .collect::<Vec<_>>()
    );
}
