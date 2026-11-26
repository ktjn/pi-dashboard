use dashboard_core::board_config::load_board_config;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let path = match std::env::args_os().nth(1) {
        Some(arg) => PathBuf::from(arg),
        None => {
            eprintln!("usage: validate_board_config <path-to-board.toml>");
            return ExitCode::FAILURE;
        }
    };

    match load_board_config(&path) {
        Ok(_) => {
            println!("{} is valid", path.display());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}
