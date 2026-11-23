//! T-FIX-158：远程 demo 下载核心的判据（本地夹具服务器，不依赖真 GitHub）。

use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;

use yeban_app::demo_download::{
    CancelFlag, DEMOS, DownloadError, DownloadEvent, cover_url, download_to, part_path, project_url,
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
    assert_eq!(
        project_url(base, &DEMOS[0]),
        "http://example.invalid/base/rain-too-loud/雨声太吵.yeban"
    );
    assert_eq!(
        cover_url(base, &DEMOS[0]),
        "http://example.invalid/base/rain-too-loud/雨声太吵.png"
    );
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
