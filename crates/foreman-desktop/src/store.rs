//! Saved servers + session tokens, kept as JSON in the app data folder (never localStorage).

use std::{
    fs,
    io::Write as _,
    path::{Path, PathBuf},
};

use serde_json::Value;

pub const FILE: &str = "servers.json";

pub fn path(app_data: &Path) -> PathBuf {
    app_data.join(FILE)
}

/// The stored document, or `{}` when there is none yet.
pub fn load(file: &Path) -> Result<Value, String> {
    match fs::read(file) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", file.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Object(Default::default())),
        Err(e) => Err(format!("{}: {e}", file.display())),
    }
}

/// Write atomically (temp file + rename); owner-only on Unix since it holds tokens.
pub fn save(file: &Path, doc: &Value) -> Result<(), String> {
    if !doc.is_object() {
        return Err("store must be a JSON object".into());
    }
    if let Some(dir) = file.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = file.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(doc).map_err(|e| e.to_string())?;
    {
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp).map_err(|e| e.to_string())?;
        f.write_all(&bytes).map_err(|e| e.to_string())?;
        f.sync_all().map_err(|e| e.to_string())?;
    }
    fs::rename(&tmp, file).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn roundtrip_and_missing() {
        let dir = std::env::temp_dir().join(format!("foreman-store-{}", std::process::id()));
        let file = path(&dir);
        let _ = fs::remove_file(&file);
        assert_eq!(load(&file).expect("empty"), json!({}));
        let doc = json!({ "last": "abc", "servers": [{ "id": "abc", "token": "fms_x" }] });
        save(&file, &doc).expect("save");
        assert_eq!(load(&file).expect("load"), doc);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = fs::metadata(&file).expect("meta").permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        assert!(save(&file, &json!([1])).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
