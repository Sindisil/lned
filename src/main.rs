use std::path;

use clap::Parser;
use wild;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct CmdArgs {
    /// Specifies the name of a file to read.
    files: Vec<path::PathBuf>,
}

fn main() {
    let args = wild::args();
    let cmd_args = CmdArgs::parse_from(args);
    println!("files: {:?}", cmd_args.files);
}
