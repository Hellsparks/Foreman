//! Embeds the git commit and build time so `/api/health` and the startup log can
//! prove which code the running binary was built from.

use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    (!s.is_empty()).then_some(s)
}

fn main() {
    let commit = git(&["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=FOREMAN_GIT_COMMIT={commit}");

    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    println!("cargo:rustc-env=FOREMAN_BUILD_EPOCH={secs}");

    // Re-run when HEAD moves (checkout, commit) so the embedded commit stays true,
    // and when server/core sources change so the build time tracks real rebuilds.
    if let Some(git_dir) = git(&["rev-parse", "--absolute-git-dir"]) {
        println!("cargo:rerun-if-changed={git_dir}/HEAD");
        if let Some(head_ref) = git(&["symbolic-ref", "-q", "HEAD"]) {
            println!("cargo:rerun-if-changed={git_dir}/{head_ref}");
        }
        println!("cargo:rerun-if-changed={git_dir}/packed-refs");
    }
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=../foreman-core/src");
    println!("cargo:rerun-if-changed=../spoolman-rs/src");
}
