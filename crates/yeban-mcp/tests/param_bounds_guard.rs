//! **D46 长度约束的机械守卫**（R142/R146/R151：契约与实现**两侧**都要判）＋ **私有目录守卫**（R141）。
//!
//! ## 为什么需要这一份
//!
//! 第六批实测（已入 R159）：把 `maxLength` 写进 `schemas/**` **不会**改变任何行为 ——
//! "契约说有界、实现无界"是 **R142 的静默永不生效**；而 `properties.insert` 的**覆盖**
//! 又会让"校验面"与"广播面"分叉（`yeban_edit_notes` 自己声明了一份 `idempotencyKey`
//! ⇒ 公共那份的 `maxLength` 在 `tools/list` 里**消失**）。本文件把这三件事机械化：
//!
//! | 判据 | 它挡住什么 |
//! | :--- | :--- |
//! | [`every_declared_bound_is_broadcast_and_every_broadcast_bound_is_declared`] | 声明了约束却不广播 / 广播了没声明的约束（**两方向**） |
//! | [`every_bounded_param_has_a_registered_behavioural_criterion`] | 有约束但**没有**任何判据证明它**真的判**（allowlist 归零） |
//! | [`no_private_work_directory_lives_inside_the_repository`] | 私有目录落进仓库树（R141 ⇒ R103 干净性检查会 REFUSED） |
//!
//! ## 单位（R149/R150/R156：口径进表头，⛔ 不数注释行）
//!
//! * 判据一数的单位 = **`ParamSpec` 实例**（`ToolSpec::all_params()` 的笛卡尔展开），
//!   口径是"`min_len`/`max_len` 为 `Some` 的实例"与"`input_schema()` 里带
//!   `minLength`/`maxLength` 的**属性**"两边的**集合相等**；
//! * 判据二数的单位 = **注册表的条目**（`(参数名, 判据名)` 对），⛔ 不是源码行数。

use std::collections::BTreeSet;
use std::path::PathBuf;

use yeban_mcp::tools::{TOOLS, ToolSpec};

/// **已被行为判据证明"真的判"**的受约束参数（R151：每个实例各配一条判据）。
///
/// ⚠ **allowlist 必须归零**（R157）：注册表 = 全部受约束参数 ⇒ 多一个就红、少一个也红。
const ENFORCED_BOUNDS: [(&str, &str); 3] = [
    (
        "idempotencyKey",
        "the_idempotency_key_boundary_is_emptiness_and_256_codepoints",
    ),
    (
        "name",
        "an_empty_import_name_is_rejected_and_never_lands_in_the_document",
    ),
    (
        "license",
        "a_missing_license_is_refused_before_any_file_is_read",
    ),
];

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// 一个受约束参数在某工具里的 `(工具名, 参数名, min, max)`。
fn declared_bounds(spec: &ToolSpec) -> Vec<(String, String, Option<usize>, Option<usize>)> {
    spec.all_params()
        .into_iter()
        .filter(|param| param.min_len.is_some() || param.max_len.is_some())
        .map(|param| {
            (
                spec.name.to_owned(),
                param.name.to_owned(),
                param.min_len,
                param.max_len,
            )
        })
        .collect()
}

/// **每个 string 参数的界状态**（⭐ 第八批：把"覆盖面"机械化 —— ⛔ 不靠人读源码）。
///
/// 口径（单位 = **不同的 JSON 参数名**，登记条目数必须与注册表**双向相等**）：
/// 新增一个 string 参数（或改它的界）⇒ **必须在表里改一行**，否则
/// [`every_string_params_bound_status_is_registered`] 红。
///
/// `Unbounded` 的第二项是 **R161 要求的爆炸半径**（谁能触发／后果），⛔ 不许写"未定"。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BoundStatus {
    /// 有长度约束：`(min_len, max_len)`，必须与 `ParamSpec` **逐值相同**。
    Length(Option<usize>, Option<usize>),
    /// 身份类：由 ULID 解析器定**字符集与长度**（26 字符 Crockford Base32）。
    Identity,
    /// 哈希类：由 SHA-256 解析器定（64 位十六进制）。
    Hash,
    /// 词表／白名单类：由枚举字面量表定。
    WordList,
    /// 结构化文本：由**格式解析器**定（例如 `"N/D"` 拍号）。
    Parsed(&'static str),
    /// 自由文本：**无界**（已按 R161 登记为候选 ＋ 爆炸半径；收口需改 `schemas/**`）。
    Unbounded(&'static str),
}

/// 47 条 string 参数声明（`TOOLS` 里 `json_type == "string"` 的**不同名**）。
const STRING_PARAM_BOUNDS: [(&str, BoundStatus); 47] = [
    ("idempotencyKey", BoundStatus::Length(None, Some(256))),
    ("name", BoundStatus::Length(Some(1), None)),
    ("license", BoundStatus::Length(Some(1), None)),
    ("trackId", BoundStatus::Identity),
    ("clipId", BoundStatus::Identity),
    ("edgeId", BoundStatus::Identity),
    ("pointId", BoundStatus::Identity),
    ("placementId", BoundStatus::Identity),
    ("proposalId", BoundStatus::Identity),
    // ⭐ `#290`：场景身份（26 字符 ULID；解析在 `scene_launch::resolve`）。
    ("sceneId", BoundStatus::Identity),
    ("assetHash", BoundStatus::Hash),
    ("lane", BoundStatus::WordList),
    // #176：`yeban_list_proposals` 的类别过滤 —— 取值是固定词表（section / notes / macro）。
    ("kind", BoundStatus::WordList),
    ("format", BoundStatus::WordList),
    (
        "timeSignature",
        BoundStatus::Parsed("N/D 形状（分子/分母各自有区间）"),
    ),
    (
        "path",
        BoundStatus::Unbounded(
            "本地操作员自选路径：`app:admin` 作用域（stdio 形态 = 能写本进程 stdin 的进程；HTTP 形态 = 持 \nBearer 令牌者）。后果：超长路径只影响该次调用的**错误报文长度**；不改工程、\n不落盘（除它自己指定的目标）。收口面：`schemas/**` 的 `maxLength`（集成者独占）。",
        ),
    ),
    (
        "outDir",
        BoundStatus::Unbounded(
            "同 `path`：诊断包的输出目录，操作员自选。后果限于该次调用的目录创建与报文长度。\n收口面：`schemas/**`。",
        ),
    ),
    (
        "port",
        BoundStatus::Unbounded(
            "MIDI 端口选择（十进制下标或名字子串）：操作员自选的硬件端口标识 ⇒ 长度维无界。\n后果：只影响该次探测报文长度与**匹配歧义**（歧义本身已由 `midiInputAmbiguousPort` 结构化拒绝）。\n收口面：`schemas/**` 的 `maxLength`（集成者独占）。",
        ),
    ),
    (
        "title",
        BoundStatus::Unbounded(
            "新建工程的标题：落进文档 `title` 字段 ⇒ 超长标题会**写进工程文件**（体积放大）。\n收口面：`schemas/**` 的 `maxLength`。",
        ),
    ),
    (
        "clipName",
        BoundStatus::Unbounded(
            "片段名（`create:true` 时的标签）：落进文档 ⇒ 超长名字写进工程文件。收口面：`schemas/**`。",
        ),
    ),
    (
        "commitMessage",
        BoundStatus::Unbounded(
            "提交信息：落进提交图（`Commit`）⇒ 超长信息写进工程文件与历史。收口面：`schemas/**`。",
        ),
    ),
    (
        // ⭐ `ADR-0029` D6：`autoMerge` 的合并提交信息，与 `commitMessage` 同一个落点
        // （同一条 `Commit` 链）⇒ 爆炸半径逐字相同，⛔ 不因为它是新参数就宣称更安全。
        "mergeCommitMessage",
        BoundStatus::Unbounded(
            "合并提交信息：落进提交图（`Commit`）⇒ 超长信息写进工程文件与历史；只在\n`autoMerge:true` 时被消费，缺省沿用手动合并的 `\"merge proposal {id}\"`。\n收口面：`schemas/**` 的 `maxLength`（集成者独占）。",
        ),
    ),
    (
        "sectionName",
        BoundStatus::Unbounded("曲式段落名：落进文档 ⇒ 同上。收口面：`schemas/**`。"),
    ),
    (
        "reason",
        BoundStatus::Unbounded("拒绝提案的原因：落进提交信息 ⇒ 同上。收口面：`schemas/**`。"),
    ),
    (
        "scale",
        BoundStatus::Unbounded(
            "音阶名：**未知值会走词表校验**（`STYLE_NOT_FOUND` / `INVALID_PARAMETER_RANGE`）⇒ \n界的形态是词表而不是长度；这里的无界只指「长度」这一维。",
        ),
    ),
    (
        "stylePreset",
        BoundStatus::Unbounded(
            "风格预设名：同 `scale`（未知值走词表校验），长度维无界。收口面：`schemas/**`。",
        ),
    ),
    // ADR-0016：`yeban_propose_section` 的两个新增可选实参。
    ("materialSource", BoundStatus::WordList),
    (
        "progression",
        BoundStatus::Parsed(
            "罗马数字走向：`Degree::parse` 词表 + 分隔符（`-` / 空格 / `,` / `|`）",
        ),
    ),
    // ADR-0028：`yeban_propose_section` 的鼓密度档（词表 = normal / sparse，
    // 未知值由 `DrumDensity::parse` 拒绝并列出 `accepted`）。
    ("drumDensity", BoundStatus::WordList),
    // ADR-0044：`yeban_propose_section` 的鼓口径档。界的形态是**词表**而不是长度，
    // 且词表**不在 MCP 里抄写** —— 候选集现场取自 theory 的 `DrumStyle::ALL`（`name()`），
    // 未知值由 `drum_style_choice` 拒绝并列出 `accepted`；`schemas/**` 同步声明 `enum`、
    // ⛔ `default`（182 个 GENRES 里 169 走 `metric`／13 走 `four-on-the-floor` ⇒ 无单一默认）。
    ("drumStyle", BoundStatus::WordList),
    // ADR-0069：`yeban_load_plugin` 的插件路径与类指名。两者都是操作员自选的
    // 本机路径／类标识：长度维无界（路径长度由操作系统定，类名／CID 由插件厂商定）。
    // 后果限于该次调用的报文长度；绑定记录落盘时原值进工程文件（与 `path` 同一条纪律）。
    // 收口面：`schemas/**` 的 `maxLength`（集成者独占）。
    (
        "pluginPath",
        BoundStatus::Unbounded(
            "插件路径：操作员自选的本机 .vst3 包／动态库路径或测试桩标识。后果限于该次调用的加载失败报文长度；绑定记录落盘时原值进工程文件（与 `path` 同一条纪律）。\n收口面：`schemas/**` 的 `maxLength`（集成者独占）。",
        ),
    ),
    (
        "pluginId",
        BoundStatus::Unbounded(
            "插件类指名：类名或 32 位十六进制 CID，取值空间由插件厂商定。未知值走类查找拒绝（`INVALID_PARAMETER_RANGE`），长度维无界。后果限于该次调用的报文长度。\n收口面：`schemas/**` 的 `maxLength`（集成者独占）。",
        ),
    ),
    // SVS：`synthesize_singing` 与 `svs_job` 新增的 4 个不同名 string 参数（`license` 已在表内，
    // `trackId` 已在表内；两处 `service_url` 的规格一致，见方向 ③）。
    (
        "service_url",
        BoundStatus::Parsed(
            "http URL 形状：只支持 http、有 host、端口全数字（`domain::svs` 的服务地址解析）",
        ),
    ),
    (
        "job_id",
        BoundStatus::Parsed(
            "服务端生成的 16 位十六进制 id；本层只接受字母/数字/`_`/`-`，防 URL 路径与暂存文件名越界",
        ),
    ),
    ("want", BoundStatus::WordList),
    (
        "part",
        BoundStatus::Unbounded(
            "调用方自选的任务内 part 标签；后果限于该次调用的查找失败与报文长度。收口面：`schemas/**`。",
        ),
    ),
    // 夜半原生预制 4 条新增的 3 个不同名 string 参数（`trackId`、`name`、
    // `mergeCommitMessage` 已在表内；`categories` 是 array，不进本表）。
    (
        "instrument",
        BoundStatus::Unbounded(
            "乐器 slug 过滤（`yeban_list_presets`）：调用方自选的过滤串；未知 slug 只滤出空列表，不写工程、不落盘。后果限于该次调用的报文长度。\n收口面：`schemas/**` 的 `maxLength`（集成者独占）。",
        ),
    ),
    (
        "presetId",
        BoundStatus::Parsed(
            "`sys:<乐器>:<名>` 或 `user:<ulid>`（`domain::preset::resolve_preset` 的标识形状）；查不到 ⇒ `ENTITY_NOT_FOUND`，响亮拒绝，不静默回干声",
        ),
    ),
    // `yeban_save_preset` 的 scope：`instrument`（乐器调优，缺省）| `track`
    // （整轨快照）；其它值 ⇒ `INVALID_PARAMETER_RANGE`。
    ("scope", BoundStatus::WordList),
    // 整曲总览图：`get_song_overview` 的 `resolution` —— 取值是固定词表（`bar` / `beat`），
    // 未知值由 `plan_song_overview` 以 `unknownResolution` 拒绝并列出 `accepted`。
    ("resolution", BoundStatus::WordList),
    // 整曲总览图 Phase 2：`get_song_overview` 的 `profile` —— 取值是固定词表
    // （`default` / `orchestral` / `electronic`），未知值由 `plan_song_overview`
    // 以 `unknownProfile` 拒绝并列出 `accepted`。
    ("profile", BoundStatus::WordList),
    // 整曲总览图 HTML 报告：`get_song_overview` 的 `format` —— 取值是固定词表
    // （`json` / `html`），未知值由 `plan_song_overview` 以 `unknownFormat`
    // 拒绝并列出 `accepted`。
    ("format", BoundStatus::WordList),
    // ARA 宿主：`yeban_ara_bind` / `yeban_ara_query` / `yeban_ara_render` 新增的
    // 3 个不同名 string 参数（`pluginPath`、`pluginId` 已在表内）。
    (
        "wavPath",
        BoundStatus::Unbounded(
            "片段 wav 路径（`yeban_ara_bind`）：操作员自选的本机音频文件路径，解码失败 ⇒ `IO_ERROR`。\\n后果限于该次调用的解码失败报文长度；路径本身不进工程文件（会话只活在内存注册表里）。\\n收口面：`schemas/**` 的 `maxLength`（集成者独占）。",
        ),
    ),
    (
        "sessionId",
        BoundStatus::Unbounded(
            "ARA 会话 id（`yeban_ara_query` / `yeban_ara_render`）：服务端生成的 `ara-N`（自增数），\\n本层只作注册表键查找；未知值 ⇒ `INVALID_PARAMETER_RANGE` + `araSessionNotFound`。\\n后果限于该次调用的报文长度；不写工程、不落盘。收口面：`schemas/**` 的 `maxLength`（集成者独占）。",
        ),
    ),
    (
        "outPath",
        BoundStatus::Unbounded(
            "渲染输出 wav 路径（`yeban_ara_render`）：操作员自选的本机输出路径；父目录不存在 ⇒ `IO_ERROR`。\\n后果限于该次调用的落盘失败报文长度与该路径的一次写盘。收口面：`schemas/**` 的 `maxLength`（集成者独占）。",
        ),
    ),
    // ADR-0070：OTCP 音高校正新增的 2 个不同名 string 参数。
    // `mode`：校正模式词表（snapNotes / regenerate / notesOnly），
    // 未知值由 `otcp::parse_mode` 拒绝；`regenerate` / `notesOnly` 在 v1
    // 同样响亮拒绝（须先在无头环境探针验证，见设计稿 §9）。
    ("mode", BoundStatus::WordList),
    (
        "contentId",
        BoundStatus::Parsed(
            "OTCP 服务端返回的不透明 contentId，原样透传作任务收据查找键；未知值 ⇒ `otcpNotesUnavailable`",
        ),
    ),
];

#[test]
fn every_declared_bound_is_broadcast_and_every_broadcast_bound_is_declared() {
    // ⭐ R56：先喂**一条已知红**（广播缺 `minLength`）与**一条已知绿**（广播与声明一致）。
    let red = serde_json::json!({"type": "string"});
    let green = serde_json::json!({"type": "string", "minLength": 1});
    let broadcast = |property: &serde_json::Value| -> (Option<u64>, Option<u64>) {
        (
            property
                .get("minLength")
                .and_then(serde_json::Value::as_u64),
            property
                .get("maxLength")
                .and_then(serde_json::Value::as_u64),
        )
    };
    assert_eq!(broadcast(&red), (None, None), "已知红样本：广播里没有约束");
    assert_eq!(
        broadcast(&green),
        (Some(1), None),
        "已知绿样本：广播里有约束"
    );

    // 方向 ①：凡**声明**了约束的实例，`tools/list` 里必须**如实广播**（同一对数值）。
    // 方向 ②：凡**广播**了约束的属性，注册表里必须**真的声明**了它。
    let mut declared: BTreeSet<(String, Option<usize>, Option<usize>)> = BTreeSet::new();
    let mut broadcasted: BTreeSet<(String, Option<usize>, Option<usize>)> = BTreeSet::new();
    let mut instances = 0usize;
    let mut constrained_instances = 0usize;
    let mut failures: Vec<String> = Vec::new();
    for spec in &TOOLS {
        let schema = spec.input_schema();
        let properties = schema["properties"]
            .as_object()
            .unwrap_or_else(|| panic!("`{}` 的 inputSchema 必须有 properties", spec.name));
        for (tool, name, min, max) in declared_bounds(spec) {
            instances += 1;
            constrained_instances += 1;
            declared.insert((name.clone(), min, max));
            let (seen_min, seen_max) = broadcast(&properties[&name]);
            if seen_min != min.map(|value| value as u64)
                || seen_max != max.map(|value| value as u64)
            {
                failures.push(format!(
                    "{tool}.{name} 声明 (min={min:?}, max={max:?}) 但广播 (min={seen_min:?}, max={seen_max:?})"
                ));
            }
        }
        for (name, property) in properties {
            let (min, max) = broadcast(property);
            if min.is_some() || max.is_some() {
                broadcasted.insert((
                    name.clone(),
                    min.map(|v| v as usize),
                    max.map(|v| v as usize),
                ));
            }
        }
    }
    // ⭐ R119/R120（覆盖假象）：被扫集合必须非空且达下界 —— 否则"没有违规"可能只是**什么都没扫到**。
    assert!(
        declared.len() >= 2 && constrained_instances >= 17,
        "受约束参数的种类必须 ≥ 2、实例必须 ≥ 17（公共参数会展开到每个工具）：\
         declared={declared:?} instances={instances} constrained={constrained_instances}"
    );
    assert_eq!(
        declared.len(),
        broadcasted.len(),
        "受约束的**种类**数必须两边相等（声明 {declared:?} vs 广播 {broadcasted:?}）"
    );
    assert_eq!(
        declared, broadcasted,
        "声明与广播必须是**同一个集合**（方向 ① 声明了要广播 / 方向 ② 广播了要声明）"
    );
    assert!(failures.is_empty(), "声明与广播的数值不一致：{failures:#?}");
}

#[test]
fn every_bounded_param_has_a_registered_behavioural_criterion() {
    let mut unregistered: Vec<String> = Vec::new();
    let mut bounded_names: BTreeSet<String> = BTreeSet::new();
    for spec in &TOOLS {
        for (_tool, name, _min, _max) in declared_bounds(spec) {
            bounded_names.insert(name);
        }
    }
    for name in &bounded_names {
        if !ENFORCED_BOUNDS.iter().any(|(bound, _)| bound == name) {
            unregistered.push(name.clone());
        }
    }
    // allowlist 归零（R157）：注册表既不能少（有约束没人证明）也不能多（注册了不存在的约束）。
    assert!(
        unregistered.is_empty(),
        "这些受约束参数**没有**行为判据证明它真的判：{unregistered:?}"
    );
    let mut stale: Vec<&str> = Vec::new();
    for (bound, criterion) in ENFORCED_BOUNDS {
        if !bounded_names.contains(bound) {
            stale.push(bound);
        }
        assert!(
            TOOLS.iter().any(|spec| spec
                .all_params()
                .iter()
                .any(|param| param.name == bound
                    && (param.min_len.is_some() || param.max_len.is_some()))),
            "注册表里的 `{bound}` 在 `ParamSpec` 里找不到对应约束（注册表腐烂）"
        );
        assert!(
            !criterion.is_empty(),
            "每个实例都必须点名它自己的判据（⛔ 不许只写'已测'）"
        );
    }
    assert!(stale.is_empty(), "注册表里有不存在的参数：{stale:?}");
    // 判据名必须真的存在于测试源码里（否则"注册"只是字符串）——见下一条判据。
    assert_eq!(
        ENFORCED_BOUNDS.len(),
        bounded_names.len(),
        "注册表必须**恰好**覆盖"
    );
}

#[test]
fn the_registered_criteria_really_exist_in_the_sources() {
    let root = manifest_dir();
    let mut sources = String::new();
    let mut files = 0usize;
    for dir in [root.join("src"), root.join("tests")] {
        let mut stack = vec![dir];
        while let Some(current) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&current) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                if path.extension().and_then(std::ffi::OsStr::to_str) != Some("rs") {
                    continue;
                }
                if let Ok(text) = std::fs::read_to_string(&path) {
                    files += 1;
                    sources.push_str(&text);
                    sources.push('\n');
                }
            }
        }
    }
    // ⭐ R119/R120：扫描面非空且达下界（本 crate 的 .rs 文件数 ≥ 50）。
    assert!(
        files >= 50,
        "源码扫描面太小（{files} 个 .rs）—— 目录枚举可能写错了"
    );
    for (bound, criterion) in ENFORCED_BOUNDS {
        assert!(
            sources.contains(&format!("fn {criterion}(")),
            "注册表点名的判据 `{criterion}`（为 `{bound}`）在源码里找不到"
        );
    }
}

#[test]
fn no_private_work_directory_lives_inside_the_repository() {
    // ⭐ R141：私有工作目录必须在**会话工作区根**（`/Users/crow/work/music/.mod-mcp/`），
    // ⛔ 不在仓库工作树内（否则 R103 的干净性检查会 REFUSED）。
    // 单位 = **仓库工作树内的 `.mod-*` 目录条目数**（必须为 0）。
    let repo_root = manifest_dir()
        .parent() // crates/
        .and_then(|path| path.parent()) // 仓库根（worktree）
        .expect("仓库根")
        .to_path_buf();
    let mut offenders: Vec<String> = Vec::new();
    let mut scanned = 0usize;
    // 只扫**仓库根的第一层**与 `crates/**` 的一层：私有目录按约定落在根或 crate 根。
    let mut dirs = vec![repo_root.clone(), repo_root.join("crates")];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            scanned += 1;
            if name.starts_with(".mod-") {
                offenders.push(entry.path().display().to_string());
            }
        }
    }
    assert!(
        scanned >= 10,
        "目录扫描面太小（{scanned} 个条目）—— 仓库根解析可能错了"
    );
    assert!(
        offenders.is_empty(),
        "仓库树内不得有私有工作目录（R141：应放在会话工作区根的 `.mod-mcp/`）：{offenders:#?}"
    );
}

/// **每个 string 参数的界状态都必须在表里**（两方向 ＋ 非真空 ＋ 与注册表逐值一致）。
///
/// 注入（配对已知红）：把某个 `Unbounded` 条目从表里删掉 ⇒ 本判据红（"新参数没登记"）；
/// 把某个 `Length` 的数值改掉 ⇒ 本判据红（表与注册表不一致）。
#[test]
fn every_string_params_bound_status_is_registered() {
    let declared: BTreeSet<String> = TOOLS
        .iter()
        .flat_map(|spec| spec.all_params())
        .filter(|param| param.json_type == "string")
        .map(|param| param.name.to_owned())
        .collect();
    let registered: BTreeSet<String> = STRING_PARAM_BOUNDS
        .iter()
        .map(|(name, _status)| (*name).to_owned())
        .collect();
    // ⭐ R119/R120：非真空下界（本 crate 的 string 参数声明数）。
    assert!(
        declared.len() >= 20,
        "string 参数声明数太小（{}）—— 注册表枚举可能写错了",
        declared.len()
    );
    // 方向 ①：注册表里的每个 string 参数都必须在表里；方向 ②：表里不许有注册表没有的名字。
    let missing: Vec<&String> = declared.difference(&registered).collect();
    let stale: Vec<&String> = registered.difference(&declared).collect();
    assert!(
        missing.is_empty(),
        "这些 string 参数**没有登记界状态**（新增参数必须在本文件的表里改一行）：{missing:?}"
    );
    assert!(stale.is_empty(), "表里有注册表中不存在的参数：{stale:?}");
    // 方向 ③：`Length` 条目必须与 `ParamSpec` **逐值相同**。
    let mut mismatches: Vec<String> = Vec::new();
    for (name, status) in STRING_PARAM_BOUNDS {
        let specs: Vec<&yeban_mcp::tools::ParamSpec> = TOOLS
            .iter()
            .flat_map(|spec| spec.all_params())
            .filter(|param| param.name == name)
            .collect();
        assert!(!specs.is_empty(), "表里的 `{name}` 在注册表里找不到");
        let seen: BTreeSet<(Option<usize>, Option<usize>)> = specs
            .iter()
            .map(|param| (param.min_len, param.max_len))
            .collect();
        assert_eq!(seen.len(), 1, "`{name}` 在不同工具里的界不一致：{seen:?}");
        let actual = seen.into_iter().next().expect("非空");
        match status {
            BoundStatus::Length(min, max) => {
                if actual != (min, max) {
                    mismatches.push(format!("{name}: 表 ({min:?},{max:?}) vs 注册表 {actual:?}"));
                }
            }
            other => {
                if actual != (None, None) {
                    mismatches.push(format!(
                        "{name}: 表登记为 {other:?}，但注册表里带了长度界 {actual:?}"
                    ));
                }
            }
        }
        // `Unbounded` 的**爆炸半径**必须写明（R161：⛔ 不许"未定"）。
        if let BoundStatus::Unbounded(blast) = status {
            assert!(
                blast.chars().count() >= 20 && !blast.contains("未定"),
                "`{name}` 的爆炸半径太短或写了'未定'：{blast}"
            );
        }
    }
    assert!(mismatches.is_empty(), "表与注册表不一致：{mismatches:#?}");
}
