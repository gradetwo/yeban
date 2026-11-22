use std::path::Path;

fn resolve_git_head() -> Option<String> {
    let git_dir = Path::new("../../.git");
    let head_path = git_dir.join("HEAD");
    if head_path.exists() {
        println!("cargo:rerun-if-changed={}", head_path.display());
    }
    let head = std::fs::read_to_string(&head_path).ok()?;
    let trimmed = head.trim();
    if let Some(rel_ref) = trimmed.strip_prefix("ref:") {
        let rel_ref = rel_ref.trim();
        let ref_path = git_dir.join(rel_ref);
        if ref_path.exists() {
            println!("cargo:rerun-if-changed={}", ref_path.display());
            if let Ok(sha) = std::fs::read_to_string(&ref_path) {
                let sha = sha.trim();
                if !sha.is_empty() {
                    return Some(sha.to_owned());
                }
            }
        }
        return Some(trimmed.to_owned());
    }
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_owned())
    }
}

fn main() {
    let head = resolve_git_head().unwrap_or_else(|| "unknown".to_owned());
    println!("cargo:rustc-env=YEBAN_GIT_HEAD={head}");
}
