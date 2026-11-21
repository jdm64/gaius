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

    // Describe HEAD relative to the nearest tag. With --long, git emits
    // <tag>-<distance>-g<hash>, including when HEAD is exactly on a tag.
    let git_description = Command::new("git")
        .args(["describe", "--tags", "--long"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());

    let git_version = if let Some(description) = git_description {
        let mut parts = description.rsplitn(3, '-');
        let hash = parts.next().unwrap_or_default();
        let distance = parts.next().unwrap_or_default();
        let tag = parts.next().unwrap_or_default();

        if !tag.is_empty() && hash.starts_with('g') {
            if distance == "0" {
                // Preserve the existing version format for commits on a tag.
                format!("v{version}-{hash}")
            } else if let Ok(distance) = distance.parse::<u64>() {
                format!("{tag}-{hash}-build-{distance}")
            } else {
                format!("v{version}-{hash}")
            }
        } else {
            format!("v{version}-unknown")
        }
    } else {
        // No tag is available (or this is not a git checkout); retain the
        // previous Cargo-version-plus-commit format when possible.
        let git_hash = Command::new("git")
            .args(["rev-parse", "--short", "HEAD"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_else(|| "unknown".into());
        format!("v{version}-{git_hash}")
    };

    let platform = format!(
        "{}-{}",
        std::env::var("CARGO_CFG_TARGET_OS").expect("missing target OS"),
        std::env::var("CARGO_CFG_TARGET_ARCH").expect("missing target architecture")
    );
    let upgrade_repo = "https://api.github.com/repos/jdm64/gaius/releases/latest";

    println!("cargo:rustc-env=GIT_VERSION={git_version}");
    println!("cargo:rustc-env=TAG_VERSION=v{version}");
    println!("cargo:rustc-env=PLATFORM={platform}");
    println!("cargo:rustc-env=UPGRADE_REPO={upgrade_repo}");
}
