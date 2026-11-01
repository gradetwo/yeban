//! 采样器路径的**存入归一化**（纯函数，不读环境、不查存在性、不碰磁盘）。
//!
//! 规则（Wang 2026-11-01 定）：采样器路径落在库根下时，**自动存相对形式**。
//! 起因是采样版工程把绝对路径（`/workspace/yeban/assets/samples/vsco2ce/*.sfz`）
//! 存进 `.yeban`，换机器打开就因 `sfz_load` 规则 1（绝对路径原样返回）硬错。
//! 解析规则本身不动 —— 本函数只改**新存入**的值：
//!
//! - 输入有根（`has_root`：Unix 绝对路径；Windows 下带盘符的绝对路径与
//!   `/x/…` 这类驱动器相对写法都有根 —— 后者 `is_absolute()` 为假、
//!   故判据不用 `is_relative()`，否则 Windows 下 Unix 拼写的测试路径会被
//!   误判为相对输入而原样返回），且按组件逐级落在 `library_root` 之下、
//!   余量非空 ⇒ 存 `assets/samples/` ＋ 库根下相对部分（分隔符统一写 `/`）；
//! - 输入已是相对形式 ⇒ 原样返回；
//! - 输入恰等于库根本身（余量为空）⇒ 原样返回；
//! - 输入是库根外的绝对路径 ⇒ 原样返回；
//! - 余量含 `..` ⇒ 原样返回（`strip_prefix` 是纯词法比较，`..` 会让存入的
//!   相对形式在库根换地方后指到别的文件 —— 相对化的可移植承诺就破了；
//!   且 `assets/samples/../…` 是走出库的形态，⛔ 不许存成库内相对形式）；
//! - 库根为空 ⇒ 原样返回（空路径的组件前缀恒成立，⛔ 不能让它误改写）。
//!
//! 存入的相对形式经 `sfz_load::resolve_sampler_path` 规则 2（库根、剥
//! `assets/samples/` 前缀）解析回原文件 —— 该往返由 app 侧 `sfz_load` 的
//! 内联判据与 MCP 侧 `sampler_path_roots` 的渲染判据钉住。
//!
//! ⚠ 本函数**不知道**库根是多少：库根由调用方给（app 侧
//! `samples::library_root()`，MCP 侧 `sample_roots::library_root()`，两者是
//! 既有的同口径镜像）。⛔ 不把 env／平台逻辑抄成第三份；⛔ 不迁移已存的
//! 绝对路径（打开／加载时不改写）；⛔ 不碰展示侧（`track_sampler_paths`
//! 只读存入值）。

use std::path::{Component, Path};

/// 采样器路径存入 `TrackV3::sampler` 之前的归一化。
///
/// 只作用于**新存入**的值：调用方在构造 `Op::SetTrackSampler` 之前调它；
/// `old_sampler`／陈旧载荷校验读文档原样，⛔ 不经过本函数。
#[must_use]
pub fn normalize_sampler_path_for_storage(library_root: &Path, path: &str) -> String {
    let raw = Path::new(path);
    // 有根才进相对化：Windows 下 `/x/…` 是驱动器相对写法（`is_absolute()` 为
    // 假），但它有根、且与同样写法的库根逐组件可比 —— 用 `has_root()` 判，
    // 真相对输入（无根）仍原样返回。
    if !raw.has_root() {
        return path.to_owned();
    }
    if library_root.as_os_str().is_empty() {
        return path.to_owned();
    }
    let Ok(relative) = raw.strip_prefix(library_root) else {
        return path.to_owned();
    };
    if relative.as_os_str().is_empty() {
        return path.to_owned();
    }
    // `strip_prefix` 是纯词法比较：余量里的 `..` 会让存入形式在库根换地方后
    // 指到别的文件（可移植承诺破了），⛔ 一律原样。
    if relative
        .components()
        .any(|component| component == Component::ParentDir)
    {
        return path.to_owned();
    }
    // 输入是 `&str` ⇒ 各组件恒为合法 UTF-8，`to_string_lossy` 在此无损。
    let mut stored = String::from("assets/samples");
    for component in relative.components() {
        stored.push('/');
        stored.push_str(&component.as_os_str().to_string_lossy());
    }
    stored
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn under_root_absolute_path_is_stored_relative() {
        assert_eq!(
            normalize_sampler_path_for_storage(
                Path::new("/x/samples"),
                "/x/samples/vsco2ce/Harp_slim.sfz",
            ),
            "assets/samples/vsco2ce/Harp_slim.sfz",
        );
    }

    #[test]
    fn outside_root_absolute_path_is_verbatim() {
        assert_eq!(
            normalize_sampler_path_for_storage(
                Path::new("/x/samples"),
                "/elsewhere/instruments/Harp.sfz",
            ),
            "/elsewhere/instruments/Harp.sfz",
        );
    }

    #[test]
    fn sibling_prefix_does_not_match() {
        // 组件逐级比较：`/x/samples2/...` 不是 `/x/samples` 之下（⛔ 不是字符串前缀）。
        assert_eq!(
            normalize_sampler_path_for_storage(
                Path::new("/x/samples"),
                "/x/samples2/vsco2ce/Harp.sfz",
            ),
            "/x/samples2/vsco2ce/Harp.sfz",
        );
    }

    #[test]
    fn already_relative_input_is_untouched() {
        assert_eq!(
            normalize_sampler_path_for_storage(
                Path::new("/x/samples"),
                "assets/samples/vsco2ce/Harp_slim.sfz",
            ),
            "assets/samples/vsco2ce/Harp_slim.sfz",
        );
        assert_eq!(
            normalize_sampler_path_for_storage(Path::new("/x/samples"), "tone.sfz"),
            "tone.sfz",
        );
    }

    #[test]
    fn path_equal_to_the_root_itself_is_verbatim() {
        assert_eq!(
            normalize_sampler_path_for_storage(Path::new("/x/samples"), "/x/samples"),
            "/x/samples",
        );
    }

    #[test]
    fn empty_library_root_is_verbatim() {
        assert_eq!(
            normalize_sampler_path_for_storage(Path::new(""), "/x/samples/Harp.sfz"),
            "/x/samples/Harp.sfz",
        );
    }

    #[test]
    fn dotdot_component_does_not_fake_membership() {
        // `strip_prefix` 是纯词法比较（`/x/samples/../…` 也能剥出 `../…` 余量），
        // 但余量含 `..` 时存入的相对形式在库根换地方后会指到别的文件 ⇒ 原样。
        assert_eq!(
            normalize_sampler_path_for_storage(
                Path::new("/x/samples"),
                "/x/samples/../samples/Harp.sfz",
            ),
            "/x/samples/../samples/Harp.sfz",
        );
        assert_eq!(
            normalize_sampler_path_for_storage(
                Path::new("/x/samples"),
                "/x/samples/../../etc/secret.sfz",
            ),
            "/x/samples/../../etc/secret.sfz",
        );
    }

    #[test]
    fn trailing_slash_on_the_root_changes_nothing() {
        assert_eq!(
            normalize_sampler_path_for_storage(
                Path::new("/x/samples/"),
                "/x/samples/vsco2ce/Harp_slim.sfz",
            ),
            "assets/samples/vsco2ce/Harp_slim.sfz",
        );
    }
}
