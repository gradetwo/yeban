//! ⭐ SFZ Doctor —— **非官方音源**的体检 + 自动修复（用户确认后才跑）。
//!
//! ## 为什么有这一层
//!
//! 音源管理里没有包标记（[`crate::sound_library::PACK_MARKER_FILE`]）的用户库，
//! 是用户从别处下载 / 拷进来的第三方 SFZ 库。这类库按老 SFZ 生态的习惯写成，
//! 常见三类与夜半引擎口径不一致的毛病（2026-11 全量审计实测，见仓外报告
//! `sfz-octave-audit` / `load-fix-report`）：
//!
//! 1. **采样路径大小写对不上**（库在大小写不敏感的系统上写成）：引擎按原样
//!    找文件 ⇒ 整件乐器无声。本层给「只差大小写」的那一段建符号链接，
//!    ⛔ 不改 SFZ 文本、不动采样本体。
//! 2. **操作码超出引擎合法区间**（`fil_veltrack=10000`、`tune=-109`、
//!    `amp_keycenter` 写音名等）：引擎的取舍是**硬拒绝整件乐器**。本层把
//!    越界值钳回区间边界（差几音分，听感无损），或把音名换算成 IPN 数字。
//! 3. 上两类之外的解析失败：把引擎自己的报错**原样**报给用户，⛔ 不猜、不硬修。
//!
//! ## 触发口径（⛔ 不许静默自动改用户的库）
//!
//! - 官方库（内置 / 带包标记）**永不**进本层；
//! - 非官方库第一次被扫描到时，界面在那一行给出提示，用户点「开始检查」
//!   才跑 [`run`]；点「不用了」写一份 declined 记录，之后不再提示；
//! - 每次 [`run`] 的全部改动（建了哪些链接、改了哪些值、还有哪些修不了）
//!   都写进库目录里的记录文件 [`DOCTOR_RECORD_FILE`]，并由
//!   [`format_doctor_readout`] 换算成一行人话显示在界面状态行。
//!
//! 八度类问题（采样元数据虚标等）**不在**本层处理：修法是绑定时的
//! `noteOctaveOverride`，属于工程层，不动库文件。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::cli::Language;
use crate::sound_library::{InstalledLibrary, PACK_MARKER_FILE, USER_ID_PREFIX};

/// 体检记录文件名（写在库目录根；扫描时以它判「查过了 / 拒绝过」）。
pub const DOCTOR_RECORD_FILE: &str = ".sfz-doctor.json";

/// 一件已安装库在 SFZ Doctor 眼里的状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DoctorState {
    /// 官方库（内置或带包标记）：不提示、不检查。
    Official,
    /// 非官方库且从未表态：界面应当在那一行给出提示。
    Needed,
    /// 已经检查并修复过（记录在案）。
    Done,
    /// 用户明确拒绝过：不再提示。
    Declined,
    /// 记录文件存在但读不出或解析失败：仍需提示，且界面应当先给一次可见警告。
    Corrupt,
}

/// 这件库现在是什么状态。判据只看两样东西：包标记与体检记录，⛔ 不读 SFZ 内容。
/// 记录文件存在但读不出/解析失败时返回 [`DoctorState::Corrupt`]（不是 Needed：
/// 调用方要先给一次可见警告，并把坏记录隔离备份）。
#[must_use]
pub fn doctor_state(library: &InstalledLibrary) -> DoctorState {
    if !library.id.starts_with(USER_ID_PREFIX) {
        return DoctorState::Official;
    }
    if library.location.join(PACK_MARKER_FILE).exists() {
        return DoctorState::Official;
    }
    match read_record(&library.location) {
        RecordRead::Ok(record) => {
            if record.get("outcome").and_then(|v| v.as_str()) == Some("declined") {
                DoctorState::Declined
            } else {
                DoctorState::Done
            }
        }
        RecordRead::Missing => DoctorState::Needed,
        RecordRead::Corrupt => DoctorState::Corrupt,
    }
}

/// 用户点了「不用了」：落一份 declined 记录，之后 [`doctor_state`] 不再提示。
///
/// 落盘失败会返回 `Err` —— 调用方必须按失败写状态（不得显示"不再提示"），
/// 否则下次启动读不到记录又会重新提示（T-FIX-306：`let _ =` 吞掉的假成功）。
pub fn mark_declined(dir: &Path) -> std::io::Result<()> {
    let record = serde_json::json!({
        "version": 1,
        "outcome": "declined",
        "checked_at_epoch_s": epoch_s(),
    });
    fs::write(
        dir.join(DOCTOR_RECORD_FILE),
        serde_json::to_string_pretty(&record).unwrap_or_default(),
    )
}

/// 一次被改动的操作码（记录与读数都用它，⛔ 不只报计数）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DoctorEdit {
    /// 被改的文件。
    pub file: PathBuf,
    /// 操作码名（如 `tune`）。
    pub opcode: String,
    /// 原值（文本原样）。
    pub old: String,
    /// 新值（文本原样）。
    pub new: String,
}

/// 一次 [`run`] 的完整结果。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DoctorReport {
    /// 探针检查过的顶层 SFZ 数。
    pub files_checked: usize,
    /// 大小写通道建的符号链接（链接路径）。
    pub links_created: Vec<PathBuf>,
    /// 钳值 / 换算的操作码改动。
    pub edits: Vec<DoctorEdit>,
    /// 修完仍装不上的文件 + 引擎原样报错（截断到一行）。
    pub still_failing: Vec<(PathBuf, String)>,
    /// 体检记录落盘失败时的错误（`None` = 记录已写好）。体检本身可能全绿，
    /// 但记录没存住 ⇒ 下次启动仍会提示，调用方必须把这条报出来（T-FIX-306）。
    pub record_error: Option<String>,
}

/// 对一个库目录跑完整体检 + 修复，并把结果写进记录文件。
///
/// 三趟：大小写链接 → 已知钳值 → 引擎探针回路（拿引擎报错里给的区间
/// 通用钳值，最多 6 轮）。全程只动**这个目录之内**的文件。
#[must_use]
pub fn run(root: &Path) -> DoctorReport {
    let mut report = DoctorReport::default();
    let tops = top_level_sfzs(root);

    // ---- 第一趟：采样路径大小写 -------------------------------------------
    let mut links: BTreeMap<PathBuf, String> = BTreeMap::new();
    for file in &tops {
        let main_dir = file.parent().unwrap_or(Path::new("/")).to_path_buf();
        for region in parse_regions(file) {
            let Some(sample) = region.sample else {
                continue;
            };
            if sample == "*silence" {
                continue;
            }
            let sample = sample.replace('\\', "/");
            let resolved = if sample.starts_with('/') {
                PathBuf::from(&sample)
            } else {
                normalize_join(&main_dir, &region.default_path.replace('\\', "/"), &sample)
            };
            if resolved.exists() {
                continue;
            }
            if let Some((link, target)) = first_diff_link(&resolved) {
                links.entry(link).or_insert(target);
            }
        }
    }
    for (link, target) in &links {
        if link.is_symlink() || link.exists() {
            continue;
        }
        #[cfg(unix)]
        {
            if std::os::unix::fs::symlink(target, link).is_ok() {
                report.links_created.push(link.clone());
            }
        }
        #[cfg(windows)]
        {
            // Windows 分文件与目录两种符号链接；目标已在盘上，is_dir 可判。
            // 建不上（无权限等）不假报：探针回路会把装不上的文件记进 still_failing。
            let created = if Path::new(target.as_str()).is_dir() {
                std::os::windows::fs::symlink_dir(target, link)
            } else {
                std::os::windows::fs::symlink_file(target, link)
            };
            if created.is_ok() {
                report.links_created.push(link.clone());
            }
        }
    }

    // ---- 第二趟：已知钳值（全树 .sfz/.txt） --------------------------------
    for path in walk_files(root, &["sfz", "txt"]) {
        clamp_file_known(&path, &mut report.edits);
    }

    // ---- 第三趟：引擎探针回路 ---------------------------------------------
    for file in &tops {
        report.files_checked += 1;
        let mut verdict = probe(file);
        let mut rounds = 0;
        while let Err(message) = &verdict {
            if rounds >= 6 {
                break;
            }
            let Some((opcode, lo, hi)) = parse_range_error(message) else {
                break;
            };
            let mut changed = clamp_opcode_in_file(file, &opcode, lo, hi, &mut report.edits);
            if !changed {
                let dir = file.parent().unwrap_or(Path::new("/")).to_path_buf();
                for sibling in walk_files(&dir, &["sfz", "txt"]) {
                    if clamp_opcode_in_file(&sibling, &opcode, lo, hi, &mut report.edits) {
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
            verdict = probe(file);
            rounds += 1;
        }
        if let Err(message) = verdict {
            let one_line: String = message
                .lines()
                .find(|l| l.contains("SFZ") || l.contains("失败"))
                .unwrap_or(&message)
                .chars()
                .take(160)
                .collect();
            report.still_failing.push((file.clone(), one_line));
        }
    }

    // ---- 记录落盘 ----------------------------------------------------------
    // T-FIX-306：落盘失败不再 `let _ =` 吞掉 —— 体检本身可能全绿，但记录没
    // 存住 ⇒ 下次启动仍会提示。失败记进 `record_error`，由调用方报给用户。
    let record = serde_json::json!({
        "version": 1,
        "outcome": "checked",
        "checked_at_epoch_s": epoch_s(),
        "files_checked": report.files_checked,
        "links_created": report.links_created.len(),
        "edits": report.edits.len(),
        "still_failing": report.still_failing.len(),
    });
    if let Err(err) = fs::write(
        root.join(DOCTOR_RECORD_FILE),
        serde_json::to_string_pretty(&record).unwrap_or_default(),
    ) {
        report.record_error = Some(one_line_io_error(&err));
    }
    report
}

/// 把一次体检结果换算成一行人话（界面状态行口径，与下载读数同一条纪律：
/// 换算只在产生数据的地方做一次）。
///
/// 记录落盘失败时读数尾巴带一句"检查完成，但记录未能保存，下次可能再次提示"，
/// 调用方应当把它当错误状态显示（T-FIX-306）。
///
/// T-FIX-454：旧实现拼了三条硬编码中文模板，英文态下状态行照样显示中文。
/// 文案是常驻 UI 显示（体检结局经跨线程槽排空进状态行），按
/// `docs/app-i18n-notes.md` §4 收尾补记（2026-11-17 裁定）须译；形态与
/// T-FIX-432 的设置警告同款——模板按当前语言渲染
/// （[`format_doctor_readout_in`]），九语模板表住在本文件，由单测逐语看守；
/// 不走 Slint 的 `@tr`/`.po`（`@tr` 是 Slint 编译期宏，Rust 代码调不了；
/// `.po` 里加一条没有 `@tr` 调用点的词条会被 `i18n_coverage` 的词表精确
/// 相等判据当孤儿条打红）。库名与引擎报错原文按 §4 口径不译，原样进占位。
/// 本函数在体检工人线程里调用，[`crate::host::current_language`] 读原子
/// 变量，线程安全。
#[must_use]
pub fn format_doctor_readout(name: &str, report: &DoctorReport) -> String {
    format_doctor_readout_in(name, report, crate::host::current_language())
}

/// 在 `lang` 下的完整展示读数。中文态与旧实现逐字相同（单测钉死）；库名与
/// 引擎报错原文不译，原样进占位。
#[must_use]
pub fn format_doctor_readout_in(name: &str, report: &DoctorReport, lang: Language) -> String {
    use Language::{DeDe, EnUs, EsEs, FrFr, JaJp, KoKr, PtBr, RuRu, ZhCn};
    let mut out = if report.still_failing.is_empty() {
        match lang {
            ZhCn => format!(
                "SFZ Doctor：《{name}》检查完成 —— {} 件乐器全部可装载（建大小写链接 {}、修正操作码 {}）",
                report.files_checked,
                report.links_created.len(),
                report.edits.len()
            ),
            EnUs => format!(
                "SFZ Doctor: “{name}” check complete — all instruments loadable (checked: {}, case links created: {}, opcodes fixed: {})",
                report.files_checked,
                report.links_created.len(),
                report.edits.len()
            ),
            FrFr => format!(
                "SFZ Doctor : vérification de « {name} » terminée — tous les instruments se chargent ({} vérifiés, liens de casse créés : {}, opcodes corrigés : {})",
                report.files_checked,
                report.links_created.len(),
                report.edits.len()
            ),
            EsEs => format!(
                "SFZ Doctor: revisión de «{name}» completada — todos los instrumentos cargan ({} revisados, enlaces de mayúsculas creados: {}, opcodes corregidos: {})",
                report.files_checked,
                report.links_created.len(),
                report.edits.len()
            ),
            PtBr => format!(
                "SFZ Doctor: verificação de «{name}» concluída — todos os instrumentos carregam ({} verificados, links de maiúsculas criados: {}, opcodes corrigidos: {})",
                report.files_checked,
                report.links_created.len(),
                report.edits.len()
            ),
            JaJp => format!(
                "SFZ Doctor：《{name}》のチェック完了 —— {} 件の楽器がすべて読み込み可能（大文字小文字リンク {} 件作成、オペコード {} 件修正）",
                report.files_checked,
                report.links_created.len(),
                report.edits.len()
            ),
            KoKr => format!(
                "SFZ Doctor: “{name}” 검사 완료 — {}개 악기 모두 로드 가능(대소문자 링크 {}개 생성, 오프코드 {}개 수정)",
                report.files_checked,
                report.links_created.len(),
                report.edits.len()
            ),
            RuRu => format!(
                "SFZ Doctor: проверка «{name}» завершена — все инструменты загружаются (проверено: {}, создано ссылок регистра: {}, исправлено опкодов: {})",
                report.files_checked,
                report.links_created.len(),
                report.edits.len()
            ),
            DeDe => format!(
                "SFZ Doctor: Prüfung von „{name}“ abgeschlossen — alle Instrumente ladbar (geprüft: {}, Case-Links erstellt: {}, Opcodes korrigiert: {})",
                report.files_checked,
                report.links_created.len(),
                report.edits.len()
            ),
        }
    } else {
        let first_error = report
            .still_failing
            .first()
            .map(|(_, e)| e.as_str())
            .unwrap_or_default();
        match lang {
            ZhCn => format!(
                "SFZ Doctor：《{name}》已修复大小写链接 {}、操作码 {}；仍有 {} 件装不上（{}）",
                report.links_created.len(),
                report.edits.len(),
                report.still_failing.len(),
                first_error
            ),
            EnUs => format!(
                "SFZ Doctor: “{name}” — case links fixed: {}, opcodes fixed: {}; {} still failing to load ({})",
                report.links_created.len(),
                report.edits.len(),
                report.still_failing.len(),
                first_error
            ),
            FrFr => format!(
                "SFZ Doctor : « {name} » — liens de casse corrigés : {}, opcodes corrigés : {} ; encore {} à charger ({})",
                report.links_created.len(),
                report.edits.len(),
                report.still_failing.len(),
                first_error
            ),
            EsEs => format!(
                "SFZ Doctor: «{name}» — enlaces de mayúsculas corregidos: {}, opcodes corregidos: {}; aún {} por cargar ({})",
                report.links_created.len(),
                report.edits.len(),
                report.still_failing.len(),
                first_error
            ),
            PtBr => format!(
                "SFZ Doctor: «{name}» — links de maiúsculas corrigidos: {}, opcodes corrigidos: {}; ainda {} por carregar ({})",
                report.links_created.len(),
                report.edits.len(),
                report.still_failing.len(),
                first_error
            ),
            JaJp => format!(
                "SFZ Doctor：《{name}》大文字小文字リンク {} 件、オペコード {} 件を修正；{} 件はまだ読み込めません（{}）",
                report.links_created.len(),
                report.edits.len(),
                report.still_failing.len(),
                first_error
            ),
            KoKr => format!(
                "SFZ Doctor: “{name}” 대소문자 링크 {}개, 오프코드 {}개 수정; {}개는 여전히 로드되지 않음({})",
                report.links_created.len(),
                report.edits.len(),
                report.still_failing.len(),
                first_error
            ),
            RuRu => format!(
                "SFZ Doctor: «{name}» — исправлено ссылок регистра: {}, опкодов: {}; не загружается: {} ({})",
                report.links_created.len(),
                report.edits.len(),
                report.still_failing.len(),
                first_error
            ),
            DeDe => format!(
                "SFZ Doctor: „{name}“ — Case-Links korrigiert: {}, Opcodes korrigiert: {}; weiterhin nicht ladbar: {} ({})",
                report.links_created.len(),
                report.edits.len(),
                report.still_failing.len(),
                first_error
            ),
        }
    };
    if report.record_error.is_some() {
        out.push_str(match lang {
            ZhCn => "；检查完成，但记录未能保存，下次可能再次提示",
            EnUs => {
                "; check complete, but the record failed to save and you may be prompted again next time"
            }
            FrFr => {
                " ; vérification terminée, mais l’enregistrement n’a pas pu être sauvegardé et pourra être proposé à nouveau la prochaine fois"
            }
            EsEs => {
                "; revisión completada, pero no se pudo guardar el registro y es posible que se le vuelva a avisar la próxima vez"
            }
            PtBr => {
                "; verificação concluída, mas o registro não foi salvo e poderá ser solicitado novamente na próxima vez"
            }
            JaJp => {
                "；チェックは完了しましたが、記録を保存できなかったため、次回再度通知される可能性があります"
            }
            KoKr => {
                "; 검사는 완료되었지만 기록을 저장하지 못해 다음에 다시 알림이 표시될 수 있습니다"
            }
            RuRu => {
                "; проверка завершена, но запись не сохранена, и при следующем запуске может появиться повторное напоминание"
            }
            DeDe => {
                "; Prüfung abgeschlossen, aber der Eintrag konnte nicht gespeichert werden und wird beim nächsten Start möglicherweise erneut gemeldet"
            }
        });
    }
    out
}

/// 一行 io 错误（截断到 160 字符，写进记录/状态行不撑爆界面）。
fn one_line_io_error(err: &std::io::Error) -> String {
    err.to_string().chars().take(160).collect()
}

// ---------------------------------------------------------------- 记录

/// 读体检记录的三种结局：T-FIX-306 之前读失败与"没有记录"都坍缩成 `None`，
/// 于是坏记录与从未表态无法区分（`doctor_state` 一律回 Needed，静默重提示）。
/// 现在区分开：`Missing`（文件不存在）与 `Corrupt`（文件存在但读不出/解析失败）。
enum RecordRead {
    /// 记录文件不存在。
    Missing,
    /// 记录读出并解析成功。
    Ok(serde_json::Value),
    /// 记录文件存在，但读不出（权限/目录占位等）或解析失败。
    Corrupt,
}

fn read_record(dir: &Path) -> RecordRead {
    let path = dir.join(DOCTOR_RECORD_FILE);
    if !path.exists() {
        return RecordRead::Missing;
    }
    let Ok(text) = fs::read_to_string(&path) else {
        return RecordRead::Corrupt;
    };
    match serde_json::from_str(&text) {
        Ok(value) => RecordRead::Ok(value),
        Err(_) => RecordRead::Corrupt,
    }
}

/// 把一件坏记录隔离备份：改名成 `.sfz-doctor.json.corrupt-<纳秒>`，返回备份路径。
///
/// 只有 `doctor_state` 判定为 `Corrupt` 时才调。成功后该目录回到 Needed（下次
/// 提示体检），坏记录本身留着备查，不删除。改名失败返回 `None`（调用方不报
/// 告、不反复警告；行内仍提示体检）。
#[must_use]
pub fn quarantine_corrupt_record(dir: &Path) -> Option<PathBuf> {
    if !matches!(read_record(dir), RecordRead::Corrupt) {
        return None;
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let mut backup = dir.join(format!("{DOCTOR_RECORD_FILE}.corrupt-{nanos}"));
    for suffix in 1..100 {
        if !backup.exists() {
            break;
        }
        backup = dir.join(format!("{DOCTOR_RECORD_FILE}.corrupt-{nanos}-{suffix}"));
    }
    if backup.exists() {
        return None;
    }
    fs::rename(dir.join(DOCTOR_RECORD_FILE), &backup)
        .ok()
        .map(|()| backup)
}

fn epoch_s() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------- 探针

/// 用应用自己的装载口径探一件 SFZ（与轨道装载同一条路径，裁决一致）。
fn probe(path: &Path) -> Result<(), String> {
    crate::sfz_load::load_sfz(path, None, 48_000.0)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// 从引擎报错里抠出「`op` value V out of range A..=B」的 (op, A, B)。
fn parse_range_error(message: &str) -> Option<(String, i64, i64)> {
    let open = message.find('`')?;
    let close = message[open + 1..].find('`')? + open + 1;
    let opcode = message[open + 1..close].to_string();
    let range_at = message.find(" out of range ")? + " out of range ".len();
    let range = &message[range_at..];
    let (lo_s, hi_s) = range.split_once("..=")?;
    let hi_s = hi_s
        .split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
        .next()?;
    let lo: f64 = lo_s.trim().parse().ok()?;
    let hi: f64 = hi_s.trim().parse().ok()?;
    Some((opcode, lo as i64, hi as i64))
}

// ---------------------------------------------------------------- 文件行走

fn walk_files(root: &Path, exts: &[&str]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path
                    .extension()
                    .map(|e| exts.iter().any(|x| e.eq_ignore_ascii_case(x)))
                    .unwrap_or(false)
                {
                    out.push(path);
                }
            }
        }
    }
    out.sort();
    out
}

/// 顶层 SFZ = 不在 `includes/` 组件目录里的 .sfz（那些是被包含的部件，
/// 不是独立乐器，与全量审计同一口径）。
fn top_level_sfzs(root: &Path) -> Vec<PathBuf> {
    walk_files(root, &["sfz"])
        .into_iter()
        .filter(|p| {
            let rel = p.strip_prefix(root).unwrap_or(p);
            !rel.components()
                .any(|c| c.as_os_str().eq_ignore_ascii_case("includes"))
        })
        .collect()
}

// ---------------------------------------------------------------- 钳值

/// 在一段文本里找 `opcode=`（前面不能是 `[a-z0-9_]`，防匹配到更长的名字），
/// 对每个值调用 `f`，`f` 返回 `Some(新值)` 表示要改。返回改后的文本。
fn rewrite_opcode_values(
    text: &str,
    opcode: &str,
    mut f: impl FnMut(&str) -> Option<String>,
) -> String {
    let needle = format!("{opcode}=");
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(&needle) {
        let absolute = text.len() - rest.len() + at;
        let prev_ok =
            absolute == 0 || !matches!(bytes[absolute - 1], b'a'..=b'z' | b'0'..=b'9' | b'_');
        if !prev_ok {
            out.push_str(&rest[..at + needle.len()]);
            rest = &rest[at + needle.len()..];
            continue;
        }
        let value_start = at + needle.len();
        let value_end = rest[value_start..]
            .find(|c: char| c.is_whitespace())
            .map(|i| value_start + i)
            .unwrap_or(rest.len());
        let raw = &rest[value_start..value_end];
        match f(raw) {
            Some(new_value) => {
                out.push_str(&rest[..value_start]);
                out.push_str(&new_value);
                rest = &rest[value_end..];
            }
            None => {
                out.push_str(&rest[..value_end]);
                rest = &rest[value_end..];
            }
        }
    }
    out.push_str(rest);
    out
}

fn clamp_int(raw: &str, limit: i64) -> Option<String> {
    let value: i64 = raw.parse().ok()?;
    if value.abs() > limit {
        Some(value.clamp(-limit, limit).to_string())
    } else {
        None
    }
}

/// 已知钳值一趟：fil_veltrack ±9600 / tune ±100 / amp_keycenter 音名→数字。
fn clamp_file_known(path: &Path, edits: &mut Vec<DoctorEdit>) {
    let Ok(text) = fs::read_to_string(path) else {
        return;
    };
    let mut out = rewrite_opcode_values(&text, "fil_veltrack", |raw| clamp_int(raw, 9600));
    out = rewrite_opcode_values(&out, "tune", |raw| clamp_int(raw, 100));
    out = rewrite_opcode_values(&out, "amp_keycenter", |raw| ipn(raw).map(|n| n.to_string()));
    if out != text {
        record_edits(path, &text, &out, edits);
        let _ = fs::write(path, &out);
    }
}

/// 引擎探针回路用：把某个操作码的全部越界值钳进 [lo, hi]。返回是否改了。
fn clamp_opcode_in_file(
    path: &Path,
    opcode: &str,
    lo: i64,
    hi: i64,
    edits: &mut Vec<DoctorEdit>,
) -> bool {
    let Ok(text) = fs::read_to_string(path) else {
        return false;
    };
    let out = rewrite_opcode_values(&text, opcode, |raw| {
        let value: i64 = raw.parse().ok()?;
        if value < lo || value > hi {
            Some(value.clamp(lo, hi).to_string())
        } else {
            None
        }
    });
    if out != text {
        record_edits(path, &text, &out, edits);
        let _ = fs::write(path, &out);
        return true;
    }
    false
}

/// 从前后文本里对出改了哪些操作码（记录用；逐个 needle 数差异即可——
/// 记录只需 (file, opcode, old, new)，从两版里按操作码各扫一遍配对）。
fn record_edits(path: &Path, before: &str, after: &str, edits: &mut Vec<DoctorEdit>) {
    for opcode in ["fil_veltrack", "tune", "amp_keycenter"] {
        let olds = opcode_values(before, opcode);
        let news = opcode_values(after, opcode);
        for (old, new) in olds.iter().zip(news.iter()) {
            if old != new {
                edits.push(DoctorEdit {
                    file: path.to_path_buf(),
                    opcode: opcode.to_string(),
                    old: old.clone(),
                    new: new.clone(),
                });
            }
        }
    }
}

fn opcode_values(text: &str, opcode: &str) -> Vec<String> {
    let needle = format!("{opcode}=");
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find(&needle) {
        let value_start = at + needle.len();
        let value_end = rest[value_start..]
            .find(|c: char| c.is_whitespace())
            .map(|i| value_start + i)
            .unwrap_or(rest.len());
        out.push(rest[value_start..value_end].to_string());
        rest = &rest[value_end..];
    }
    out
}

/// IPN 音名 → MIDI 号（C4 = 60）。
fn ipn(name: &str) -> Option<i64> {
    let lower = name.to_lowercase();
    let mut chars = lower.chars();
    let semi = match chars.next()? {
        'c' => 0,
        'd' => 2,
        'e' => 4,
        'f' => 5,
        'g' => 7,
        'a' => 9,
        'b' => 11,
        _ => return None,
    };
    let mut acc = 0i64;
    let mut rest: String = chars.collect();
    if rest.starts_with('#') {
        acc = 1;
        rest.remove(0);
    } else if rest.starts_with('b') {
        acc = -1;
        rest.remove(0);
    }
    let octave: i64 = rest.parse().ok()?;
    Some(semi + acc + (octave + 1) * 12)
}

// ---------------------------------------------------------------- 大小写通道

/// 把路径逐段解析：第一段「只差大小写」处返回 (链接应建的位置, 实际名字)。
fn first_diff_link(path: &Path) -> Option<(PathBuf, String)> {
    let mut cur = PathBuf::from("/");
    for comp in path.components() {
        let name = comp.as_os_str().to_string_lossy().into_owned();
        match name.as_str() {
            "/" | "" | "." => continue,
            ".." => {
                cur.pop();
                continue;
            }
            _ => {}
        }
        let candidate = cur.join(&name);
        if candidate.exists() {
            cur = candidate;
            continue;
        }
        let entries: Vec<String> = fs::read_dir(&cur)
            .ok()?
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        let matches: Vec<&String> = entries
            .iter()
            .filter(|e| e.eq_ignore_ascii_case(&name))
            .collect();
        if matches.len() != 1 {
            return None;
        }
        return Some((cur.join(&name), matches[0].clone()));
    }
    None
}

/// 词法归一 base/default_path/sample 拼接（处理 `.` / `..`）。
fn normalize_join(base: &Path, default_path: &str, sample: &str) -> PathBuf {
    let mut parts: Vec<String> = base
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();
    for seg in format!("{default_path}/{sample}").split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                if parts.len() > 1 {
                    parts.pop();
                }
            }
            s => parts.push(s.to_string()),
        }
    }
    let mut out = PathBuf::from("/");
    for part in parts.iter().skip(1) {
        out.push(part);
    }
    out
}

// ---------------------------------------------------------------- SFZ 解析（只为大小写通道取 sample/default_path）

struct Region {
    sample: Option<String>,
    default_path: String,
}

/// `#include` 展开（深度 ≤ 8、去重、跳过 `//` 注释行），与审计脚本同口径。
fn expand(path: &Path, depth: usize, seen: &mut HashSet<PathBuf>, out: &mut Vec<String>) {
    if depth > 8 {
        return;
    }
    let canon = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if !seen.insert(canon) {
        return;
    }
    let Ok(text) = fs::read_to_string(path) else {
        return;
    };
    let dir = path.parent().unwrap_or(Path::new("/")).to_path_buf();
    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("#include")
            && let Some(quoted) = rest.split('"').nth(1)
        {
            expand(&dir.join(quoted.replace('\\', "/")), depth + 1, seen, out);
            continue;
        }
        if !trimmed.starts_with("//") {
            out.push(line.to_string());
        }
    }
}

/// 空格宽容的操作码切分：值一直取到下一个 `name=` 之前，
/// 采样路径里的空格不会被截断（审计实测的老坑）。
fn ops_in(text: &str) -> Vec<(String, String)> {
    let bytes = text.as_bytes();
    let mut starts: Vec<(usize, usize, String)> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let is_name_char = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
        if is_name_char(bytes[i]) {
            let start = i;
            while i < bytes.len() && is_name_char(bytes[i]) {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'=' {
                starts.push((start, i + 1, text[start..i].to_lowercase()));
            }
        } else {
            i += 1;
        }
    }
    let mut out = Vec::new();
    for (idx, (_, value_start, name)) in starts.iter().enumerate() {
        let end = if idx + 1 < starts.len() {
            starts[idx + 1].0
        } else {
            text.len()
        };
        let value = text[*value_start..end].trim().trim_matches('"').to_string();
        out.push((name.clone(), value));
    }
    out
}

fn parse_regions(path: &Path) -> Vec<Region> {
    let mut lines = Vec::new();
    expand(path, 0, &mut HashSet::new(), &mut lines);
    let mut regions: Vec<Region> = Vec::new();
    let mut gstate: HashMap<String, String> = HashMap::new();
    let mut state: HashMap<String, String> = HashMap::new();
    let mut cur: Option<usize> = None;
    for line in &lines {
        // 按 <...> 头切分（头本身单独成 token）。
        let mut tokens: Vec<&str> = Vec::new();
        let mut last = 0;
        let bytes = line.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'<'
                && let Some(close) = line[i..].find('>')
            {
                if i > last {
                    tokens.push(&line[last..i]);
                }
                tokens.push(&line[i..=i + close]);
                i += close + 1;
                last = i;
                continue;
            }
            i += 1;
        }
        if last < line.len() {
            tokens.push(&line[last..]);
        }
        for token in tokens {
            let t = token.trim();
            if t.starts_with('<') {
                let kind = t.trim_matches(|c| c == '<' || c == '>').to_lowercase();
                match kind.as_str() {
                    "global" => {
                        cur = None;
                        gstate.clear();
                    }
                    "group" => {
                        cur = None;
                        state = gstate.clone();
                    }
                    "control" => {
                        cur = None;
                    }
                    "region" => {
                        regions.push(Region {
                            sample: state.get("sample").cloned(),
                            default_path: state.get("default_path").cloned().unwrap_or_default(),
                        });
                        cur = Some(regions.len() - 1);
                    }
                    _ => {}
                }
                continue;
            }
            for (key, value) in ops_in(token) {
                if let Some(ci) = cur {
                    if key == "sample" {
                        regions[ci].sample = Some(value);
                    } else if key == "default_path" {
                        regions[ci].default_path = value;
                    }
                } else if key == "default_path" {
                    state.insert(key.clone(), value.clone());
                    gstate.insert(key, value);
                } else {
                    state.insert(key, value);
                }
            }
        }
    }
    regions
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Language;

    /// 三场景共用的样本报告：3 件已查、2 条大小写链接、1 处操作码修正。
    fn sample_report() -> DoctorReport {
        DoctorReport {
            files_checked: 3,
            links_created: vec![PathBuf::from("a.sfz"), PathBuf::from("b.sfz")],
            edits: vec![DoctorEdit {
                file: PathBuf::from("a.sfz"),
                opcode: "tune".to_string(),
                old: "-109".to_string(),
                new: "-100".to_string(),
            }],
            still_failing: vec![],
            record_error: None,
        }
    }

    /// 仍有 1 件装不上 + 记录落盘失败的样本报告（引擎报错原文 ASCII）。
    fn failing_report() -> DoctorReport {
        let mut report = sample_report();
        report.still_failing = vec![(PathBuf::from("bad.sfz"), "engine error (test)".to_string())];
        report.record_error = Some("io error (test)".to_string());
        report
    }

    /// 中文态与旧实现逐字相同（三场景：全好 / 仍有装不上 / 记录落盘失败尾巴）。
    #[test]
    fn format_doctor_readout_zh_cn_is_byte_identical() {
        assert_eq!(
            format_doctor_readout_in("ext-lib", &sample_report(), Language::ZhCn),
            "SFZ Doctor：《ext-lib》检查完成 —— 3 件乐器全部可装载（建大小写链接 2、修正操作码 1）"
        );
        let failing = failing_report();
        assert_eq!(
            format_doctor_readout_in("ext-lib", &failing, Language::ZhCn),
            "SFZ Doctor：《ext-lib》已修复大小写链接 2、操作码 1；仍有 1 件装不上（engine error (test)）；检查完成，但记录未能保存，下次可能再次提示"
        );
        let mut record_failed = sample_report();
        record_failed.record_error = Some("io error (test)".to_string());
        assert_eq!(
            format_doctor_readout_in("ext-lib", &record_failed, Language::ZhCn),
            "SFZ Doctor：《ext-lib》检查完成 —— 3 件乐器全部可装载（建大小写链接 2、修正操作码 1）；检查完成，但记录未能保存，下次可能再次提示"
        );
    }

    /// 英文态三场景逐条钉死：两模板 + 尾巴，引擎报错原文原样进占位。
    #[test]
    fn format_doctor_readout_en_us_renders_english() {
        assert_eq!(
            format_doctor_readout_in("ext-lib", &sample_report(), Language::EnUs),
            "SFZ Doctor: “ext-lib” check complete — all instruments loadable (checked: 3, case links created: 2, opcodes fixed: 1)"
        );
        let failing = failing_report();
        assert_eq!(
            format_doctor_readout_in("ext-lib", &failing, Language::EnUs),
            "SFZ Doctor: “ext-lib” — case links fixed: 2, opcodes fixed: 1; 1 still failing to load (engine error (test)); check complete, but the record failed to save and you may be prompted again next time"
        );
    }

    /// 九语全覆盖：每语两场景均有读数；非中文语的模板不含中文模板残片
    /// （「检查完成」「全部可装载」「装不上」「记录未能保存」「下次可能再次提示」
    /// 「建大小写链接」「修正操作码」「已修复」）；库名与引擎报错原文
    /// （ASCII 记号）每语原样进占位。
    #[test]
    fn format_doctor_readout_all_languages_have_no_chinese_template_residue() {
        let failing = failing_report();
        for lang in Language::ALL {
            for report in [&sample_report(), &failing] {
                let rendered = format_doctor_readout_in("ext-lib", report, lang);
                assert!(
                    rendered.contains("ext-lib"),
                    "{lang:?} 渲染必须含库名，实际：{rendered}"
                );
                if !report.still_failing.is_empty() {
                    assert!(
                        rendered.contains("engine error (test)"),
                        "{lang:?} 仍失败场景渲染必须原样保留引擎报错原文，实际：{rendered}"
                    );
                }
                if lang != Language::ZhCn {
                    for fragment in [
                        "检查完成",
                        "全部可装载",
                        "装不上",
                        "记录未能保存",
                        "下次可能再次提示",
                        "建大小写链接",
                        "修正操作码",
                        "已修复",
                    ] {
                        assert!(
                            !rendered.contains(fragment),
                            "{lang:?} 模板不许含中文残片「{fragment}」，实际：{rendered}"
                        );
                    }
                }
            }
        }
    }
}
