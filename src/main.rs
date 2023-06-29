mod char_utils;
mod cli;
mod command;
mod edit_buffer;
mod main_loop;
mod peeking;

use std::io;

fn main() {
    let args = match cli::parse_args(&mut io::stdout().lock(), wild::args_os()) {
        Ok(args) => args,
        Err(cli::Error::WroteMessage) => std::process::exit(0),
        Err(err) => {
            eprintln!("Error: {err}");
            std::process::exit(1);
        }
    };
    if let Err(err) = main_loop::run(io::stdin().lock(), io::stdout().lock(), &args) {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}
