//! `--export-midi` 的**消费侧**：映射与编码已下移到 `yeban-midi`（与 MCP 工具**共用同一实现**）。

#![allow(unused_imports)] // 宽集合：缺失由编译器点名，多余由本行放行（账本第 339-341 轮）
use crate::save::{SaveError, write_file_atomically};
pub use yeban_midi::export::{MidiExportReport, export_from_project};

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use yeban_midi::midi::{
    DEFAULT_PPQ, MidiError, MidiExport, MidiExportTrack, MidiFormat, MidiTempo,
};
use yeban_model::music::MidiNote;
use yeban_model::project::{ClipContent, ClipPlacement, YebanProjectV1};
use yeban_model::{EntityId, PPQ};

/// 导出失败的原因。
#[derive(Debug)]
pub enum MidiExportError {
    /// 映射/编码失败（领域侧，来自共享 crate）。
    Export(yeban_midi::export::MidiExportError),
    /// 编码器拒绝。
    Encode(MidiError),
    /// 原子落盘失败（I/O）。
    Save(crate::save::SaveError),
}

impl core::fmt::Display for MidiExportError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Export(e) => write!(f, "{e}"),
            Self::Encode(e) => write!(f, "{e}"),
            Self::Save(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for MidiExportError {}

impl From<yeban_midi::export::MidiExportError> for MidiExportError {
    fn from(error: yeban_midi::export::MidiExportError) -> Self {
        Self::Export(error)
    }
}

/// 把工程写成 `.mid` 文件（**语义与下移前完全一致**）。
pub fn export_project_to_file(
    project: &YebanProjectV1,
    path: impl AsRef<Path>,
) -> Result<MidiExportReport, MidiExportError> {
    let export = export_from_project(project)?;
    let bytes = export.to_smf_bytes().map_err(MidiExportError::Encode)?;
    let saved = crate::save::write_file_atomically(&bytes, path).map_err(MidiExportError::Save)?;
    // T-FIX-505：通道复用必须与导出通道分配同源（同一份 export.tracks）。
    let (channels_reused, max_tracks_per_channel) =
        yeban_midi::export::channel_reuse_report(&export.tracks);
    Ok(MidiExportReport {
        path: saved.path,
        bytes: saved.bytes,
        temp_name: saved.temp_name,
        // T-FIX-691：`write_file_atomically` 的持久化警告必须进回执（此前只取了
        // path/bytes/temp_name，警告被静默丢弃，刷目录失败时导出照样报成功）。
        durability_warning: saved.durability.warning().map(str::to_owned),
        tracks: export.tracks.len(),
        notes: export.tracks.iter().map(|t| t.notes.len()).sum(),
        tempos: export.tempos.len(),
        ppq: export.ppq,
        format: export.format,
        channels_reused,
        max_tracks_per_channel,
    })
}

#[cfg(test)]
mod tests {
    #![allow(dead_code)] // 测试夹具：只被已下移的领域测试使用（账本第 341 轮）
    use super::*;

    const DEMO_NOTES: [(u64, u8); 6] = [
        (0, 60),
        (480, 64),
        (960, 67),
        (1440, 72),
        (1920, 74),
        (2400, 76),
    ];
    /// 演示夹具的音符时值（`MidiNote::new(.., 480)`）。
    const DEMO_DURATION: u64 = 480;
    /// 演示夹具的力度（`yeban_model::music::DEFAULT_VELOCITY`）。
    const DEMO_VELOCITY: u8 = 100;

    fn scratch_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "yeban-export-midi-{tag}-{}-{}",
            std::process::id(),
            EntityId::new().to_canonical_string()
        ));
        std::fs::create_dir_all(&dir).expect("建临时目录");
        dir
    }

    #[test]
    fn a_failed_write_leaves_no_half_file() {
        let dir = scratch_dir("atomic");
        let before = std::fs::read_dir(&dir).expect("列目录").count();
        let blocked = dir.join("missing").join("out.mid");

        let error = export_project_to_file(&crate::bridge::demo_project(), &blocked)
            .expect_err("父目录不存在 ⇒ 必须失败");
        assert!(
            error.to_string().contains("写临时文件"),
            "必须是精确的 I/O 原因: {error}"
        );
        assert!(!blocked.exists(), "失败不得留下目标文件");
        assert_eq!(
            std::fs::read_dir(&dir).expect("列目录").count(),
            before,
            "失败不得留下临时文件"
        );

        // 成功路径: 文件真的在, 且没有 `.tmp-` 残留。
        let target = dir.join("ok.mid");
        let report =
            export_project_to_file(&crate::bridge::demo_project(), &target).expect("导出必须成功");
        assert_eq!(
            report.bytes,
            std::fs::metadata(&target).expect("文件在").len() as usize
        );
        assert_eq!(report.ppq, 960);
        assert_eq!(report.tracks, 1);
        assert_eq!(report.notes, DEMO_NOTES.len());
        assert_eq!(report.format, MidiFormat::Parallel);
        assert!(report.temp_name.contains(crate::save::TEMP_INFIX));
        let leftovers: Vec<String> = std::fs::read_dir(&dir)
            .expect("列目录")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(crate::save::TEMP_INFIX))
            .collect();
        assert!(leftovers.is_empty(), "残留临时文件: {leftovers:?}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // ⚠ 与 `save.rs:596` / `cli.rs:3568` 同一条判据**同款**，那两处一直带 `#[cfg(unix)]`，
    // 本处漏了 ⇒ 它的 `use std::os::unix::…` 与 `set_mode` 在 **Windows 上编译不过**
    // （2026-10-12 实测：`windows` 腿的 clippy 步红在 `export_midi.rs:138/146/169`，
    // E0433 + E0599 ×2）。该判据的构造本身依赖 Unix 权限位，Windows 上无对应物 ⇒
    // 按仓库既有写法整条 `#[cfg(unix)]`（**不是**把它改写成永不失败的形状）。
    #[cfg(unix)]
    #[test]
    fn a_read_only_directory_never_touches_the_existing_file() {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = scratch_dir("readonly");
        let target = dir.join("keep.mid");
        export_project_to_file(&crate::bridge::demo_project(), &target).expect("先写一份");
        let before = std::fs::read(&target).expect("读原文");

        let mut permissions = std::fs::metadata(&dir).expect("元数据").permissions();
        permissions.set_mode(0o555);
        std::fs::set_permissions(&dir, permissions).expect("降权");
        let probe = dir.join(".probe");
        let writable = std::fs::write(&probe, b"x").is_ok();
        let _ = std::fs::remove_file(&probe);

        if writable {
            eprintln!("[export_midi] 只读目录仍可写 (特权进程?) —— 本判据无从判定, 响亮跳过");
        } else {
            let error = export_project_to_file(&crate::bridge::demo_project(), &target)
                .expect_err("只读目录必须失败");
            assert!(
                matches!(error, MidiExportError::Save(_)),
                "必须是落盘失败: {error}"
            );
            assert_eq!(
                std::fs::read(&target).expect("旧文件必须还在"),
                before,
                "失败的导出绝不能破坏已有文件"
            );
        }

        let mut permissions = std::fs::metadata(&dir).expect("元数据").permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&dir, permissions).expect("还原权限");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 判据（T-FIX-691）：rename 成功但**目录项刷盘失败** ⇒ 导出仍成功，但回执必须带
    /// 持久化警告（修前这里是 `None`：`saved.durability` 被静默丢弃，导出照样报成功）。
    #[cfg(unix)]
    #[test]
    fn a_failed_directory_sync_is_carried_in_the_receipt() {
        let dir = scratch_dir("dirsync");
        let target = dir.join("warned.mid");

        crate::save::DIR_SYNC_INJECTION.with(|cell| cell.set(true));
        let report = export_project_to_file(&crate::bridge::demo_project(), &target);
        crate::save::DIR_SYNC_INJECTION.with(|cell| cell.set(false));

        let report = report.expect("rename 已成功 ⇒ 导出已提交，不是失败");
        let warning = report
            .durability_warning
            .as_deref()
            .expect("刷目录失败时回执必须带持久化警告（⛔ 不许静默丢弃 saved.durability）");
        assert!(
            warning.contains("目录项未确认持久化"),
            "警告必须说清'文件已替换、目录项未确认': {warning}"
        );
        // 文件内容完整在位（警告不是失败，不回滚）。
        assert_eq!(
            report.bytes,
            std::fs::metadata(&target).expect("文件在").len() as usize
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 上条判据的反面（T-FIX-691）：注入关掉时同一导出必须仍是干净回执（无警告）。
    #[cfg(unix)]
    #[test]
    fn a_normal_export_carries_no_durability_warning() {
        let dir = scratch_dir("durable");
        let target = dir.join("quiet.mid");
        let report =
            export_project_to_file(&crate::bridge::demo_project(), &target).expect("导出必须成功");
        assert!(
            report.durability_warning.is_none(),
            "健康路径回执不得凭空多出警告: {:?}",
            report.durability_warning
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
