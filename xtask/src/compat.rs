//! `cargo compat` — runs the upstream Spoolman v0.27.1 `tests_integration/`
//! suite directly against a locally-started spoolman-rs server.
//!
//! # Usage
//!
//! ```
//! cargo compat                        # all four engines
//! cargo compat --engine sqlite        # SQLite only (no Docker)
//! cargo compat --engine postgres      # Postgres (requires Docker)
//! cargo compat --engine mariadb       # MariaDB  (requires Docker)
//! cargo compat --engine cockroach     # CockroachDB (report only, requires Docker)
//! ```
//!
//! # What it does
//!
//! 1. Builds the `spoolman-rs` binary (debug).
//! 2. Locates or clones the upstream `tests_integration/` suite
//!    (`vendor/spoolman-upstream/` preferred; falls back to
//!    `compat-tests/spoolman-v0.27.1/`).
//! 3. Creates a Python venv in `compat-tests/venv/` and installs pytest deps.
//! 4. For each requested engine:
//!    a. Starts a Docker container for the DB engine (skipped for SQLite).
//!    b. Starts `spoolman-rs` on a free port with appropriate env vars.
//!    c. Runs `pytest --exitfirst tests/` from the `tests_integration/` dir.
//!    d. Stops the server and DB container.
//! 5. Prints a pass/fail summary.  CockroachDB failures are reported but do
//!    not cause a non-zero exit.

use std::{
    env, fs,
    net::TcpStream,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn run(args: Vec<String>) {
    let engines = parse_engines(&args);

    let cargo = env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let workspace = workspace_root();

    println!("=== cargo compat ===");
    println!("workspace: {}", workspace.display());

    // Build the server binary first so each engine run doesn't have to.
    build_server(&cargo, &workspace);

    // Locate upstream integration tests.
    let tests_int = find_tests_integration(&workspace);
    println!("tests_integration: {}", tests_int.display());

    // Create / verify Python venv.
    let pytest_bin = setup_venv(&workspace, &tests_int);
    println!("pytest: {}", pytest_bin.display());

    // Run engines.
    let mut results: Vec<EngineResult> = Vec::new();
    for engine in &engines {
        let r = run_engine(engine, &workspace, &pytest_bin, &tests_int);
        results.push(r);
    }

    print_summary(&results);

    // Fail the process if any non-report-only engine failed.
    let hard_fail = results.iter().any(|r| !r.passed && !r.report_only);
    if hard_fail {
        std::process::exit(1);
    }
}

// ---------------------------------------------------------------------------
// Argument parsing
// ---------------------------------------------------------------------------

const ALL_ENGINES: &[&str] = &["sqlite", "postgres", "mariadb", "cockroach"];

fn parse_engines(args: &[String]) -> Vec<String> {
    let mut engines: Vec<String> = Vec::new();
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == "--engine" {
            if let Some(e) = iter.next() {
                engines.push(e.clone());
            }
        } else if let Some(e) = arg.strip_prefix("--engine=") {
            engines.push(e.to_owned());
        }
    }
    if engines.is_empty() {
        ALL_ENGINES.iter().map(|s| s.to_string()).collect()
    } else {
        for e in &engines {
            if !ALL_ENGINES.contains(&e.as_str()) {
                eprintln!(
                    "Unknown engine '{e}'. Expected one of: {}",
                    ALL_ENGINES.join(", ")
                );
                std::process::exit(2);
            }
        }
        engines
    }
}

// ---------------------------------------------------------------------------
// Workspace root
// ---------------------------------------------------------------------------

fn workspace_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is set to the xtask crate dir by cargo.
    // The workspace root is one level up.
    let manifest = env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR not set — run via `cargo compat`");
    PathBuf::from(manifest)
        .parent()
        .expect("xtask has no parent directory?")
        .to_path_buf()
}

// ---------------------------------------------------------------------------
// Build server
// ---------------------------------------------------------------------------

fn build_server(cargo: &str, workspace: &Path) {
    println!("Building spoolman-rs …");
    // Pass RUSTC explicitly so cargo doesn't fall through to any system shim.
    let rustc = env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let status = Command::new(cargo)
        .args(["build", "-p", "spoolman-rs"])
        .env("RUSTC", &rustc)
        .current_dir(workspace)
        .status()
        .expect("failed to run cargo build");
    if !status.success() {
        eprintln!("cargo build failed");
        std::process::exit(1);
    }
    println!("Build OK.");
}

fn server_bin(workspace: &Path) -> PathBuf {
    let name = if cfg!(windows) {
        "spoolman-rs.exe"
    } else {
        "spoolman-rs"
    };
    workspace.join("target").join("debug").join(name)
}

// ---------------------------------------------------------------------------
// Locate upstream tests_integration/
// ---------------------------------------------------------------------------

fn find_tests_integration(workspace: &Path) -> PathBuf {
    // 1. Prefer vendor/ (already cloned locally by the developer).
    let vendor = workspace
        .join("vendor")
        .join("spoolman-upstream")
        .join("tests_integration");
    if vendor.join("tests").exists() {
        return vendor;
    }

    // 2. Fallback: compat-tests/spoolman-v0.27.1/ (previously cloned by this tool).
    let compat = workspace
        .join("compat-tests")
        .join("spoolman-v0.27.1")
        .join("tests_integration");
    if compat.join("tests").exists() {
        return compat;
    }

    // 3. Clone from GitHub.
    println!("Upstream tests not found; cloning Donkie/Spoolman v0.27.1 …");
    let clone_dir = workspace.join("compat-tests").join("spoolman-v0.27.1");
    fs::create_dir_all(&clone_dir).expect("failed to create compat-tests/");
    let status = Command::new("git")
        .args([
            "clone",
            "--depth",
            "1",
            "--branch",
            "v0.27.1",
            "--filter=blob:none",
            "--sparse",
            "https://github.com/Donkie/Spoolman",
        ])
        .arg(&clone_dir)
        .status()
        .expect("failed to run git clone");
    if !status.success() {
        eprintln!("git clone failed — ensure git and network access are available");
        std::process::exit(1);
    }
    // Sparse checkout: only tests_integration/
    Command::new("git")
        .args(["sparse-checkout", "set", "tests_integration"])
        .current_dir(&clone_dir)
        .status()
        .ok();

    let result = clone_dir.join("tests_integration");
    if !result.join("tests").exists() {
        eprintln!(
            "tests_integration/tests not found after clone at {}",
            result.display()
        );
        std::process::exit(1);
    }
    result
}

// ---------------------------------------------------------------------------
// Python venv setup
// ---------------------------------------------------------------------------

/// Returns (uv_available, pytest_bin_path).
fn setup_venv(workspace: &Path, tests_int: &Path) -> PathBuf {
    let venv_dir = workspace.join("compat-tests").join("venv");
    let requirements = tests_int.join("requirements.txt");

    // Determine pytest / python binary paths (platform-specific).
    let (pytest_bin, python_bin) = if cfg!(windows) {
        (
            venv_dir.join("Scripts").join("pytest.exe"),
            venv_dir.join("Scripts").join("python.exe"),
        )
    } else {
        (
            venv_dir.join("bin").join("pytest"),
            venv_dir.join("bin").join("python"),
        )
    };

    // Create venv + install deps if pytest doesn't exist yet.
    if !pytest_bin.exists() {
        println!("Creating Python venv at {} …", venv_dir.display());
        fs::create_dir_all(workspace.join("compat-tests")).expect("failed to create compat-tests/");

        let uv_available = uv_exists();

        if uv_available {
            let ok = Command::new("uv")
                .arg("venv")
                .arg(&venv_dir)
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            if !ok {
                eprintln!("uv venv failed");
                std::process::exit(1);
            }
        } else if !try_python_venv(&venv_dir) {
            eprintln!("Failed to create Python venv — install uv or python3");
            std::process::exit(1);
        }

        println!("Installing Python dependencies …");
        let install_ok = if uv_available {
            // `uv pip install` works without pip in the venv.
            Command::new("uv")
                .args(["pip", "install", "--quiet", "--python"])
                .arg(&python_bin)
                .arg("-r")
                .arg(&requirements)
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        } else {
            Command::new(&python_bin)
                .args(["-m", "pip", "install", "--quiet", "-r"])
                .arg(&requirements)
                .status()
                .map(|s| s.success())
                .unwrap_or(false)
        };

        if !install_ok {
            eprintln!("pip install -r requirements.txt failed");
            std::process::exit(1);
        }
        println!("Python venv ready.");
    }

    pytest_bin
}

fn uv_exists() -> bool {
    Command::new("uv")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn try_python_venv(venv_dir: &Path) -> bool {
    // Try python3 then python.
    for python in &["python3", "python"] {
        if Command::new(python)
            .args(["-m", "venv"])
            .arg(venv_dir)
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Engine runner
// ---------------------------------------------------------------------------

struct EngineResult {
    engine: String,
    passed: bool,
    report_only: bool,
    note: String,
}

fn run_engine(engine: &str, workspace: &Path, pytest_bin: &Path, tests_int: &Path) -> EngineResult {
    println!("\n━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━");
    println!("  Engine: {engine}");
    println!("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    let (spoolman_db_type, pytest_db_type, report_only) = engine_params(engine);

    // Allocate a free port for spoolman-rs.
    let port = free_port();

    // Data dir for SQLite (unused for remote DBs but harmless to set).
    let data_dir = env::temp_dir().join(format!("spoolman-compat-{engine}-{port}"));
    fs::create_dir_all(&data_dir).expect("failed to create temp data dir");

    // Start DB container if this engine needs one.
    let container = if engine != "sqlite" {
        match start_container(engine) {
            Ok(c) => Some(c),
            Err(e) => {
                return EngineResult {
                    engine: engine.to_owned(),
                    passed: false,
                    report_only,
                    note: format!("Failed to start DB container: {e}"),
                };
            }
        }
    } else {
        None
    };

    // Start spoolman-rs.
    let server = {
        let mut cmd = Command::new(server_bin(workspace));
        cmd.env("SPOOLMAN_HOST", "127.0.0.1")
            .env("SPOOLMAN_PORT", port.to_string())
            .env("SPOOLMAN_DB_TYPE", spoolman_db_type)
            .env("SPOOLMAN_DIR_DATA", &data_dir)
            .env("SPOOLMAN_AUTOMATIC_BACKUP", "FALSE")
            // Forward RUST_LOG if set, otherwise silence the server.
            .env(
                "RUST_LOG",
                env::var("RUST_LOG").unwrap_or_else(|_| "error".into()),
            )
            .stdout(Stdio::inherit())
            .stderr(Stdio::inherit());

        if let Some(ref c) = container {
            apply_db_env(&mut cmd, engine, c);
        }

        match cmd.spawn() {
            Ok(child) => child,
            Err(e) => {
                stop_container(container_name(container.as_ref()));
                let _ = fs::remove_dir_all(&data_dir);
                return EngineResult {
                    engine: engine.to_owned(),
                    passed: false,
                    report_only,
                    note: format!("Failed to start spoolman-rs: {e}"),
                };
            }
        }
    };

    // Wait for the server to accept connections.
    let addr = format!("127.0.0.1:{port}");
    if !wait_for_port(&addr, Duration::from_secs(30)) {
        kill_child(server);
        stop_container(container_name(container.as_ref()));
        let _ = fs::remove_dir_all(&data_dir);
        return EngineResult {
            engine: engine.to_owned(),
            passed: false,
            report_only,
            note: format!("spoolman-rs did not start within 30 s on port {port}"),
        };
    }
    println!("spoolman-rs ready on port {port}.");

    // Run pytest.
    let status = Command::new(pytest_bin)
        .arg("--exitfirst")
        .arg("tests")
        .current_dir(tests_int)
        .env("SPOOLMAN_HOST", "127.0.0.1")
        .env("SPOOLMAN_PORT", port.to_string())
        .env("DB_TYPE", pytest_db_type)
        .status();

    // Tear down.
    kill_child(server);
    stop_container(container_name(container.as_ref()));
    let _ = fs::remove_dir_all(&data_dir);

    match status {
        Ok(s) => EngineResult {
            engine: engine.to_owned(),
            passed: s.success(),
            report_only,
            note: String::new(),
        },
        Err(e) => EngineResult {
            engine: engine.to_owned(),
            passed: false,
            report_only,
            note: format!("pytest invocation error: {e}"),
        },
    }
}

fn engine_params(engine: &str) -> (&'static str, &'static str, bool) {
    // (SPOOLMAN_DB_TYPE, DB_TYPE for pytest, report_only)
    match engine {
        "sqlite" => ("sqlite", "sqlite", false),
        "postgres" => ("postgres", "postgres", false),
        "mariadb" => ("mysql", "mysql", false),
        "cockroach" => ("cockroachdb", "cockroachdb", true),
        other => panic!("Unknown engine '{other}'"),
    }
}

// ---------------------------------------------------------------------------
// Docker helpers
// ---------------------------------------------------------------------------

struct Container {
    name: String,
    /// Port on the host that the DB listens on.
    host_port: u16,
}

fn start_container(engine: &str) -> Result<Container, String> {
    let pid = std::process::id();
    let (name, image, host_port, run_args): (&str, &str, u16, Vec<String>) = match engine {
        "postgres" => (
            "compat-pg",
            "postgres:15-alpine",
            5432,
            vec![
                "-e".into(),
                "POSTGRES_PASSWORD=abc".into(),
                "-e".into(),
                "POSTGRES_USER=john".into(),
                "-e".into(),
                "POSTGRES_DB=spoolman".into(),
            ],
        ),
        "mariadb" => (
            "compat-mysql",
            "mariadb:lts",
            3306,
            vec![
                "-e".into(),
                "MARIADB_USER=john".into(),
                "-e".into(),
                "MARIADB_PASSWORD=abc".into(),
                "-e".into(),
                "MARIADB_DATABASE=spoolman".into(),
                "-e".into(),
                "MARIADB_RANDOM_ROOT_PASSWORD=yes".into(),
            ],
        ),
        "cockroach" => (
            "compat-crdb",
            "cockroachdb/cockroach:v23.1.2",
            26257,
            vec![
                "-p".into(),
                "8080:8080".into(), // admin UI
            ],
        ),
        other => return Err(format!("No Docker spec for engine '{other}'")),
    };

    let container_name = format!("{name}-{pid}");

    // Remove a stale container from a previous run.
    Command::new("docker")
        .args(["rm", "-f", &container_name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok();

    println!("Starting {engine} container ({container_name}) …");

    let port_map = format!("{host_port}:{host_port}");
    let mut args = vec![
        "run".to_owned(),
        "-d".into(),
        "--name".into(),
        container_name.clone(),
        "-p".into(),
        port_map,
    ];
    args.extend(run_args);

    // CockroachDB needs a start subcommand.
    if engine == "cockroach" {
        args.push(image.into());
        args.push("start-single-node".into());
        args.push("--insecure".into());
    } else {
        args.push(image.into());
    }

    let status = Command::new("docker")
        .args(&args)
        .status()
        .map_err(|e| format!("docker run: {e}"))?;

    if !status.success() {
        return Err(format!("docker run failed for {engine}"));
    }

    let container = Container {
        name: container_name.clone(),
        host_port,
    };

    // Wait for the DB port to be reachable.
    println!("Waiting for {engine} DB on port {host_port} …");
    if !wait_for_port(&format!("127.0.0.1:{host_port}"), Duration::from_secs(60)) {
        stop_container(Some(&container_name));
        return Err(format!(
            "{engine} DB did not become reachable on port {host_port} within 60 s"
        ));
    }

    // CockroachDB: create DB + user after the node is up.
    if engine == "cockroach" {
        // Extra wait: port open doesn't mean the SQL layer is ready.
        thread::sleep(Duration::from_secs(3));

        let sql = "CREATE DATABASE IF NOT EXISTS spoolman; \
                   CREATE USER IF NOT EXISTS john; \
                   GRANT ALL ON DATABASE spoolman TO john;";
        let init_status = Command::new("docker")
            .args([
                "exec",
                &container_name,
                "./cockroach",
                "sql",
                "--insecure",
                "--execute",
                sql,
            ])
            .status()
            .map_err(|e| format!("docker exec cockroach sql: {e}"))?;
        if !init_status.success() {
            stop_container(Some(&container_name));
            return Err("Failed to initialise CockroachDB schema".into());
        }
    }

    // MariaDB: extra time for the server to fully initialise after port opens.
    if engine == "mariadb" {
        thread::sleep(Duration::from_secs(5));
    }

    println!("{engine} DB ready.");
    Ok(container)
}

fn apply_db_env(cmd: &mut Command, engine: &str, container: &Container) {
    let host_port = container.host_port.to_string();
    match engine {
        "postgres" => {
            cmd.env("SPOOLMAN_DB_HOST", "127.0.0.1")
                .env("SPOOLMAN_DB_PORT", &host_port)
                .env("SPOOLMAN_DB_NAME", "spoolman")
                .env("SPOOLMAN_DB_USERNAME", "john")
                .env("SPOOLMAN_DB_PASSWORD", "abc");
        }
        "mariadb" => {
            cmd.env("SPOOLMAN_DB_HOST", "127.0.0.1")
                .env("SPOOLMAN_DB_PORT", &host_port)
                .env("SPOOLMAN_DB_NAME", "spoolman")
                .env("SPOOLMAN_DB_USERNAME", "john")
                .env("SPOOLMAN_DB_PASSWORD", "abc");
        }
        "cockroach" => {
            cmd.env("SPOOLMAN_DB_HOST", "127.0.0.1")
                .env("SPOOLMAN_DB_PORT", &host_port)
                .env("SPOOLMAN_DB_NAME", "spoolman")
                .env("SPOOLMAN_DB_USERNAME", "john");
            // CockroachDB insecure mode: no password needed.
        }
        _ => {}
    }
}

fn container_name(c: Option<&Container>) -> Option<&str> {
    c.map(|c| c.name.as_str())
}

fn stop_container(name: Option<&str>) {
    let Some(name) = name else { return };
    Command::new("docker")
        .args(["stop", name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok();
    Command::new("docker")
        .args(["rm", "-f", name])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok();
}

// ---------------------------------------------------------------------------
// Port utilities
// ---------------------------------------------------------------------------

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind to :0 failed");
    listener.local_addr().unwrap().port()
}

fn wait_for_port(addr: &str, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if TcpStream::connect(addr).is_ok() {
            return true;
        }
        thread::sleep(Duration::from_millis(300));
    }
    false
}

// ---------------------------------------------------------------------------
// Process helpers
// ---------------------------------------------------------------------------

fn kill_child(mut child: Child) {
    // Best-effort kill.
    child.kill().ok();
    child.wait().ok();
}

// ---------------------------------------------------------------------------
// Summary
// ---------------------------------------------------------------------------

fn print_summary(results: &[EngineResult]) {
    println!("\n╔══════════════════════════════════════════════╗");
    println!("║            cargo compat — summary            ║");
    println!("╠══════════════════════════════════════════════╣");
    for r in results {
        let icon = if r.passed { "✓" } else { "✗" };
        let tag = if r.report_only { " (report only)" } else { "" };
        println!("║  {icon} {:<38}║", format!("{}{tag}", r.engine));
        if !r.note.is_empty() {
            println!("║    {:<42}║", &r.note[..r.note.len().min(42)]);
        }
    }
    println!("╚══════════════════════════════════════════════╝");
}
