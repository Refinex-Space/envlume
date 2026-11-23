use std::{env, process::ExitCode};

const USAGE: &str = "Usage: envlume --version";

fn main() -> ExitCode {
    let mut args = env::args_os().skip(1);
    let first = args.next();
    if args.next().is_none() {
        match first.as_deref().and_then(|arg| arg.to_str()) {
            Some("--version" | "-V") => {
                println!("envlume {}", envlume_application::VERSION);
                return ExitCode::SUCCESS;
            }
            Some("--help" | "-h") => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            _ => {}
        }
    }
    eprintln!("{USAGE}");
    ExitCode::from(2)
}
