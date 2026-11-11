//! `cargo check-all`: runs `scripts/dev-check.sh` (needs `bash` on PATH; on Windows,
//! Git for Windows provides it). Arguments pass through, e.g. `--printers --keep`.

use std::path::PathBuf;
use std::process::Command;

pub fn run(args: Vec<String>) {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let script = root.join("scripts").join("dev-check.sh");
    let status = Command::new("bash")
        .arg(&script)
        .args(&args)
        .current_dir(&root)
        .status();
    match status {
        Ok(s) => std::process::exit(s.code().unwrap_or(1)),
        Err(e) => {
            eprintln!("xtask: could not run bash {}: {e}", script.display());
            std::process::exit(2);
        }
    }
}
