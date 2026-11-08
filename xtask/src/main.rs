mod compat;

fn main() {
    let mut args = std::env::args().skip(1);
    match args.next().as_deref() {
        Some("compat") => compat::run(args.collect()),
        Some(cmd) => {
            eprintln!("xtask: unknown command '{cmd}'");
            eprintln!("Available: compat");
            std::process::exit(2);
        }
        None => {
            eprintln!("Usage: cargo xtask <command>");
            eprintln!("       cargo compat [--engine sqlite|postgres|mariadb|cockroach]");
            std::process::exit(2);
        }
    }
}
