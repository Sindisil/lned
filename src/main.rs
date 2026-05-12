#[cfg(target_has_atomic = "64")]
mod cli;
mod command;
mod edit_buffer;
mod editor;
mod eol;
mod error;
mod iter_utils;
mod undo_stack;

use std::error::Error;
use std::io::{self, IsTerminal};
use std::iter;

use line_edit::LineEditor;

use crate::editor::OutputTarget;

#[cfg(not(tarpaulin_include))]
fn main() {
    let args = match cli::parse_args(&mut io::stdout(), wild::args_os()) {
        Ok(Some(args)) => args,
        Ok(None) => std::process::exit(0),
        Err(err) => {
            eprintln!("Error: {err}");
            std::process::exit(1);
        }
    };

    let output_target = if io::stdout().is_terminal() {
        OutputTarget::Terminal
    } else {
        OutputTarget::Other
    };
    if let Err(err) =
        editor::run(LineEditor::new(), io::stdout(), output_target, &args)
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
