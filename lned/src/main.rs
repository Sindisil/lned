#[cfg(target_has_atomic = "64")]
mod cli;
mod command;
mod edit_buffer;
mod iter_utils;
mod main_loop;
mod num_utils;
mod str_utils;

use std::error::Error;
use std::io;
use std::iter;

use line_reader::LineReader;

fn main() {
    let args = match cli::parse_args(&mut io::stdout().lock(), wild::args_os())
    {
        Ok(args) => args,
        Err(cli::Error::WroteMessage) => std::process::exit(0),
        Err(err) => {
            eprintln!("Error: {err}");
            std::process::exit(1);
        }
    };

    if let Err(err) =
        main_loop::run(LineReader::new(), io::stdout().lock(), &args)
    {
        eprintln!("Error: {err}");
        if let Some(cause) = err.source() {
            println!("\nCaused by:");
            for (i, error) in
                iter::successors(Some(cause), |&e| e.source()).enumerate()
            {
                eprintln!("    {i}: {error}");
            }
            std::process::exit(1);
        }
    }
}
