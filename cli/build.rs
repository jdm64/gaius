use std::process::Command;

fn main() {
    // Re-run if HEAD changes (new commit, checkout, etc.)
    if let Ok(git_dir) = Command::new("git")
        .arg("rev-parse")
        .arg("--git-dir")
        .output()
        && git_dir.status.success()
    {
        let git_dir = String::from_utf8_lossy(&git_dir.stdout).trim().to_string();
        // .git/HEAD changes on every commit/checkout; refs changes on branch switches
        println!("cargo:rerun-if-changed={git_dir}/HEAD");
        println!("cargo:rerun-if-changed={git_dir}/refs");
    }

    // Re-run if Cargo.toml changes
    println!("cargo:rerun-if-changed=Cargo.toml");

    // Extract version from Cargo.toml
    let cargo_toml = std::fs::read_to_string("Cargo.toml").expect("Failed to read Cargo.toml");
    let version = cargo_toml
        .lines()
        .find(|line| line.trim_start().starts_with("version"))
        .and_then(|line| line.split_once('='))
        .map(|(_, val)| val.trim().trim_matches('"'))
        .unwrap_or("0.0.0")
        .to_string();

    // Get git commit hash
    let git_hash = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".into());

    // Combine version and git hash as <version>-<git commit>
    let git_version = format!("v{version}-{git_hash}");

    println!("cargo:rustc-env=GIT_VERSION={git_version}");
}
