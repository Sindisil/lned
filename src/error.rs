use std::fmt::{Display, Formatter, Result};
use std::ops::Range;
use std::path::PathBuf;

use crate::undo_stack::ChangeSet;

#[derive(Debug)]
pub enum Error {
    DestinationIntersectsSource,
    DiffReadFile {
        source: Option<Box<dyn std::error::Error>>,
        filename: PathBuf,
    },
    EditFileOpen {
        source: Option<Box<dyn std::error::Error>>,
        filename: PathBuf,
    },
    FileNotFound(PathBuf),
    GlobalCmdErrorStop {
        source: Box<Error>,
        changes: Option<ChangeSet>,
    },
    InvalidAddress,
    InvalidCmdLine {
        source: Option<Box<dyn std::error::Error>>,
    },
    InvalidCmdSuffix,
    InvalidDelimiter,
    InvalidNewline,
    InvalidOffset,
    MissingDestination,
    MissingPatternDelimiter,
    NestedGlobalCmd,
    NoFilename,
    NoMatch,
    NoPreviousPattern,
    NothingToRedo,
    NothingToUndo,
    Quit,
    ReadCommand {
        source: Option<Box<dyn std::error::Error>>,
    },
    ReadFileOpen {
        source: Option<Box<dyn std::error::Error>>,
        file: PathBuf,
    },
    ReadGlobalCmd {
        source: Option<Box<dyn std::error::Error>>,
    },
    ReadLines {
        source: Option<Box<dyn std::error::Error>>,
    },
    Regex {
        source: Option<Box<dyn std::error::Error>>,
    },
    TrailingBackslash,
    UnexpectedAddress,
    UnknownCmd(String),
    UnsupportedGlobalCmd,
    Warning(Warning),
    WriteAsCurrentFile,
    WriteBackupFileCreate {
        source: Option<Box<dyn std::error::Error>>,
        filename: PathBuf,
        backup_filename: Option<PathBuf>,
    },
    WriteFile {
        source: Option<Box<dyn std::error::Error>>,
        filename: PathBuf,
        backup_filename: Option<PathBuf>,
    },
    WriteFileOpen {
        source: Option<Box<dyn std::error::Error>>,
        filename: PathBuf,
    },
    WriteMakeBackup {
        source: Option<Box<dyn std::error::Error>>,
        filename: PathBuf,
        backup_filename: Option<PathBuf>,
    },
    WriteRemoveBackup {
        source: Option<Box<dyn std::error::Error>>,
        backup_filename: Option<PathBuf>,
    },
}

impl std::error::Error for Error {
    #[cfg(not(tarpaulin_include))]
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            // No source
            Error::DestinationIntersectsSource
            | Error::FileNotFound(_)
            | Error::InvalidAddress
            | Error::InvalidCmdSuffix
            | Error::InvalidDelimiter
            | Error::InvalidNewline
            | Error::InvalidOffset
            | Error::MissingDestination
            | Error::MissingPatternDelimiter
            | Error::NestedGlobalCmd
            | Error::NoFilename
            | Error::NoMatch
            | Error::NoPreviousPattern
            | Error::NothingToUndo
            | Error::NothingToRedo
            | Error::Quit
            | Error::TrailingBackslash
            | Error::UnexpectedAddress
            | Error::UnknownCmd(_)
            | Error::UnsupportedGlobalCmd
            | Error::Warning(_)
            | Error::WriteAsCurrentFile => None,
            // Source is Box<editor::Error>
            Error::GlobalCmdErrorStop { ref source, .. } => Some(source),
            // Source is Option<Box<dyn std::error::Error>>
            Error::DiffReadFile { ref source, .. }
            | Error::ReadLines { ref source }
            | Error::WriteBackupFileCreate { ref source, .. }
            | Error::WriteFile { ref source, .. }
            | Error::WriteFileOpen { ref source, .. }
            | Error::WriteMakeBackup { ref source, .. }
            | Error::WriteRemoveBackup { ref source, .. }
            | Error::InvalidCmdLine { ref source, .. }
            | Error::EditFileOpen { ref source, .. }
            | Error::ReadGlobalCmd { ref source, .. }
            | Error::ReadCommand { ref source }
            | Error::ReadFileOpen { ref source, .. }
            | Error::Regex { ref source, .. } => source.as_deref(),
        }
    }
}

impl Display for Error {
    #[allow(clippy::too_many_lines)]
    #[cfg(not(tarpaulin_include))]
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match self {
            Error::DestinationIntersectsSource => {
                write!(f, "destination intersects source")
            }
            Error::DiffReadFile { filename, .. } => {
                write!(f, "error reading {} for diff", filename.display())
            }
            Error::EditFileOpen { filename, .. } => {
                write!(f, "error opening \"{}\" to edit", filename.display())
            }
            Error::FileNotFound(filename) => {
                write!(f, "{} not found", filename.display())
            }
            Error::GlobalCmdErrorStop { .. } => {
                write!(f, "error executing global command")
            }
            Error::InvalidAddress => write!(f, "invalid address"),
            Error::InvalidCmdLine { .. } => write!(f, "invalid command line"),
            Error::InvalidCmdSuffix => write!(f, "invalid command suffix"),
            Error::InvalidDelimiter => {
                write!(f, "invalid delimiter")
            }
            Error::InvalidNewline => {
                write!(f, "invalid newline (valid: CR, CRLF)")
            }
            Error::InvalidOffset => write!(f, "invalid offset"),
            Error::MissingDestination => write!(f, "missing destination"),
            Error::MissingPatternDelimiter => {
                write!(f, "missing pattern delimiter")
            }
            Error::NestedGlobalCmd => {
                write!(f, "invalid nested global command")
            }
            Error::NoFilename => write!(f, "no filename"),
            Error::NoMatch => {
                write!(f, "no matches found")
            }
            Error::NoPreviousPattern => write!(f, "no previous pattern"),
            Error::NothingToRedo => write!(f, "nothing to redo"),
            Error::NothingToUndo => write!(f, "nothing to undo"),
            Error::Quit => write!(f, "exiting ..."),
            Error::ReadCommand { .. } => {
                write!(f, "error reading command input")
            }
            Error::ReadFileOpen { file, .. } => {
                write!(f, "error opening \"{}\" to read", file.display())
            }
            Error::ReadGlobalCmd { .. } => {
                write!(f, "error reading global command")
            }
            Error::ReadLines { .. } => {
                write!(f, "error reading input lines")
            }
            Error::Regex { .. } => write!(f, "bad regex"),
            Error::TrailingBackslash => write!(f, "invalid trailing backslash"),
            Error::UnexpectedAddress => {
                write!(f, "unexpected line address")
            }
            Error::UnknownCmd(c) => write!(f, "unknown command '{c}'"),
            Error::UnsupportedGlobalCmd => {
                write!(f, "unsupported global command")
            }
            Error::Warning(warning) => {
                write!(f, "{warning}")
            }
            Error::WriteAsCurrentFile => {
                write!(f, "specified filename may not be same as current file")
            }
            Error::WriteBackupFileCreate {
                filename, backup_filename, ..
            } => {
                write!(
                    f,
                    "error creating \"{}\" as backup for \"{}\"",
                    backup_filename
                        .as_ref()
                        .expect("backup path exists if this error produced")
                        .display(),
                    filename.display(),
                )
            }
            Error::WriteFile { filename, backup_filename, .. } => {
                write!(
                    f,
                    "error writing buffer to \"{}\"",
                    filename.display(),
                )?;
                if let Some(backup_filename) = backup_filename {
                    write!(
                        f,
                        ", backup left in \"{}\"",
                        backup_filename.display()
                    )?;
                }
                Ok(())
            }
            Error::WriteFileOpen { filename, .. } => {
                write!(
                    f,
                    "error opening \"{}\" for writing",
                    filename.display()
                )
            }
            Error::WriteMakeBackup { filename, backup_filename, .. } => {
                write!(
                    f,
                    "error writing \"{}\" as backup of \"{}\"",
                    backup_filename
                        .as_ref()
                        .expect("backup path exists if this error produced")
                        .display(),
                    filename.display()
                )
            }
            Error::WriteRemoveBackup { backup_filename, .. } => {
                write!(
                    f,
                    "error removing \"{}\"",
                    backup_filename
                        .as_ref()
                        .expect("backup path exists if this error produced")
                        .display()
                )
            }
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum Warning {
    NewUnsaved,
    EditUnsaved(PathBuf),
    ReloadUnsaved,
    WriteOverwrite,
    QuitUnsaved,
    WriteAsOverwrite(Option<Range<usize>>, PathBuf),
}

impl Display for Warning {
    #[cfg(not(tarpaulin_include))]
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        match self {
            Warning::EditUnsaved(_)
            | Warning::ReloadUnsaved
            | Warning::NewUnsaved
            | Warning::QuitUnsaved => {
                write!(f, "unsaved changes - repeat command to discard changes")
            }
            Warning::WriteOverwrite => write!(
                f,
                "current file was altered externally - repeat command to overwrite with buffer contents",
            ),
            Warning::WriteAsOverwrite(span, file) => write!(
                f,
                "'{}' exists - repeat command to overwrite with{}buffer contents",
                file.display(),
                span.as_ref().map_or(" ", |_| " partial ")
            ),
        }
    }
}
#[derive(Debug, Copy, Clone, PartialEq)]
pub struct ParseEolError;

impl std::error::Error for ParseEolError {}

impl Display for ParseEolError {
    #[cfg(not(tarpaulin_include))]
    fn fmt(&self, f: &mut Formatter<'_>) -> Result {
        write!(f, "invalid prevailing EOL string")
    }
}
