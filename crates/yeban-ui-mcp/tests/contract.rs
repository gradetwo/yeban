//! 跨 crate 的**承重**判据（集成测试）：契约、样本对账、manifest 安全开关、指纹对账。
//!
//! 与模块内的单元判据分工：
//!
//! | 侧 | 跑在哪 | 覆盖什么 |
//! | :--- | :--- | :--- |
//! | 模块内 `#[cfg(test)]` | 本机（`rustc --test`）+ CI | 管线 / 投影 / 遮罩 / 参数 / 安全矩阵 |
//! | 本文件 | **只有 CI**（它一引用 `yeban_ui_test_port::render` 就拖进 Slint） | manifest 开关、与 Tier-1 渲染器的指纹对账、跨语言样本对账 |
//!
//! 这样切分的理由：本机纪律禁止编译 Slint，而"零 Slint 的那一半"必须能在本机真跑；
//! 需要 Slint 的那几条就**只**放在这里，不做成"本机跑不动的单元判据"。

use std::path::PathBuf;

/// 每个判据独立的临时目录。
fn temp_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-ui-mcp-contract-{}-{tag}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// 判据: 本 crate 的 `Cargo.toml` **不得**把 `ui-mcp-http` / `mcp-http` 放进 `default`
/// [AGENTS.md §2 红线 6 / MUST-GATE-009]。
///
/// 这不是重复 `policy_check.py` 的 G05：G05 只看**声明**，而这条同时钉住
/// "dev-dependency 打开 `mcp-http`"这件事**没有**顺手污染默认特性集
/// （那是本 crate 为了让 http 模块在 CI 里被编译而做的取舍，见 `Cargo.toml` 的注释）。
#[test]
fn manifest_default_features_do_not_enable_ui_mcp_http() {
    let manifest =
        std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
            .expect("读本 crate 的 Cargo.toml");
    let features = manifest
        .split("[features]")
        .nth(1)
        .expect("有 [features] 段")
        .split("\n[")
        .next()
        .expect("截到下一个段");
    let default_line = features
        .lines()
        .find(|line| line.trim_start().starts_with("default"))
        .expect("有 default 特性");
    assert_eq!(
        default_line.trim(),
        "default = []",
        "default 必须是空集 (红线 6): {default_line}"
    );
    assert!(
        !default_line.contains("ui-mcp-http") && !default_line.contains("mcp-http"),
        "危险 feature 不得进 default: {default_line}"
    );

    // 两道开关的名字必须是常量里写的那两个（防止改名后文档与代码漂移）。
    assert!(manifest.contains("ui-mcp-http = [\"yeban-mcp/mcp-http\"]"));
    assert_eq!(
        yeban_ui_mcp::HTTP_FEATURE_NAME,
        "ui-mcp-http",
        "feature 名必须与常量一致"
    );
    assert_eq!(yeban_ui_mcp::ENABLE_HTTP_FLAG, "--enable-ui-mcp-http");
}

/// 判据: 本 crate 的 FNV-1a 指纹与 **Tier-1 渲染器** 的逐位相同。
///
/// 本 crate 为了保持零 Slint 而自带一份 [`yeban_ui_mcp::surface::fnv1a64`]
/// （`render.rs` 依赖 Slint，引用它会让整块逻辑无法在本机跑）。这条判据是
/// "两份实现不许漂移"的对账 —— 它**必须**在能编译 Slint 的地方跑。
#[test]
fn fingerprint_matches_the_tier1_renderer() {
    for bytes in [
        Vec::new(),
        b"yeban".to_vec(),
        (0..=255_u8).collect::<Vec<u8>>(),
        vec![0_u8; 4096],
    ] {
        assert_eq!(
            yeban_ui_mcp::surface::fnv1a64(&bytes),
            yeban_ui_test_port::render::fnv1a64(&bytes),
            "FNV-1a 实现漂移了 (长度 {})",
            bytes.len()
        );
    }
    // PNG 魔数的指纹是稳定的短标识（不是安全摘要，只是文件指纹）。
    assert_eq!(
        yeban_ui_mcp::surface::fnv1a64(&yeban_ui_test_port::png::PNG_SIGNATURE),
        yeban_ui_test_port::render::fnv1a64(&yeban_ui_test_port::png::PNG_SIGNATURE)
    );
}

/// 判据（**承重**）: 本线样本 + 领域 MCP 的真实例放进同一目录时，
/// `validate_schemas.py --samples-dir` 必须通过；混进违法实例必须变红。
///
/// 缺 `jsonschema` / `python3` 时**响亮 SKIP**（`eprintln!` 一行大写的 SKIP），
/// 绝不静默通过 —— 与 `yeban-mcp` 的 `contract_rejects_a_deliberately_invalid_sample` 同口径。
#[test]
fn samples_reconcile_with_the_domain_contract() {
    let dir = temp_dir("reconcile");
    let mine = yeban_ui_mcp::samples::export_all(&dir).expect("导出本线样本");
    assert_eq!(mine.len(), 3, "只导出 3 份 .meta. 文档样本");
    yeban_mcp::samples::export_all(&dir).expect("导出领域样本 (真实例)");

    let verdict = yeban_ui_mcp::samples::reconcile_with_schemas(&dir);
    match &verdict {
        yeban_ui_mcp::samples::Reconcile::Passed { detail } => {
            assert!(detail.contains("契约校验通过"), "{detail}");
        }
        yeban_ui_mcp::samples::Reconcile::Skipped { detail } => {
            eprintln!(
                "========================= LOUD SKIP =========================\n\
                 跨语言契约对账**没有跑** (缺 python3/jsonschema):\n{detail}\n\
                 ============================================================"
            );
        }
        yeban_ui_mcp::samples::Reconcile::Failed { detail } => {
            panic!("本线样本 + 领域真实例必须通过根 oneOf 对账:\n{detail}");
        }
    }

    // 负向对照：故意违法的实例必须让对账变红（证明判据承重）。
    if !matches!(verdict, yeban_ui_mcp::samples::Reconcile::Skipped { .. }) {
        let bogus = dir.join("mcp-tools.ui-bogus.json");
        std::fs::write(&bogus, "{\"anything\":[1,2,3]}\n").expect("写违法样本");
        assert!(
            matches!(
                yeban_ui_mcp::samples::reconcile_with_schemas(&dir),
                yeban_ui_mcp::samples::Reconcile::Failed { .. }
            ),
            "故意违法的样本必须让对账变红"
        );
        std::fs::remove_file(&bogus).ok();
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// 判据: 只导出本线样本会让 `mcp-tools` 前缀只剩 `.meta.` —— 脚本必须**响亮地**红。
///
/// 这条把"必须与 `export_mcp_samples` 一起对账"变成可执行的依赖，而不是口头约定。
#[test]
fn ui_samples_alone_are_meta_only_and_the_prefix_guard_says_so() {
    let dir = temp_dir("meta-only");
    yeban_ui_mcp::samples::export_all(&dir).expect("只导出本线样本");
    match yeban_ui_mcp::samples::reconcile_with_schemas(&dir) {
        yeban_ui_mcp::samples::Reconcile::Skipped { detail } => {
            eprintln!(
                "========================= LOUD SKIP =========================\n{detail}\n============================================================"
            );
        }
        yeban_ui_mcp::samples::Reconcile::Failed { detail } => {
            assert!(
                detail.contains("只有 .meta."),
                "红的原因必须是前缀守卫: {detail}"
            );
        }
        yeban_ui_mcp::samples::Reconcile::Passed { detail } => {
            panic!("只有 .meta. 时不该通过 (脚本的前缀守卫失效了):\n{detail}");
        }
    }
    std::fs::remove_dir_all(&dir).ok();
}

/// 判据: 端到端的 stdio 一帧 —— 真实二进制形状的入口（`handle_line`）在
/// 未鉴权时给 401、在生产模式下硬拒注入。
///
/// 这条与 `src/transport/stdio.rs` 的单元判据互补：它走的是**公开 API**，
/// 因此"集成者按文档接线"时不会遇到只在 crate 内部成立的行为。
#[test]
fn public_entry_point_behaves_like_the_documented_contract() {
    use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};
    use yeban_ui_mcp::{CAPTURE_FAILED, ELEMENT_NOT_FOUND, GEOMETRY_UNAVAILABLE, UiService};

    // 一个"什么都没有"的执行面：`ui/tree` 返回空树 —— 用来证明服务**不需要**
    // 任何真实窗口就能完成鉴权/授权判定。
    let surface = EmptySurface {
        tree: yeban_ui_test_port::tree::ControlTree::new(),
    };
    let token = BearerToken::generate().token;
    let mut ui = UiService::new(
        token.clone(),
        ScopeSet::all(),
        RunMode::Production,
        Box::new(surface),
    );

    // 未鉴权 ⇒ 401 且不解析。
    let outcome = ui.handle_line(Channel::Http, None, "{不是 JSON");
    assert_eq!(outcome.http_status, 401);

    // 生产模式 + 合法 token + 注入 ⇒ 403（硬禁在 token 之后仍然生效）。
    let header = format!("Bearer {}", token.expose());
    let outcome = ui.handle_line(
        Channel::Http,
        Some(&header),
        r#"{"jsonrpc":"2.0","id":1,"method":"ui/dispatch_pointer_up","params":{"button":"left"}}"#,
    );
    assert_eq!(outcome.http_status, 403);
    assert_eq!(
        outcome
            .response
            .expect("有响应")
            .error_object()
            .expect("有 error")
            .code,
        yeban_mcp::jsonrpc::FORBIDDEN
    );

    // 只读方法 ⇒ 200，空树也是合法结果。
    let outcome = ui.handle_line(
        Channel::Http,
        Some(&header),
        r#"{"jsonrpc":"2.0","id":2,"method":"ui/tree"}"#,
    );
    assert_eq!(outcome.http_status, 200);
    let result = outcome.response.expect("有响应").result.expect("有 result");
    assert_eq!(result["tree"]["count"], 0);
    assert_eq!(result["tree"]["source"], "runtime");

    // 新错误码对外可见且稳定（契约面）。
    assert_eq!(ELEMENT_NOT_FOUND, -32006);
    assert_eq!(CAPTURE_FAILED, -32008);
    assert_eq!(GEOMETRY_UNAVAILABLE, -32009);
}

/// 一个空执行面（**零 Slint**）：只用于走通公开入口的鉴权/授权判定。
struct EmptySurface {
    tree: yeban_ui_test_port::tree::ControlTree,
}

impl yeban_ui_mcp::UiSurface for EmptySurface {
    fn surface_name(&self) -> &'static str {
        "empty"
    }
    fn capture_image(
        &self,
    ) -> Result<yeban_ui_test_port::image::Rgb8Image, yeban_ui_test_port::port::PortError> {
        Err(yeban_ui_test_port::port::PortError::Rejected {
            message: "空执行面没有像素".to_owned(),
        })
    }
}

impl yeban_ui_test_port::port::UiTestPort for EmptySurface {
    fn permission(&self) -> yeban_ui_test_port::port::Permission {
        yeban_ui_test_port::port::Permission::ReadOnly
    }
    fn tree(&self) -> &yeban_ui_test_port::tree::ControlTree {
        &self.tree
    }
    fn capture_png(&self) -> Result<Vec<u8>, yeban_ui_test_port::port::PortError> {
        Err(yeban_ui_test_port::port::PortError::Rejected {
            message: "空执行面没有像素".to_owned(),
        })
    }
    fn read_property(
        &self,
        element_id: &str,
        _name: &str,
    ) -> Result<String, yeban_ui_test_port::port::PortError> {
        Err(yeban_ui_test_port::port::PortError::UnknownElement {
            id: element_id.to_owned(),
        })
    }
    fn dispatch_pointer_down_impl(
        &mut self,
        _element_id: &str,
        _x_offset: f64,
        _y_offset: f64,
        _button: yeban_ui_test_port::port::PointerButton,
    ) -> Result<(), yeban_ui_test_port::port::PortError> {
        Ok(())
    }
    fn dispatch_pointer_move_impl(
        &mut self,
        _x: f64,
        _y: f64,
    ) -> Result<(), yeban_ui_test_port::port::PortError> {
        Ok(())
    }
    fn dispatch_pointer_up_impl(
        &mut self,
        _button: yeban_ui_test_port::port::PointerButton,
    ) -> Result<(), yeban_ui_test_port::port::PortError> {
        Ok(())
    }
    fn dispatch_key_press_impl(
        &mut self,
        _key: yeban_ui_test_port::port::KeyCode,
    ) -> Result<(), yeban_ui_test_port::port::PortError> {
        Ok(())
    }
    fn switch_main_view_impl(
        &mut self,
        _view: &str,
    ) -> Result<(), yeban_ui_test_port::port::PortError> {
        Ok(())
    }
    fn force_save_impl(&mut self) -> Result<(), yeban_ui_test_port::port::PortError> {
        Ok(())
    }
    fn reload_engine_impl(&mut self) -> Result<(), yeban_ui_test_port::port::PortError> {
        Ok(())
    }
}
