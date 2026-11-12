//! The autostart toggle writes (and removes) the OS entry: an XDG autostart
//! `.desktop` file on Linux, a `Run` registry value on Windows.
//!
//! Its own test binary: on Linux it points `HOME` at a temp dir for the whole process.

use foreman_desktop::autostart;
use tauri_plugin_autostart::ManagerExt as _;

/// Distinct from the real "Foreman" entry so the test never touches the user's own.
const TEST_NAME: &str = "ForemanAutostartTest";

#[test]
fn toggle_writes_the_os_entry() {
    #[cfg(target_os = "linux")]
    let home = {
        let home = std::env::temp_dir().join(format!("foreman-autostart-{}", std::process::id()));
        std::fs::create_dir_all(&home).expect("temp home");
        std::env::set_var("HOME", &home);
        home
    };

    let app = tauri::test::mock_builder()
        .plugin(autostart::plugin(TEST_NAME))
        .build(tauri::test::mock_context(tauri::test::noop_assets()))
        .expect("mock app");
    let al = app.autolaunch();
    let _ = al.disable();
    assert!(!al.is_enabled().expect("is_enabled"));

    al.enable().expect("enable");
    assert!(al.is_enabled().expect("is_enabled"));
    let exe = std::env::current_exe().expect("exe").display().to_string();
    check_entry(Some(&exe));

    al.disable().expect("disable");
    assert!(!al.is_enabled().expect("is_enabled"));
    check_entry(None);

    #[cfg(target_os = "linux")]
    let _ = std::fs::remove_dir_all(&home);
}

#[cfg(target_os = "linux")]
fn check_entry(exe: Option<&str>) {
    let home = std::env::var("HOME").expect("HOME");
    let file = std::path::Path::new(&home)
        .join(".config/autostart")
        .join(format!("{TEST_NAME}.desktop"));
    match exe {
        Some(exe) => {
            let body = std::fs::read_to_string(&file).expect("desktop entry written");
            assert!(body.starts_with("[Desktop Entry]"), "{body}");
            assert!(body.contains(&format!("Name={TEST_NAME}")), "{body}");
            assert!(
                body.contains(&format!("Exec={exe} {}", autostart::ARG)),
                "{body}"
            );
        }
        None => assert!(!file.exists(), "desktop entry removed"),
    }
}

#[cfg(windows)]
fn check_entry(exe: Option<&str>) {
    // Elevated runs (CI) write HKLM, normal users HKCU; accept either.
    let query = |root: &str| {
        std::process::Command::new("reg")
            .args([
                "query",
                &format!(r"{root}\SOFTWARE\Microsoft\Windows\CurrentVersion\Run"),
                "/v",
                TEST_NAME,
            ])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
    };
    let found = query("HKCU").or_else(|| query("HKLM"));
    match exe {
        Some(exe) => {
            let out = found.expect("Run value written");
            assert!(out.contains(exe), "{out}");
            assert!(out.contains(autostart::ARG), "{out}");
        }
        None => assert!(found.is_none(), "Run value removed: {found:?}"),
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
fn check_entry(_exe: Option<&str>) {}
