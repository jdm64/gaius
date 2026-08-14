use std::process::Command;

fn main() {
    // Re-run if HEAD changes (new commit, checkout, etc.)
    if let Ok(git_dir) = Command::new("git")
        .arg("rev-parse")
        .arg("--git-dir")
        .output()
    {
        if git_dir.status.success() {
            let git_dir = String::from_utf8_lossy(&git_dir.stdout).trim().to_string();
            // .git/HEAD changes on every commit/checkout; refs changes on branch switches
            println!("cargo:rerun-if-changed={git_dir}/HEAD");
            println!("cargo:rerun-if-changed={git_dir}/refs");
        }
    }

    let describe = Command::new("git")
        .args(["describe", "--always"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".into());

    println!("cargo:rustc-env=GIT_VERSION={describe}");
}
