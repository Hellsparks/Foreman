mod compat;
mod dev_check;
mod set_version;

fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("compat") => compat::run(args.collect()),
        Some("dev-check") => dev_check::run(args.collect()),
        Some("set-version") => set_version::run(args.collect()),
        Some(cmd) => {
            eprintln!("xtask: unknown command '{cmd}'");
            eprintln!("Available: compat, dev-check, set-version");
            std::process::exit(2);
        }
        None => {
            eprintln!("Usage: cargo xtask <command>");
            eprintln!("       cargo compat [--engine sqlite|postgres|mariadb|cockroach]");
            std::process::exit(2);
        }
    }
}
