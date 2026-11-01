//! `wbquery-cli`: run a saved query and export the matches.

fn main() -> std::process::ExitCode {
    wbquery_cli::main_with(std::env::args_os())
}
