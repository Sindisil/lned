use std::path::PathBuf;

const APP_NAME: &str = env!("CARGO_PKG_NAME");
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const APP_DESCRIPTION: &str = env!("CARGO_PKG_DESCRIPTION");
const APP_HELP: &str = "
Usage: lned [OPTIONS] [file ...]

Options:
  -h, --help               print this help text and exit
  -V, --version            print version information and exit
  -s, --quiet, --silent    suppress diagnostic messages
  -p, --no-prompt          suppress display of prompts

Arugments:
  [file ...]  optional list of files to read into buffers
              for editing.

Files, if specified, will be loaded into separate buffers for editing.
If no files are specified, an empty buffer will be created for editing.
The first edit buffer will initially be the active buffer.
";

#[derive(Debug)]
pub struct CmdArgs {
    /// Indicates if diagnostic messages should be suppressed
    quiet: bool,

    /// Indicates if prompts should be suppressed
    no_prompt: bool,

    /// Indicates that default print operation should be n, rather than
    /// p (i.e., print line numbers by default). Explicit use of n or p
    /// commands work normally -- this affects other display commands,
    /// such as z, as well as cases where display occurs as a part of
    /// another operation (such as a bare line address, or the p suffix
    /// to the s command.
    line_numbers: bool,

    /// Specifies the names of files to read
    files: Vec<PathBuf>,
}

pub(crate) fn parse_args() -> Result<CmdArgs, lexopt::Error> {
    use lexopt::prelude::*;

    let mut quiet = false;
    let mut no_prompt = false;
    let mut files = Vec::new();
    let mut line_numbers: bool = false;

    let mut parser = lexopt::Parser::from_iter(wild::args_os());
    while let Some(arg) = parser.next()? {
        match arg {
            Short('h') | Long("help") => {
                if APP_DESCRIPTION.trim().is_empty() {
                    println!("{APP_NAME}");
                } else {
                    println!("{APP_NAME} - {APP_DESCRIPTION}");
                }
                println!("Version {APP_VERSION}");
                print!("{APP_HELP}");
                std::process::exit(0);
            }
            Short('p') | Long("no-prompt") => no_prompt = true,
            Short('s') | Long("silent") | Long("quiet") => quiet = true,
            Short('n') | Long("line-numbers") => line_numbers = true,
            Short('V') | Long("version") => {
                println!("{APP_NAME} version {APP_VERSION}");
                std::process::exit(0);
            }
            Value(val) => {
                files.push(PathBuf::from(val));
                files.extend(parser.raw_args()?.map(PathBuf::from));
            }
            _ => return Err(arg.unexpected()),
        }
    }
    Ok(CmdArgs {
        quiet,
        no_prompt,
        line_numbers,
        files,
    })
}
