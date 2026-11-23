//! T-FIX-158：远程 demo 下载核心的判据（本地夹具服务器，不依赖真 GitHub）。

use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;

use yeban_app::demo_download::{
    CancelFlag, DEMOS, DownloadError, DownloadEvent, cover_url, download_to, encode_segment,
    part_path, project_url,
};

/// 起一个只回答 `requests` 次的极简 HTTP 服务器；返回 (基址, 收到的请求路径列表句柄)。
fn serve(body: Vec<u8>, requests: usize) -> (String, Arc<std::sync::Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("绑定端口");
    let addr = listener.local_addr().expect("地址");
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen_thread = Arc::clone(&seen);
    std::thread::spawn(move || {
        for _ in 0..requests {
            let Ok((mut stream, _)) = listener.accept() else {
                break;
            };
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            let text = String::from_utf8_lossy(&buf).to_string();
            if let Some(path) = text.split_whitespace().nth(1) {
                seen_thread.lock().unwrap().push(path.to_owned());
            }
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = stream.write_all(header.as_bytes());
            let _ = stream.write_all(&body);
            let _ = stream.flush();
        }
    });
    (format!("http://{addr}"), seen)
}

fn temp_root(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "yeban-demo-dl-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).expect("建临时目录");
    dir
}

#[test]
fn manifest_lists_eight_unique_demos() {
    assert_eq!(DEMOS.len(), 8);
    let mut dirs: Vec<&str> = DEMOS.iter().map(|spec| spec.dir).collect();
    dirs.sort_unstable();
    dirs.dedup();
    assert_eq!(dirs.len(), 8, "子目录必须两两不同");
    let base = "http://example.invalid/base";
    // ⭐ T-FIX-158 返工：清单必须与 gradetwo/yeban-extras 的 `demos/` **逐字**对应
    //    （目录名就是歌名；工程/封面的文件名带 `_男声` / `_叙事版` 等后缀），
    //    且中文段一律百分号编码 —— http 1.5 的 URI 不接受 ≥127 的字节。
    let spec = DEMOS
        .iter()
        .find(|spec| spec.title == "雨声太吵")
        .expect("清单必须有《雨声太吵》");
    assert_eq!(
        project_url(base, spec),
        "http://example.invalid/base/%E9%9B%A8%E5%A3%B0%E5%A4%AA%E5%90%B5/%E9%9B%A8%E5%A3%B0%E5%A4%AA%E5%90%B5.yeban"
    );
    assert_eq!(
        cover_url(base, spec),
        "http://example.invalid/base/%E9%9B%A8%E5%A3%B0%E5%A4%AA%E5%90%B5/%E9%9B%A8%E5%A3%B0%E5%A4%AA%E5%90%B5-cover.png"
    );
    // 每条 URL 都必须是纯 ASCII（否则 ureq 在发请求前就解析失败），工程名都以 `.yeban` 结尾。
    for spec in &DEMOS {
        let url = project_url(base, spec);
        assert!(url.is_ascii(), "URL 必须纯 ASCII: {url}");
        assert!(
            spec.file.ends_with(".yeban"),
            "工程名必须以 .yeban 结尾: {}",
            spec.file
        );
        assert!(
            spec.cover.ends_with("-cover.png") || spec.cover.ends_with("-cover.jpg"),
            "封面名必须是真实后缀: {}",
            spec.cover
        );
    }
}

#[test]
fn download_reports_monotonic_progress_and_lands_the_file() {
    let body = b"PK\x03\x04fake-container-bytes".to_vec();
    let (base, seen) = serve(body.clone(), 1);
    let root = temp_root("ok");
    let target = root.join("demo").join("a.yeban");
    let url = format!("{base}/demo/a.yeban");

    let mut events = Vec::new();
    let path = download_to(&url, &target, &CancelFlag::new(), |event| {
        events.push(event)
    })
    .expect("下载必须成功");
    assert_eq!(path, target);
    assert!(target.is_file(), "正式文件必须落位");
    assert!(!part_path(&target).exists(), "半成品必须已改名，不留 .part");
    assert_eq!(std::fs::read(&target).unwrap(), body);
    assert_eq!(seen.lock().unwrap().as_slice(), ["/demo/a.yeban"]);

    let progresses: Vec<(u64, Option<u64>)> = events
        .iter()
        .filter_map(|event| match event {
            DownloadEvent::Progress { received, total } => Some((*received, *total)),
            _ => None,
        })
        .collect();
    assert!(!progresses.is_empty(), "必须发出进度事件");
    assert!(
        progresses.windows(2).all(|pair| pair[0].0 <= pair[1].0),
        "进度必须单调不减: {progresses:?}"
    );
    assert_eq!(progresses.last().unwrap().0, body.len() as u64);
    assert_eq!(progresses.last().unwrap().1, Some(body.len() as u64));
    assert!(matches!(events.last(), Some(DownloadEvent::Finished(_))));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn corrupt_payload_is_rejected_and_leaves_no_file() {
    let (base, _seen) = serve(b"not a container".to_vec(), 1);
    let root = temp_root("corrupt");
    let target = root.join("demo").join("bad.yeban");
    let url = format!("{base}/demo/bad.yeban");
    let error = download_to(&url, &target, &CancelFlag::new(), |_| {}).expect_err("非容器必须被拒");
    assert_eq!(error.0, DownloadError::Corrupt);
    assert!(!target.exists(), "校验失败不得留下正式文件");
    assert!(!part_path(&target).exists(), "校验失败不得留下半成品");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn cancelled_download_cleans_up_partial_file() {
    let (base, _seen) = serve(b"PK\x03\x04payload".to_vec(), 1);
    let root = temp_root("cancel");
    let target = root.join("demo").join("c.yeban");
    let url = format!("{base}/demo/c.yeban");
    let cancel = CancelFlag::new();
    cancel.cancel();
    let error = download_to(&url, &target, &cancel, |_| {}).expect_err("取消必须失败");
    assert_eq!(error.0, DownloadError::Cancelled);
    assert!(!target.exists());
    assert!(!part_path(&target).exists());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn network_failure_is_loud_and_retry_succeeds() {
    // 先指向一个没人监听的端口 ⇒ Network。
    let root = temp_root("net");
    let target = root.join("demo").join("d.yeban");
    let dead = "http://127.0.0.1:9/demo/d.yeban";
    let error = download_to(dead, &target, &CancelFlag::new(), |_| {}).expect_err("必须失败");
    assert_eq!(error.0, DownloadError::Network);
    assert!(!target.exists());

    // 再指向夹具 ⇒ 重试成功（同一个 target）。
    let (base, _seen) = serve(b"PK\x03\x04retry-ok".to_vec(), 1);
    let url = format!("{base}/demo/d.yeban");
    download_to(&url, &target, &CancelFlag::new(), |_| {}).expect("重试必须成功");
    assert!(target.is_file());
    let _ = std::fs::remove_dir_all(&root);
}

/// ⭐ T-FIX-158 返工：**中文目录名的真实 demo** 必须在上线路径里百分号编码。
///
/// 修前清单是手编的 ASCII 目录（`rain-too-loud`），真实 extras 是中文目录 + 带后缀
/// 文件名；且中文不编码时 `http` crate 在解析 URI 阶段就拒绝（≥127 字节），
/// 一个字节都发不出去。本判据从夹具服务器**收到的请求行**读回线上路径。
#[test]
fn chinese_demo_paths_are_encoded_on_the_wire_and_download() {
    let body = b"PK\x03\x04chinese-demo".to_vec();
    let (base, seen) = serve(body.clone(), 1);
    let root = temp_root("cn");
    let spec = DEMOS
        .iter()
        .find(|spec| spec.title == "回头路")
        .expect("清单必须有《回头路》");
    let url = project_url(&base, spec);
    let target = root.join("demo").join(spec.file);
    let path = download_to(&url, &target, &CancelFlag::new(), |_| {})
        .expect("中文路径必须能下载（URI 已编码）");
    assert_eq!(path, target);
    assert_eq!(std::fs::read(&target).expect("读回"), body);
    let seen = seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1, "必须恰好一次请求: {seen:?}");
    assert_eq!(
        seen[0],
        format!(
            "/{}/{}",
            encode_segment(spec.dir),
            encode_segment(spec.file)
        ),
        "线上路径必须是编码后的目录/文件名"
    );
    assert!(
        !seen[0].contains('回'),
        "线上路径不得含未编码中文: {seen:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// ⭐ T-FIX-158 返工：中文路径的下载被取消后**不留任何残留**（正式文件与 `.part` 都没有）。
#[test]
fn cancelled_chinese_path_download_leaves_no_residue() {
    let root = temp_root("cn-cancel");
    let spec = DEMOS
        .iter()
        .find(|spec| spec.title == "回头路")
        .expect("清单必须有《回头路》");
    // 用活着的夹具：`download_to` 在读到第一个块时才看取消位（与既有取消判据同一条路径）。
    let (base, _seen) = serve(b"PK\x03\x04cn-cancel".to_vec(), 1);
    let url = project_url(&base, spec);
    let target = root.join("demo").join(spec.file);
    let cancel = CancelFlag::new();
    cancel.cancel();
    let error = download_to(&url, &target, &cancel, |_| {}).expect_err("取消必须失败");
    assert_eq!(error.0, DownloadError::Cancelled);
    assert!(!target.exists(), "取消不得留下正式文件");
    assert!(!part_path(&target).exists(), "取消不得留下 .part");
    let leftovers: Vec<PathBuf> = std::fs::read_dir(root.join("demo"))
        .map(|entries| entries.map(|entry| entry.expect("目录项").path()).collect())
        .unwrap_or_default();
    assert!(leftovers.is_empty(), "取消后目录必须为空: {leftovers:?}");
    let _ = std::fs::remove_dir_all(&root);
}
