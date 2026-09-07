use arb_app::config::Config;
use std::{env, fs, process::ExitCode};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() != 3 || args[0] != "check-config" || args[1] != "--config" {
        eprintln!("usage: arb-app check-config --config <path>");
        return ExitCode::FAILURE;
    }
    let result = fs::read_to_string(&args[2])
        .map_err(|_| "cannot read configuration file".to_owned())
        .and_then(|s| Config::parse(&s).map_err(|e| e.to_string()));
    match result {
        Ok(_) => {
            println!("configuration valid (offline validation; network not contacted)");
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
