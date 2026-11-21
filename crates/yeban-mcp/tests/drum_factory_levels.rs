//! **T-FIX-42 回归**：电子族出厂电平必须让默认渲染留在 0 dBFS 以下。
//!
//! 这条测试走的是**真实工具链**：`yeban_open_project`（create）→
//! `yeban_generate_drum_pattern`（autoMerge）→ `yeban_render_master`（默认
//! normalize=false）。它钉的是两条可观测读数：
//!
//! 1. `data.truePeakDbfs <= -1.0`（任务要求“真峰值不高于 −1 dBFS”）；
//! 2. `data.clippedSamples == 0`（24-bit 量化器不再被钳位）。
//!
//! 只测电子族 8 风格；原声族的电平不在本任务口径内，保持旧值。

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

use yeban_mcp::dispatch::Dispatcher;
use yeban_mcp::security::{BearerToken, Channel, RunMode, ScopeSet};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |delta| delta.as_nanos());
        let dir = std::env::temp_dir().join(format!(
            "yeban-mcp-fix42-{}-{tag}-{nanos}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&dir).expect("建临时目录");
        Self { dir }
    }

    fn join(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn dispatcher() -> (Dispatcher, String) {
    let token = BearerToken::generate().token;
    let auth = format!("Bearer {}", token.expose());
    let mut dispatcher = Dispatcher::new(token, ScopeSet::all(), RunMode::Production);
    dispatcher.domain_mut().set_now_ms(1_760_000_000_000);
    (dispatcher, auth)
}

fn call(dispatcher: &mut Dispatcher, auth: &str, name: &str, arguments: Value) -> Value {
    let line = json!({
        "jsonrpc": "2.0",
        "id": "t",
        "method": "tools/call",
        "params": {"name": name, "arguments": arguments}
    })
    .to_string();
    let outcome = dispatcher.handle_line(Channel::Http, Some(auth), &line);
    let response = outcome.response.expect("必须有响应");
    assert_eq!(outcome.http_status, 200, "{name} 必须是带内结果");
    response
        .result
        .unwrap_or_else(|| panic!("{name} 不该有 JSON-RPC 层错误: {:?}", response.error))
}

#[test]
fn electronic_drum_factory_levels_render_below_minus_one_dbfs() {
    let scratch = Scratch::new("levels");
    let (mut dispatcher, auth) = dispatcher();
    let styles = [
        "trap", "drill", "phonk", "lofi", "house", "techno", "edm", "hiphop",
    ];
    for (index, style) in styles.iter().enumerate() {
        let project = scratch.join(&format!("{style}.yeban"));
        let output = scratch.join(&format!("{style}.wav"));
        let open = call(
            &mut dispatcher,
            &auth,
            "yeban_open_project",
            json!({"path": project.display().to_string(), "create": true, "bpm": 120}),
        );
        assert_eq!(open["status"], "success", "{style} 建工程: {open}");
        let generated = call(
            &mut dispatcher,
            &auth,
            "yeban_generate_drum_pattern",
            json!({
                "style": style,
                "autoMerge": true,
                "bars": 4,
                "complexity": 50,
                "intensity": 80,
                "seed": 0,
                "fill": true
            }),
        );
        assert_eq!(generated["status"], "success", "{style} 生成: {generated}");
        let rendered = call(
            &mut dispatcher,
            &auth,
            "yeban_render_master",
            json!({
                "path": output.display().to_string(),
                "format": "wav",
                "sampleRate": 48000
            }),
        );
        assert_eq!(
            rendered["status"], "success",
            "{style} 默认渲染必须成功: {rendered}"
        );
        let true_peak = rendered["data"]["truePeakDbfs"]
            .as_f64()
            .unwrap_or_else(|| panic!("{style} 缺 truePeakDbfs: {rendered}"));
        let clipped = rendered["data"]["clippedSamples"]
            .as_u64()
            .unwrap_or_else(|| panic!("{style} 缺 clippedSamples: {rendered}"));
        assert!(
            true_peak <= -1.0,
            "{style} 真峰值 {true_peak} dBFS 高于 -1 dBFS（第 {index} 个风格）"
        );
        assert_eq!(clipped, 0, "{style} 仍有 {clipped} 个削波样本");
        let closed = call(&mut dispatcher, &auth, "yeban_close_project", json!({}));
        assert_eq!(closed["status"], "success", "{style} 关工程: {closed}");
    }
}
