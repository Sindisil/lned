use core::fmt::{self, Display, Formatter};
use core::iter::IntoIterator;
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};

use lexopt::prelude::*;

const APP_NAME: &str = env!("CARGO_PKG_NAME");
const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const APP_DESCRIPTION: &str = env!("CARGO_PKG_DESCRIPTION");
const APP_HELP: &str = "
Usage: lned [OPTIONS] [file ...]

Options:
  -h, --help               print this help text and exit
  -V, --version            print version information and exit

Arugments:
  [file ...]  optional list of files to read into buffers
              for editing.

Files, if specified, will be loaded into separate buffers for editing.
If no files are specified, an empty buffer will be created for editing.
The first edit buffer will initially be the active buffer.
";

#[derive(Debug)]
pub enum Error {
    ParsingFilename(lexopt::Error), // Error parsing filename from cmd line
    WroteMessage,                   // Wrote message to ouput and should exit with no error
    NextArg(lexopt::Error),         // Error reading next argument
    UnexpectedArg(lexopt::Error),   // Unexpected cmd line argument
}

#[derive(Debug, Default)]
pub struct CmdArgs {
    /// Indicates that default print operation should be n, rather than
    /// p (i.e., print line numbers by default). Explicit use of n or p
    /// commands work normally -- this affects other display commands,
    /// such as z, as well as cases where display occurs as a part of
    /// another operation (such as a bare line address, or the p suffix
    /// to the s command.
    pub line_numbers: bool,

    /// Specifies the names of files to read
    pub files: Vec<PathBuf>,
}

pub(crate) fn parse_args<W, I>(mut output: W, args: I) -> Result<CmdArgs, Error>
where
    W: Write,
    I: IntoIterator,
    I::Item: Into<OsString>,
{
    let mut files = Vec::new();
    let mut line_numbers: bool = false;

    let mut parser = lexopt::Parser::from_iter(args);
    while let Some(arg) = parser.next().map_err(Error::NextArg)? {
        match arg {
            Short('h') | Long("help") => {
                writeln!(&mut output, "{APP_NAME} - {APP_DESCRIPTION}")
                    .expect("Writing to stdout shouldn't fail");
                writeln!(&mut output, "Version {APP_VERSION}")
                    .expect("writing to stdout shouldn't fail");
                write!(&mut output, "{APP_HELP}").expect("writing to stdout shouldn't fail");
                return Err(Error::WroteMessage);
            }
            Short('n') | Long("line-numbers") => line_numbers = true,
            Short('V') | Long("version") => {
                writeln!(&mut output, "{APP_NAME} version {APP_VERSION}")
                    .expect("writing to stdout shouldn't fail");
                return Err(Error::WroteMessage);
            }
            Value(val) => {
                files.push(PathBuf::from(val));
                files.extend(
                    parser
                        .raw_args()
                        .map_err(Error::ParsingFilename)?
                        .map(PathBuf::from),
                );
            }
            _ => return Err(Error::UnexpectedArg(arg.unexpected())),
        }
    }
    Ok(CmdArgs {
        line_numbers,
        files,
    })
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            Error::WroteMessage => write!(f, "message output, no error"),
            Error::ParsingFilename(_) => write!(f, "error parsing filame from command line"),
            Error::NextArg(_) => write!(f, "Error parsing next command line argument"),
            Error::UnexpectedArg(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_options_output_help_message() {
        let expected = format!("{APP_NAME} - {APP_DESCRIPTION}\nVersion {APP_VERSION}\n{APP_HELP}");
        let mut output = Vec::new();
        let args = &["test", "-h"];
        let res = parse_args(&mut output, args);
        assert!(matches!(res, Err(Error::WroteMessage)));
        assert_eq!(std::str::from_utf8(&output).unwrap(), expected);
        output.clear();
        let args = &["test", "--help"];
        let res = parse_args(&mut output, args);
        assert!(matches!(res, Err(Error::WroteMessage)));
        assert_eq!(std::str::from_utf8(&output).unwrap(), expected);
    }

    #[test]
    fn version_options_output_version_message() {
        let expected = format!("{APP_NAME} version {APP_VERSION}\n");
        let mut output = Vec::new();
        let args = &["test", "-V"];
        let res = parse_args(&mut output, args);
        assert!(matches!(res, Err(Error::WroteMessage)));
        assert_eq!(std::str::from_utf8(&output).unwrap(), expected);
        output.clear();
        let args = &["test", "--version"];
        let res = parse_args(&mut output, args);
        assert!(matches!(res, Err(Error::WroteMessage)));
        assert_eq!(std::str::from_utf8(&output).unwrap(), expected);
    }

    #[test]
    fn unexpected_option_gives_error() {
        let mut output = Vec::new();
        let args = &["test", "--unexpected-arg"];
        let res = parse_args(&mut output, args);
        assert!(matches!(res, Err(Error::UnexpectedArg(_))));
    }

    #[test]
    fn filename_options() {
        let args = &["test", "src\\cli.rs", "src\\main.rs"];
        let mut output = Vec::new();
        let res = parse_args(&mut output, args).expect("parsed filenames");
        assert_eq!(2, res.files.len());
        let expected = vec![
            Path::new(r"src\cli.rs").to_path_buf(),
            Path::new(r"src\main.rs").to_path_buf(),
        ];
        assert_eq!(expected, res.files);
    }
}
