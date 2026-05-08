use std::process::Command;

fn main() {
    let git_hash = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());

    let git_dirty = Command::new("git")
        .args(["diff", "--quiet", "HEAD"])
        .status()
        .map(|s| !s.success())
        .unwrap_or(false);

    let hash_str = match git_hash {
        Some(hash) if git_dirty => format!("{hash}-dirty"),
        Some(hash) => hash,
        None => format!("v{}", std::env::var("CARGO_PKG_VERSION").unwrap()),
    };

    println!("cargo::rustc-env=AGENTSIGHT_GIT_HASH={hash_str}");

    let build_date = Command::new("date")
        .args(["-u", "+%Y-%m-%d"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    println!("cargo::rustc-env=AGENTSIGHT_BUILD_DATE={build_date}");
    println!("cargo::rerun-if-changed=.git/HEAD");
    println!("cargo::rerun-if-changed=.git/refs/");
}
