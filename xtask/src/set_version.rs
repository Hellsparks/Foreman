//! `cargo set-version <version>`: one version for the whole release.
//!
//! Writes it to `[workspace.package] version` in the root `Cargo.toml` (every crate,
//! the server's `/api/health`, and the desktop app, whose `tauri.conf.json` has no
//! version of its own) and to `frontend/package.json` + `package-lock.json`.
//! The release workflow runs this with the version from the pushed tag; a leading
//! `v` is stripped. Cargo.lock catches up on the next build.

use std::path::Path;

pub fn run(args: Vec<String>) {
    let Some(raw) = args.first() else {
        eprintln!("usage: cargo set-version <version>   e.g. 0.4.0 or v0.4.0-rc.1");
        std::process::exit(2);
    };
    let version = match normalize(raw) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("set-version: {e}");
            std::process::exit(2);
        }
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap_or(Path::new("."))
        .to_path_buf();
    // (file, number of leading "version" fields to set); 0 = Cargo.toml's
    // [workspace.package]. package-lock.json has two: top level and packages[""].
    let edits = [
        ("Cargo.toml", 0),
        ("frontend/package.json", 1),
        ("frontend/package-lock.json", 2),
    ];
    for (rel, edit) in edits {
        let path = root.join(rel);
        let result = std::fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|s| match edit {
                0 => set_workspace_version(&s, &version),
                n => set_json_versions(&s, &version, n),
            })
            .and_then(|s| std::fs::write(&path, s).map_err(|e| e.to_string()));
        if let Err(e) = result {
            eprintln!("set-version: {rel}: {e}");
            std::process::exit(1);
        }
        println!("{rel}: {version}");
    }
}

/// `v0.4.0-rc.1` → `0.4.0-rc.1`; rejects anything that isn't `X.Y.Z[-pre][+build]`.
pub fn normalize(raw: &str) -> Result<String, String> {
    let v = raw.trim().trim_start_matches('v');
    let core = v.split(['-', '+']).next().unwrap_or("");
    let parts: Vec<&str> = core.split('.').collect();
    let numeric = parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()));
    let rest_ok = v[core.len()..]
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '+' | '.'));
    if numeric && rest_ok && !v.ends_with(['-', '+', '.']) {
        Ok(v.to_string())
    } else {
        Err(format!("'{raw}' is not a version like 0.4.0 or 0.4.0-rc.1"))
    }
}

/// Replace `version = "…"` inside `[workspace.package]`.
pub fn set_workspace_version(toml: &str, version: &str) -> Result<String, String> {
    let mut out = String::with_capacity(toml.len());
    let mut in_section = false;
    let mut done = false;
    for line in toml.split_inclusive('\n') {
        let t = line.trim();
        if t.starts_with('[') {
            in_section = t == "[workspace.package]";
        }
        if in_section && !done && t.starts_with("version") && t.contains('=') {
            let nl = if line.ends_with("\r\n") {
                "\r\n"
            } else if line.ends_with('\n') {
                "\n"
            } else {
                ""
            };
            out.push_str(&format!("version = \"{version}\"{nl}"));
            done = true;
        } else {
            out.push_str(line);
        }
    }
    if done {
        Ok(out)
    } else {
        Err("no version in [workspace.package]".into())
    }
}

/// Replace the first `count` `"version": "…"` values (the package's own, which
/// npm writes before any dependency entries).
pub fn set_json_versions(json: &str, version: &str, count: usize) -> Result<String, String> {
    const KEY: &str = "\"version\": \"";
    let mut out = String::with_capacity(json.len());
    let mut rest = json;
    for _ in 0..count {
        let start = rest
            .find(KEY)
            .ok_or("fewer \"version\" fields than expected")?;
        let value_start = start + KEY.len();
        let end = rest[value_start..]
            .find('"')
            .ok_or("unterminated version string")?;
        out.push_str(&rest[..value_start]);
        out.push_str(version);
        rest = &rest[value_start + end..];
    }
    out.push_str(rest);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_tags() {
        assert_eq!(normalize("v0.4.0").as_deref(), Ok("0.4.0"));
        assert_eq!(normalize("0.4.0-rc.1").as_deref(), Ok("0.4.0-rc.1"));
        assert_eq!(
            normalize("v1.2.3-beta.2+abc").as_deref(),
            Ok("1.2.3-beta.2+abc")
        );
        for bad in ["", "v1", "1.2", "1.2.x", "1.2.3-", "1.2.3 rc", "main"] {
            assert!(normalize(bad).is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn sets_only_the_workspace_package_version() {
        let toml = "[package]\nversion = \"9.9.9\"\n\n[workspace.package]\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[dependencies]\nfoo = { version = \"1\" }\n";
        let out = set_workspace_version(toml, "0.4.0-rc.1").expect("set");
        assert_eq!(
            out,
            "[package]\nversion = \"9.9.9\"\n\n[workspace.package]\nversion = \"0.4.0-rc.1\"\nedition = \"2021\"\n\n[dependencies]\nfoo = { version = \"1\" }\n"
        );
        assert!(set_workspace_version("[workspace]\n", "1.0.0").is_err());
    }

    #[test]
    fn sets_leading_json_versions() {
        let lock = "{\n  \"name\": \"f\",\n  \"version\": \"0.1.0\",\n  \"packages\": {\n    \"\": {\n      \"version\": \"0.1.0\",\n      \"dependencies\": {}\n    },\n    \"node_modules/x\": {\n      \"version\": \"2.0.0\"\n    }\n  }\n}\n";
        let out = set_json_versions(lock, "0.4.0", 2).expect("set");
        assert_eq!(out.matches("\"version\": \"0.4.0\"").count(), 2);
        assert!(out.contains("\"version\": \"2.0.0\""));
    }
}
