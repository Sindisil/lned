use core::fmt::{self, Display, Formatter};
use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;

use lexopt::prelude::*;

pub const APP_NAME: &str = env!("CARGO_PKG_NAME");
pub const APP_VERSION: &str = env!("LNED_VERSION");
pub const APP_DESCRIPTION: &str = env!("CARGO_PKG_DESCRIPTION");
const APP_HELP: &str = "
Usage: lned [OPTIONS] [file]

Options:
  -h, --help               print this help text and exit
  -V, --version            print version information and exit

Arugments:
  [file]  optional file to load for editing

File, if specified, will be loaded into buffer for editing.
If no file is specified, an empty buffer will be created instead.
";

#[derive(Debug)]
pub enum Error {
    WroteMessage, // Wrote message to ouput and should exit with no error
    NextArg { source: lexopt::Error }, // Error reading next argument
    UnexpectedArg { source: lexopt::Error }, // Unexpected cmd line argument
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            Error::WroteMessage => None,
            Error::NextArg { ref source }
            | Error::UnexpectedArg { ref source } => Some(source),
        }
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Error::WroteMessage => write!(f, "message output, no error"),
            Error::NextArg { .. } => {
                write!(f, "error parsing next command line argument")
            }
            Error::UnexpectedArg { .. } => {
                write!(f, "unexpected command line argument")
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct CmdArgs {
    /// Specifies a file to edit
    pub file: Option<PathBuf>,
}

pub fn parse_args(
    mut output: impl Write,
    args: impl IntoIterator<Item = impl Into<OsString>>,
) -> Result<CmdArgs, Error> {
    let mut cmd_args = CmdArgs { file: None };

    let mut parser = lexopt::Parser::from_iter(args);
    while let Some(arg) =
        parser.next().map_err(|source| Error::NextArg { source })?
    {
        match arg {
            Short('h') | Long("help") => {
                writeln!(&mut output, "{APP_NAME} - {APP_DESCRIPTION}")
                    .unwrap();
                writeln!(&mut output, "{APP_VERSION}").unwrap();
                write!(&mut output, "{APP_HELP}").unwrap();
                return Err(Error::WroteMessage);
            }
            Short('V') | Long("version") => {
                writeln!(&mut output, "{APP_NAME} {APP_VERSION}").unwrap();
                return Err(Error::WroteMessage);
            }
            Value(val) if cmd_args.file.is_none() => {
                cmd_args.file = Some(PathBuf::from(val));
            }
            _ => return Err(Error::UnexpectedArg { source: arg.unexpected() }),
        }
    }
    Ok(cmd_args)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::Path;

    use similar_asserts::assert_eq;

    #[test]
    fn help_options_output_help_message() {
        let expected = format!(
            "{APP_NAME} - {APP_DESCRIPTION}\n{APP_VERSION}\n{APP_HELP}"
        );
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
        let expected = format!("{APP_NAME} {APP_VERSION}\n");
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
        assert!(matches!(res, Err(Error::UnexpectedArg { .. })));
    }

    #[test]
    fn filename_option() {
        let args = &["test", r"src\cli.rs"];
        let mut output = Vec::new();
        let res = parse_args(&mut output, args).unwrap();
        assert!(matches!(res.file, Some(p) if p == Path::new(r"src\cli.rs")));
    }

    #[test]
    fn extra_filename_is_error() {
        let args = &["test", r"src\cli.rs", r"src\main.rs"];
        let mut output = Vec::new();
        let res = parse_args(&mut output, args).expect_err("unexpected arg");
        assert!(matches!(res, Error::UnexpectedArg { .. }));
    }
}
