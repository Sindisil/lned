use core::cmp;
use core::fmt::{self, Debug, Display, Formatter};
use std::io::{self};
use std::iter::Peekable;
use std::ops::RangeInclusive;
use std::path::PathBuf;

use regex::Regex;
use unicode_segmentation::Graphemes;
use unicode_segmentation::UnicodeSegmentation;

use line_reader::LineRead;
use line_reader::LineReaderOptions;

use crate::edit_buffer::EditBuffer;
use crate::iter_utils::Peeking;

#[derive(Debug)]
pub enum Cmd {
    Append(Option<Address>),
    Change(Option<Address>),
    Delete(Option<Address>),
    Edit(Option<PathBuf>),
    Enumerate(Option<Address>),
    File(Option<PathBuf>),
    Global(Option<Address>, Regex, String),
    Insert(Option<Address>),
    Join(Option<Address>),
    Move(Option<Address>, Address),
    Null(Option<Address>),
    Print(Option<Address>),
    Quit,
    Redo,
    Substitute(Option<Address>, Regex, String, SubstitutionScope),
    Transfer(Option<Address>, Address),
    Undo,
    Write(Option<Address>, Option<PathBuf>),
}

#[derive(Debug, Copy, Clone)]
pub enum SubstitutionScope {
    Single(usize),
    Global,
}

#[derive(Debug)]
pub enum PrintSuffix {
    Enumerate,
    Print,
}

#[derive(Debug)]
pub enum Error {
    Unknown(String),
    UnexpectedAddress,
    OffsetTooLarge,
    OffsetTooSmall,
    OffsetOverflow,
    InvalidAddress,
    Regex(regex::Error),
    NoMatchingLine,
    NoPreviousPattern,
    NumberParse,
    TrailingBackslash,
    InvalidPatternDelimiter,
    InvalidCmdSuffix,
    InvalidFilename,
    ReadCommand { source: io::Error },
    MissingEol,
    MissingDestination,
    RepeatedSubstitutionScope,
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub struct Address {
    start: usize,
    end: usize,
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match *self {
            Error::Unknown(_)
            | Error::UnexpectedAddress
            | Error::OffsetTooLarge
            | Error::OffsetTooSmall
            | Error::OffsetOverflow
            | Error::InvalidAddress
            | Error::Regex(_)
            | Error::NoMatchingLine
            | Error::NoPreviousPattern
            | Error::NumberParse
            | Error::TrailingBackslash
            | Error::InvalidPatternDelimiter
            | Error::InvalidCmdSuffix
            | Error::InvalidFilename
            | Error::MissingDestination
            | Error::RepeatedSubstitutionScope
            | Error::MissingEol => None,
            Error::ReadCommand { ref source } => Some(source),
        }
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Error::UnexpectedAddress => {
                write!(f, "command takes no line address.")
            }
            Error::Unknown(c) => write!(f, "unknown command '{c}'"),
            Error::OffsetTooLarge => write!(f, "offset too large"),
            Error::OffsetOverflow => {
                write!(f, "offset results in invalid line number")
            }
            Error::OffsetTooSmall => write!(f, "offset too small"),
            Error::InvalidAddress => write!(f, "invalid address"),
            Error::Regex(e) => write!(f, "{e}"),
            Error::NoMatchingLine => write!(f, "no matching line"),
            Error::TrailingBackslash => write!(f, "invalid trailing backslash"),
            Error::NoPreviousPattern => write!(f, "no previous pattern"),
            Error::InvalidPatternDelimiter => {
                write!(f, "invalid pattern delimiter")
            }
            Error::InvalidCmdSuffix => write!(f, "invalid command suffix"),
            Error::InvalidFilename => write!(f, "invalid filename"),
            Error::ReadCommand { .. } => {
                write!(f, "error reading command input")
            }
            Error::MissingEol => write!(f, "missing line terminator"),
            Error::MissingDestination => write!(f, "missing destination"),
            Error::NumberParse => write!(f, "invalid numeric string"),
            Error::RepeatedSubstitutionScope => {
                write!(f, "only one substitution scope specifier allowed")
            }
        }
    }
}

impl Address {
    pub fn span(start: usize, end: usize) -> Address {
        Address { start, end }
    }

    pub fn line(line: usize) -> Address {
        Address { start: line, end: line }
    }

    pub fn start(&self) -> usize {
        self.start
    }

    pub fn end(&self) -> usize {
        self.end
    }

    pub fn as_end(&self) -> Self {
        Self::line(self.end)
    }

    pub fn line_count(&self) -> usize {
        self.end - self.start + 1
    }

    fn eval(
        graphemes: &mut Peekable<Graphemes<'_>>,
        buffer: &mut EditBuffer,
        previous_pattern: &mut Option<Regex>,
    ) -> Result<Option<Address>, Error> {
        let mut left = None;
        let mut right = None;

        loop {
            match graphemes.peek() {
                Some(&",") => {
                    graphemes.next();
                    left = right.or(Some(1));
                    right = right.or_else(|| Some(buffer.len()));
                }
                Some(&";") => {
                    graphemes.next();
                    left = Some(match right {
                        Some(r) if r > buffer.len() => {
                            return Err(Error::InvalidAddress)
                        }
                        Some(r) => {
                            buffer.set_current_line(r);
                            r
                        }
                        None => buffer.current_line(),
                    });
                    right = right.or_else(|| Some(buffer.len()));
                }
                Some(&"+" | &"-") => {
                    right = Some(eval_line_number(
                        graphemes,
                        buffer.current_line(),
                    )?);
                }
                Some(&".") => {
                    graphemes.next();
                    right = Some(eval_line_number(
                        graphemes,
                        buffer.current_line(),
                    )?);
                }
                Some(&"$") => {
                    graphemes.next();
                    right = Some(eval_line_number(graphemes, buffer.len())?);
                }
                Some(&"/") => {
                    graphemes.next();
                    let (pattern, _) = parse_pattern(graphemes, Some("/"))?;
                    if !pattern.is_empty() {
                        *previous_pattern =
                            Some(Regex::new(&pattern).map_err(Error::Regex)?);
                    }
                    let re = previous_pattern
                        .as_ref()
                        .ok_or(Error::NoPreviousPattern)?;
                    let line =
                        buffer.find_line(re).ok_or(Error::NoMatchingLine)?;
                    right = Some(eval_line_number(graphemes, line)?);
                }
                Some(&"?") => {
                    graphemes.next();
                    let (pattern, _) = parse_pattern(graphemes, Some("?"))?;
                    if !pattern.is_empty() {
                        *previous_pattern =
                            Some(Regex::new(&pattern).map_err(Error::Regex)?);
                    }
                    let re = previous_pattern
                        .as_ref()
                        .ok_or(Error::NoPreviousPattern)?;
                    let line = buffer
                        .find_line_rev(re)
                        .ok_or(Error::NoMatchingLine)?;
                    right = Some(eval_line_number(graphemes, line)?);
                }
                Some(&" " | &"\t") => {
                    graphemes.next();
                }
                Some(s)
                    if s.chars().next().is_some_and(|c| c.is_ascii_digit()) =>
                {
                    let num = parse_number(graphemes)?;
                    right = Some(eval_line_number(graphemes, num)?);
                }
                Some(_) => break,
                None => return Err(Error::MissingEol),
            }
            if left.is_none() && right.is_some() {
                left = right;
            }
        }

        if let Some(right) = right {
            let left = left.map_or(right, |l| l);
            if left > right {
                Err(Error::InvalidAddress)
            } else {
                Ok(Some(Address::span(left, right)))
            }
        } else {
            Ok(None)
        }
    }
}

impl From<Address> for RangeInclusive<usize> {
    fn from(value: Address) -> Self {
        value.start()..=value.end()
    }
}

impl Cmd {
    // Read lines of input into buf, stopping when a '.' alone on a line
    // is read. Clears previous content of buf, but doesn't shrink capacity.
    // Returns number of bytes read or Error::Readlines if an error is
    // encountered.
    pub fn read_input_lines(
        input: &mut impl LineRead,
        buf: &mut Vec<String>,
    ) -> Result<usize, io::Error> {
        let text_read_options =
            LineReaderOptions { history: false, ..Default::default() };
        buf.clear();
        loop {
            let mut line = String::new();
            let n = input.read(&mut line, &text_read_options)?;
            if n == 0 || line == ".\n" || line == ".\r\n" {
                return Ok(buf.len());
            }
            buf.push(line);
        }
    }

    /// Read input, parsing into a Cmd
    pub fn read(
        input: &mut impl LineRead,
        buffer: &mut EditBuffer,
        previous_pattern: &mut Option<Regex>,
    ) -> Result<(Cmd, Option<PrintSuffix>), Error> {
        let cmd_read_options =
            LineReaderOptions { prompt: ":".into(), ..Default::default() };
        let mut line = String::with_capacity(120);
        input
            .read(&mut line, &cmd_read_options)
            .map_err(|source| Error::ReadCommand { source })?;
        let mut graphemes = line.as_str().graphemes(true).peekable();
        let address = Address::eval(&mut graphemes, buffer, previous_pattern)?;
        match graphemes.next() {
            Some("a") => parse_no_args(&mut graphemes, Cmd::Append(address)),
            Some("c") => parse_no_args(&mut graphemes, Cmd::Change(address)),
            Some("d") => parse_no_args(&mut graphemes, Cmd::Delete(address)),
            Some("e") => parse_edit_cmd(&mut graphemes, address),
            Some("f") => parse_file_cmd(&mut graphemes, address),
            Some("g") => parse_global_cmd(
                &mut graphemes,
                address,
                previous_pattern,
                input,
            ),
            Some("i") => parse_no_args(&mut graphemes, Cmd::Insert(address)),
            Some("j") => parse_no_args(&mut graphemes, Cmd::Join(address)),
            Some("m") => parse_move_cmd(
                &mut graphemes,
                buffer,
                previous_pattern,
                address,
            ),
            Some("n") => parse_no_args(&mut graphemes, Cmd::Enumerate(address)),
            None | Some("\n" | "\r\n") => Ok((Cmd::Null(address), None)),
            Some("p") => parse_no_args(&mut graphemes, Cmd::Print(address)),
            Some("q") => parse_no_address(address, Cmd::Quit)
                .and_then(|cmd| parse_no_args(&mut graphemes, cmd)),
            Some("s") => parse_substitute_cmd(
                &mut graphemes,
                address,
                previous_pattern,
                input,
            ),
            Some("t") => parse_transfer_cmd(
                &mut graphemes,
                buffer,
                previous_pattern,
                address,
            ),
            Some("u") => parse_no_address(address, Cmd::Undo)
                .and_then(|cmd| parse_no_args(&mut graphemes, cmd)),
            Some("U") => parse_no_address(address, Cmd::Redo)
                .and_then(|cmd| parse_no_args(&mut graphemes, cmd)),
            Some("w") => parse_write_cmd(&mut graphemes, address),
            Some(s) => Err(Error::Unknown(s.to_owned())),
        }
    }
}

fn parse_print_suffix<'a>(
    graphemes: &mut impl Iterator<Item = &'a str>,
) -> Result<Option<PrintSuffix>, Error> {
    let res = match graphemes.next() {
        None | Some("\n" | "\r\n") => Ok(None),
        Some("n") => Ok(Some(PrintSuffix::Enumerate)),
        Some("p") => Ok(Some(PrintSuffix::Print)),
        _ => Err(Error::InvalidCmdSuffix),
    };
    if res.is_err() || res.as_ref().is_ok_and(Option::is_none) {
        return res;
    }
    match graphemes.next() {
        None | Some("\n" | "\r\n") => res,
        _ => Err(Error::InvalidCmdSuffix),
    }
}

fn parse_write_cmd<'a>(
    graphemes: &mut impl Iterator<Item = &'a str>,
    address: Option<Address>,
) -> Result<(Cmd, Option<PrintSuffix>), Error> {
    match graphemes.next() {
        None | Some("\n" | "\r\n") => Ok((Cmd::Write(address, None), None)),
        Some(" " | "\t") => {
            let filename = graphemes
                .take_while(|s| *s != "\n" && *s != "\r\n")
                .collect::<String>()
                .trim()
                .to_owned();
            if filename.is_empty() {
                Err(Error::InvalidFilename)
            } else {
                Ok((Cmd::Write(address, Some(PathBuf::from(filename))), None))
            }
        }
        _ => Err(Error::InvalidCmdSuffix),
    }
}

fn parse_edit_cmd<'a>(
    graphemes: &mut impl Iterator<Item = &'a str>,
    address: Option<Address>,
) -> Result<(Cmd, Option<PrintSuffix>), Error> {
    if address.is_some() {
        return Err(Error::UnexpectedAddress);
    }
    match graphemes.next() {
        None | Some("\n" | "\r\n") => Ok((Cmd::Edit(None), None)),
        Some(" " | "\t") => {
            let filename = graphemes
                .take_while(|s| *s != "\n" && *s != "\r\n")
                .collect::<String>()
                .trim()
                .to_owned();
            if filename.is_empty() {
                Err(Error::InvalidFilename)
            } else {
                Ok((Cmd::Edit(Some(PathBuf::from(filename))), None))
            }
        }
        _ => Err(Error::InvalidCmdSuffix),
    }
}

fn parse_move_cmd(
    graphemes: &mut Peekable<Graphemes<'_>>,
    buffer: &mut EditBuffer,
    previous_pattern: &mut Option<Regex>,
    address: Option<Address>,
) -> Result<(Cmd, Option<PrintSuffix>), Error> {
    let Some(destination) = Address::eval(graphemes, buffer, previous_pattern)?
    else {
        return Err(Error::MissingDestination);
    };
    let sfx = parse_print_suffix(graphemes)?;
    Ok((Cmd::Move(address, destination), sfx))
}

fn parse_replacement_line(
    graphemes: &mut Peekable<Graphemes<'_>>,
    replacement: &mut String,
    delimiter: &str,
) -> Result<bool, Error> {
    Ok(loop {
        match graphemes.next() {
            None => break false,
            Some(gr) if gr == delimiter || gr == "\n" || gr == "\r\n" => {
                break false
            }
            Some("\\") => {
                let escaped =
                    graphemes.next().ok_or(Error::TrailingBackslash)?;
                if escaped == "\n" || escaped == "\r\n" {
                    replacement.push_str(escaped);
                    break true;
                }
                if escaped != delimiter {
                    replacement.push('\\');
                }
                replacement.push_str(escaped);
            }
            Some(gr) => replacement.push_str(gr),
        }
    })
}

fn parse_substitution_scope(
    graphemes: &mut Peekable<Graphemes<'_>>,
) -> Result<SubstitutionScope, Error> {
    let mut s: Option<SubstitutionScope> = None;
    while let Some(&gr) = graphemes.peek() {
        let is_digit = gr.chars().next().is_some_and(|c| c.is_ascii_digit());
        if s.is_some() && (is_digit || gr == "g") {
            return Err(Error::RepeatedSubstitutionScope);
        }
        if is_digit {
            s = Some(SubstitutionScope::Single(parse_number(graphemes)?));
        } else if gr == "g" {
            s = Some(SubstitutionScope::Global);
            graphemes.next();
        } else if gr == "\n" || gr == "\r\n" {
            break;
        } else {
            return Err(Error::InvalidCmdSuffix);
        }
    }
    Ok(s.unwrap_or(SubstitutionScope::Single(1)))
}

pub(crate) fn parse_substitute_cmd(
    graphemes: &mut Peekable<Graphemes<'_>>,
    address: Option<Address>,
    previous_pattern: &mut Option<Regex>,
    input: &mut impl LineRead,
) -> Result<(Cmd, Option<PrintSuffix>), Error> {
    let (pattern, delimiter) = parse_pattern(graphemes, None)?;
    if !(pattern.is_empty()) {
        *previous_pattern = Some(Regex::new(&pattern).map_err(Error::Regex)?);
    }
    let pattern = previous_pattern.clone().ok_or(Error::NoPreviousPattern)?;

    let mut replacement = String::new();
    let more_lines = parse_replacement_line(
        graphemes,
        &mut replacement,
        delimiter.as_str(),
    )?;

    if !more_lines {
        let scope = parse_substitution_scope(graphemes)?;
        let sfx = parse_print_suffix(graphemes)?;
        return Ok((
            Cmd::Substitute(address, pattern, replacement, scope),
            sfx,
        ));
    }

    let line_read_options =
        LineReaderOptions { prompt: "".into(), ..Default::default() };
    let mut line = String::new();
    let cmd = loop {
        input
            .read(&mut line, &line_read_options)
            .map_err(|source| Error::ReadCommand { source })?;
        let mut graphemes = line.graphemes(true).peekable();
        let more_lines = parse_replacement_line(
            &mut graphemes,
            &mut replacement,
            delimiter.as_str(),
        )?;
        if !more_lines {
            let scope = parse_substitution_scope(&mut graphemes)?;
            break Cmd::Substitute(address, pattern, replacement, scope);
        }
        line.clear();
    };
    let sfx = parse_print_suffix(graphemes)?;
    Ok((cmd, sfx))
}

fn parse_transfer_cmd(
    graphemes: &mut Peekable<Graphemes<'_>>,
    buffer: &mut EditBuffer,
    previous_pattern: &mut Option<Regex>,
    address: Option<Address>,
) -> Result<(Cmd, Option<PrintSuffix>), Error> {
    let destination = Address::eval(graphemes, buffer, previous_pattern)?;
    let Some(destination) = destination else {
        return Err(Error::MissingDestination);
    };
    parse_print_suffix(graphemes)?;
    Ok((Cmd::Transfer(address, destination), None))
}

fn eval_line_number(
    graphemes: &mut Peekable<Graphemes<'_>>,
    line: usize,
) -> Result<usize, Error> {
    let offset = compute_line_offset(graphemes)?;
    line.checked_add_signed(offset).ok_or(Error::OffsetOverflow)
}

fn parse_pattern(
    graphemes: &mut Peekable<Graphemes<'_>>,
    delimiter: Option<&str>,
) -> Result<(String, String), Error> {
    let delimiter = delimiter
        .map_or_else(
            || {
                graphemes
                    .next_if(|gr| *gr != "\n" && *gr != "\r\n" && *gr != " ")
                    .ok_or(Error::InvalidPatternDelimiter)
            },
            Ok,
        )?
        .to_owned();
    let mut text = String::new();
    while let Some(gr) = graphemes.next_if(|gr| *gr != "\n" && *gr != "\r\n") {
        match gr {
            gr if gr == delimiter => break,
            "\\" => {
                let escaped_gr = graphemes
                    .next_if(|gr| *gr != "\n" && *gr != "\r\n")
                    .ok_or(Error::TrailingBackslash)?;
                if escaped_gr != delimiter {
                    text.push('\\');
                }
                text.push_str(escaped_gr);
            }
            gr => text.push_str(gr),
        }
    }
    Ok((text, delimiter))
}

fn compute_line_offset(
    graphemes: &mut Peekable<Graphemes<'_>>,
) -> Result<isize, Error> {
    let mut total_offset = 0isize;
    while let Some(s) = graphemes.peek() {
        match *s {
            " " | "\t" => {
                graphemes.next();
            }
            s if s.chars().next().is_some_and(|c| c.is_ascii_digit()) => {
                total_offset = parse_number(graphemes)
                    .and_then(|o| {
                        o.try_into().map_err(|_| Error::OffsetTooLarge)
                    })
                    .and_then(|o| {
                        total_offset.checked_add(o).ok_or(Error::OffsetTooLarge)
                    })
                    .map_err(|_| Error::OffsetTooLarge)?;
            }
            "+" => {
                graphemes.next();
                total_offset = parse_number(graphemes)
                    .map_err(|_| Error::OffsetTooLarge)
                    .and_then(|o| {
                        o.try_into().map_err(|_| Error::OffsetTooLarge)
                    })
                    .and_then(|o| {
                        total_offset
                            .checked_add(cmp::max(1, o))
                            .ok_or(Error::OffsetOverflow)
                    })?;
            }
            "-" => {
                graphemes.next();
                total_offset = parse_number(graphemes)
                    .map_err(|_| Error::OffsetTooSmall)
                    .and_then(|o| {
                        o.try_into().map_err(|_| Error::OffsetTooSmall)
                    })
                    .and_then(|o| {
                        total_offset
                            .checked_sub(cmp::max(1, o))
                            .ok_or(Error::OffsetOverflow)
                    })?;
            }

            _ => break,
        }
    }
    Ok(total_offset)
}

fn parse_no_address(address: Option<Address>, cmd: Cmd) -> Result<Cmd, Error> {
    address.map_or(Ok(cmd), |_| Err(Error::UnexpectedAddress))
}

fn parse_no_args(
    graphemes: &mut Peekable<Graphemes<'_>>,
    cmd: Cmd,
) -> Result<(Cmd, Option<PrintSuffix>), Error> {
    Ok((cmd, parse_print_suffix(graphemes)?))
}

fn parse_number<'a>(
    graphemes: &mut Peekable<impl Iterator<Item = &'a str>>,
) -> Result<usize, Error> {
    graphemes
        .peeking_take_while(|s| {
            s.chars().next().is_some_and(|c| c.is_ascii_digit())
        })
        .try_fold(0usize, |acc, s| {
            s.chars().next().and_then(|c| c.to_digit(10)).and_then(|d| {
                acc.checked_mul(10).and_then(|n| n.checked_add(d as usize))
            })
        })
        .ok_or(Error::NumberParse)
}

fn parse_file_cmd<'a>(
    graphemes: &mut impl Iterator<Item = &'a str>,
    address: Option<Address>,
) -> Result<(Cmd, Option<PrintSuffix>), Error> {
    if address.is_some() {
        return Err(Error::UnexpectedAddress);
    }
    match graphemes.next() {
        None | Some("\n" | "\r\n") => Ok((Cmd::File(None), None)),
        Some(" " | "\t") => {
            let filename = graphemes
                .take_while(|s| *s != "\n" && *s != "\r\n")
                .collect::<String>()
                .trim()
                .to_owned();
            if filename.is_empty() {
                Err(Error::InvalidFilename)
            } else {
                Ok((Cmd::File(Some(PathBuf::from(filename))), None))
            }
        }
        _ => Err(Error::InvalidCmdSuffix),
    }
}

fn parse_global_cmd(
    graphemes: &mut Peekable<Graphemes<'_>>,
    address: Option<Address>,
    previous_pattern: &mut Option<Regex>,
    input: &mut impl LineRead,
) -> Result<(Cmd, Option<PrintSuffix>), Error> {
    let (pattern, _) = parse_pattern(graphemes, None)?;
    if !(pattern.is_empty()) {
        *previous_pattern = Some(Regex::new(&pattern).map_err(Error::Regex)?);
    }
    let pattern = previous_pattern.clone().ok_or(Error::NoPreviousPattern)?;

    let mut commands = String::new();
    let mut more_lines = false;

    // Copy first command to commands string,
    // noting and unescaping escaped EOL.
    while let Some(gr) = graphemes.next() {
        if gr == "\\" && matches!(graphemes.peek(), Some(&"\n" | &"\r\n")) {
            more_lines = true;
        } else {
            commands.push_str(gr);
            if gr == "\n" || gr == "\r\n" {
                break;
            }
        }
    }

    // if the EOL was escaped, use read_input_lines() to read in rest of command list
    if more_lines {
        let mut lines = Vec::new();
        if Cmd::read_input_lines(input, &mut lines)
            .map_err(|source| Error::ReadCommand { source })?
            > 0
        {
            for line in lines {
                commands.push_str(&line);
            }
        }
    }

    Ok((Cmd::Global(address, pattern, commands), None))
}

#[cfg(test)]
mod tests {
    use super::*;

    use similar_asserts::assert_eq;

    #[test]
    fn parse_valid_lone_cmd() {
        let res = parse_no_address(None, Cmd::Quit).unwrap();
        assert!(matches!(res, Cmd::Quit));
    }

    #[test]
    fn parse_no_address_error_with_address() {
        let res = parse_no_address(Some(Address::line(1)), Cmd::Quit)
            .expect_err("unexpected address");
        assert!(matches!(res, Error::UnexpectedAddress));
    }

    #[test]
    fn parse_no_args_error_with_extra_chars() {
        let mut cmd_line = "extra\n".graphemes(true).peekable();
        let res = parse_no_args(&mut cmd_line, Cmd::Delete(None))
            .expect_err("invalid suffix");
        assert!(matches!(res, Error::InvalidCmdSuffix));
    }

    #[test]
    fn parse_no_args_both_line_terminators_valid() {
        let mut cmd_line = "\n".graphemes(true).peekable();
        let (cmd, sfx) =
            parse_no_args(&mut cmd_line, Cmd::Delete(None)).unwrap();
        assert!(matches!(cmd, Cmd::Delete(None)));
        assert!(sfx.is_none());
    }

    #[test]
    fn parse_no_args_p_print_suffix() {
        let mut cmd_line = "p\r\n".graphemes(true).peekable();
        let (_cmd, _sfx) =
            parse_no_args(&mut cmd_line, Cmd::Delete(None)).unwrap();
        assert!(matches!(Cmd::Delete(None), _cmd));
        assert!(matches!(Some(PrintSuffix::Enumerate), _sfx));
    }

    #[test]
    fn parse_no_args_n_print_suffix() {
        let mut cmd_line = "n\r\n".graphemes(true).peekable();
        let (_cmd, _sfx) =
            parse_no_args(&mut cmd_line, Cmd::Delete(None)).unwrap();
        assert!(matches!(Cmd::Delete(None), _cmd));
        assert!(matches!(Some(PrintSuffix::Enumerate), _sfx));
    }

    #[test]
    fn parse_no_args_extra_chars_after_print_suffix_error() {
        let mut cmd_line = "n!\r\n".graphemes(true).peekable();
        let _res = parse_no_args(&mut cmd_line, Cmd::Delete(None))
            .expect_err("invalid suffix");
        assert!(matches!(Error::InvalidCmdSuffix, _res));
    }

    #[test]
    fn parse_no_args_multiple_print_suffix_error() {
        let mut cmd_line = "pn\r\n".graphemes(true).peekable();
        let _res = parse_no_args(&mut cmd_line, Cmd::Delete(None))
            .expect_err("invalid suffix");
        assert!(matches!(Error::InvalidCmdSuffix, _res));
    }

    #[test]
    fn parse_print_suffix_p() {
        let mut graphs = "p\r\n".graphemes(true).peekable();
        let _res = parse_print_suffix(&mut graphs).unwrap();
        assert!(matches!(Some(PrintSuffix::Print), _res));
    }

    #[test]
    fn parse_print_suffix_n() {
        let mut graphs = "n\r\n".graphemes(true).peekable();
        let _res = parse_print_suffix(&mut graphs).unwrap();
        assert!(matches!(Some(PrintSuffix::Enumerate), _res));
    }

    #[test]
    fn parse_print_suffix_trailing_chars_error() {
        let mut graphs = "pn\r\n".graphemes(true).peekable();
        let _res = parse_print_suffix(&mut graphs).expect_err("invalid suffix");
        assert!(matches!(Error::InvalidCmdSuffix, _res));
    }

    #[test]
    fn eval_no_addr_null_cmd() {
        let mut cmd_line = "\r\n".graphemes(true).peekable();
        let address =
            Address::eval(&mut cmd_line, &mut EditBuffer::new(), &mut None)
                .unwrap();
        assert!(address.is_none());
        assert!(matches!(cmd_line.next(), Some("\r\n")));
        let mut cmd_line = "\n".graphemes(true).peekable();
        let address =
            Address::eval(&mut cmd_line, &mut EditBuffer::new(), &mut None)
                .unwrap();
        assert!(address.is_none());
        assert!(matches!(cmd_line.next(), Some("\n")));
    }

    #[test]
    fn eval_no_addr_null_cmd_skip_spaces() {
        let mut cmd_line = "\t  \r\n".graphemes(true).peekable();
        let address =
            Address::eval(&mut cmd_line, &mut EditBuffer::new(), &mut None)
                .unwrap();
        assert!(address.is_none());
        assert!(matches!(cmd_line.next(), Some("\r\n")));
        let mut cmd_line = "\n".graphemes(true).peekable();
        let address =
            Address::eval(&mut cmd_line, &mut EditBuffer::new(), &mut None)
                .unwrap();
        assert!(address.is_none());
        assert!(matches!(cmd_line.next(), Some("\n")));
    }

    #[test]
    fn eval_positive_offset() {
        let mut input = "3p".graphemes(true).peekable();
        let res = compute_line_offset(&mut input).unwrap();
        assert_eq!(res, 3);
        assert!(matches!(input.next(), Some("p")));
        let mut input = "+42p".graphemes(true).peekable();
        let res = compute_line_offset(&mut input).unwrap();
        assert_eq!(res, 42);
        assert!(matches!(input.next(), Some("p")));
    }

    #[test]
    fn eval_negative_offsets() {
        let mut input = "-2p".graphemes(true).peekable();
        let res = compute_line_offset(&mut input).unwrap();
        assert_eq!(res, -2);
        assert!(matches!(input.next(), Some("p")));
    }

    #[test]
    fn eval_mixed_offsets() {
        let mut input = "2-7+6p".graphemes(true).peekable();
        let res = compute_line_offset(&mut input).unwrap();
        assert_eq!(res, 1);
        assert!(matches!(input.next(), Some("p")));
    }

    #[test]
    fn eval_offset_overflow() {
        let mut input =
            "8399999999999999999+839999999999999999+8399999999999999999p"
                .graphemes(true)
                .peekable();
        let res = compute_line_offset(&mut input).expect_err("shouldn't parse");
        assert!(matches!(res, Error::OffsetOverflow));

        let mut input =
            "-839999999999999999-83999999999999999-8399999999999999999p"
                .graphemes(true)
                .peekable();
        let res = compute_line_offset(&mut input).expect_err("shouldn't parse");
        assert!(matches!(res, Error::OffsetOverflow));
    }

    #[test]
    fn eval_offset_too_large() {
        let mut input = "999999999999999999999p".graphemes(true).peekable();
        let res = compute_line_offset(&mut input).expect_err("shouldn't parse");
        assert!(matches!(res, Error::OffsetTooLarge));
        let mut input = "+999999999999999999999p".graphemes(true).peekable();
        let res = compute_line_offset(&mut input).expect_err("shouldn't parse");
        assert!(matches!(res, Error::OffsetTooLarge));
    }

    #[test]
    fn eval_offset_too_small() {
        let mut input = "-999999999999999999999p".graphemes(true).peekable();
        let res = compute_line_offset(&mut input).expect_err("shouldn't parse");
        assert!(matches!(res, Error::OffsetTooSmall));
    }

    #[test]
    fn eval_mixed_offsets_with_spaces() {
        let mut input = "   2 -7  6 +1p".graphemes(true).peekable();
        let res = compute_line_offset(&mut input).unwrap();
        assert_eq!(res, 2);
        assert!(matches!(input.next(), Some("p")));
    }

    #[test]
    fn parse_pattern_delimiter_invalid() {
        let mut input = " stuff + other_stuff. \n".graphemes(true).peekable();
        let res = parse_pattern(&mut input, None);
        assert!(matches!(res, Err(Error::InvalidPatternDelimiter)));
    }

    #[test]
    fn parse_pattern_trailing_backslash() {
        let mut input = "/stuff + other_stuff.\\\n".graphemes(true).peekable();
        let res =
            parse_pattern(&mut input, None).expect_err("trailing backslash");
        assert!(matches!(res, Error::TrailingBackslash));
        let mut input = "/stuff + other_stuff.\\".graphemes(true).peekable();
        let res =
            parse_pattern(&mut input, None).expect_err("trailing backslash");
        assert!(matches!(res, Error::TrailingBackslash));
    }

    #[test]
    fn parse_pattern_no_terminating_delimiter() {
        let mut input = "/stuff\\/other_stuff.\n".graphemes(true).peekable();
        let (pattern, _) = parse_pattern(&mut input, None).unwrap();
        assert_eq!("stuff/other_stuff.".to_owned(), pattern);
    }

    #[test]
    fn parse_pattern_escaped_terminator() {
        let mut input = "/stuff\\/other_stuff./\n".graphemes(true).peekable();
        let (res, _) = parse_pattern(&mut input, None).unwrap();
        assert_eq!("stuff/other_stuff.".to_owned(), res);
    }

    #[test]
    fn parse_pattern_escaped_chars() {
        let mut input =
            "?stuff \\+ other_stuff\\.?\n".graphemes(true).peekable();
        let (res, _) = parse_pattern(&mut input, None).unwrap();
        assert_eq!("stuff \\+ other_stuff\\.".to_owned(), res);
    }

    #[test]
    fn parse_pattern_no_escaped_chars() {
        let mut input = "/stuff + other_stuff./\n".graphemes(true).peekable();
        let (res, _) = parse_pattern(&mut input, None).unwrap();
        assert_eq!("stuff + other_stuff.".to_owned(), res);
    }

    #[test]
    fn eval_addr_no_eol() {
        let mut cmd_line = "".graphemes(true).peekable();
        let res =
            Address::eval(&mut cmd_line, &mut EditBuffer::new(), &mut None)
                .expect_err("mising line terminator");
        assert!(matches!(res, Error::MissingEol));
    }

    #[test]
    fn eval_no_addr() {
        let mut cmd_line = "q\n".graphemes(true).peekable();
        let address =
            Address::eval(&mut cmd_line, &mut EditBuffer::new(), &mut None)
                .unwrap();
        assert!(address.is_none());
        assert_eq!(cmd_line.next(), Some("q"));
    }

    #[test]
    fn eval_dot_addr() {
        let mut cmd_line = ".d\r\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3"]);
        buffer.set_current_line(2);
        let address =
            Address::eval(&mut cmd_line, &mut buffer, &mut None).unwrap();
        assert_eq!(address, Some(Address::line(2)));
        assert_eq!(cmd_line.next(), Some("d"));
    }

    #[test]
    fn eval_dollar_addr() {
        let mut cmd_line = "$d\r\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3"]);
        buffer.set_current_line(2);
        let address =
            Address::eval(&mut cmd_line, &mut buffer, &mut None).unwrap();
        assert_eq!(address, Some(Address::line(3)));
        assert_eq!(cmd_line.next(), Some("d"));
    }

    #[test]
    fn eval_simple_number_addr() {
        let mut cmd_line = "42d\n".graphemes(true).peekable();
        let address =
            Address::eval(&mut cmd_line, &mut EditBuffer::new(), &mut None)
                .unwrap();
        assert_eq!(cmd_line.next(), Some("d"));
        assert_eq!(address, Some(Address::line(42)));
    }

    #[test]
    fn regex_line_addr_regex_syntax() {
        let mut input = "/\\lo.+/n\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec![
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        buffer.set_current_line(2);
        let mut previous_pattern: Option<Regex> = None;
        let res = Address::eval(&mut input, &mut buffer, &mut previous_pattern)
            .expect_err("bad pattern");
        assert!(matches!(res, Error::Regex(_)));
    }

    #[test]
    fn rev_regex_line_addr_regex_syntax() {
        let mut input = "?\\lo.+?n\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec![
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        buffer.set_current_line(2);
        let mut previous_pattern: Option<Regex> = None;
        let res = Address::eval(&mut input, &mut buffer, &mut previous_pattern)
            .expect_err("bad pattern");
        assert!(matches!(res, Error::Regex(_)));
    }

    #[test]
    fn regex_line_addr_embedded_delim() {
        let mut input = "/o.+\\//n\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec![
            "one/\n", "two", "three", "four", "five", "six",
        ]);
        buffer.set_current_line(2);
        let mut previous_pattern: Option<Regex> = None;
        let res = Address::eval(&mut input, &mut buffer, &mut previous_pattern)
            .unwrap();
        assert_eq!(res, Some(Address::line(1)));
    }

    #[test]
    fn regex_line_addr_no_final_delimiter() {
        let mut input = "/o.+\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec![
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        buffer.set_current_line(2);
        let mut previous_pattern: Option<Regex> = None;
        let res = Address::eval(&mut input, &mut buffer, &mut previous_pattern)
            .unwrap();
        assert_eq!(res, Some(Address::line(4)));
    }

    #[test]
    fn regex_line_addr_needle_in_first_half_of_split_range() {
        let mut input = "/o.+/n\n".graphemes(true).peekable();
        let mut previous_pattern: Option<Regex> = None;
        let mut buffer = EditBuffer::from(vec![
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        buffer.set_current_line(2);
        let res = Address::eval(&mut input, &mut buffer, &mut previous_pattern)
            .unwrap();
        assert_eq!(res, Some(Address::line(4)));
    }

    #[test]
    fn regex_line_addr_needle_in_second_half_of_split_range() {
        let mut input = "/on.+/n\n".graphemes(true).peekable();
        let mut previous_pattern: Option<Regex> = None;
        let mut buffer = EditBuffer::from(vec![
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        buffer.set_current_line(4);
        let res = Address::eval(&mut input, &mut buffer, &mut previous_pattern)
            .unwrap();
        assert_eq!(res, Some(Address::line(1)));
    }

    #[test]
    fn regex_line_addr_contiguous_search_range() {
        let mut input = "/o.+/n\n".graphemes(true).peekable();
        let mut previous_pattern: Option<Regex> = None;
        let mut buffer = EditBuffer::from(vec![
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        buffer.set_current_line(6);
        let res = Address::eval(&mut input, &mut buffer, &mut previous_pattern)
            .unwrap();
        assert_eq!(res, Some(Address::line(1)));
    }

    #[test]
    fn rev_regex_line_addr_needle_in_first_half_of_split_range() {
        let mut input = "?o.+?n\n".graphemes(true).peekable();
        let mut previous_pattern: Option<Regex> = None;
        let mut buffer = EditBuffer::from(vec![
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        buffer.set_current_line(2);
        let res = Address::eval(&mut input, &mut buffer, &mut previous_pattern)
            .unwrap();
        assert_eq!(res, Some(Address::line(1)));
    }

    #[test]
    fn rev_regex_line_addr_needle_in_second_half_of_split_range() {
        let mut input = "?ou.+?n\n".graphemes(true).peekable();
        let mut previous_pattern: Option<Regex> = None;
        let mut buffer = EditBuffer::from(vec![
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        buffer.set_current_line(4);
        let res = Address::eval(&mut input, &mut buffer, &mut previous_pattern)
            .unwrap();
        assert_eq!(res, Some(Address::line(4)));
    }

    #[test]
    fn rev_regex_line_addr_contiguous_search_range() {
        let mut input = "?o.+?n\n".graphemes(true).peekable();
        let mut previous_pattern: Option<Regex> = None;
        let mut buffer = EditBuffer::from(vec![
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        buffer.set_current_line(1);
        let res = Address::eval(&mut input, &mut buffer, &mut previous_pattern)
            .unwrap();
        assert_eq!(res, Some(Address::line(4)));
    }

    #[test]
    fn regex_line_addr_with_offset() {
        let mut input = "/o.+/+2\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec![
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        buffer.set_current_line(2);
        let mut previous_pattern: Option<Regex> = None;
        let res = Address::eval(&mut input, &mut buffer, &mut previous_pattern)
            .unwrap();
        assert_eq!(res, Some(Address::line(6)));
    }

    #[test]
    fn rev_regex_line_addr_with_offset() {
        let mut input = "?o.+?+2\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec![
            "one\r\n", "two", "three", "four", "five", "six",
        ]);
        buffer.set_current_line(2);
        let mut previous_pattern: Option<Regex> = None;
        let res = Address::eval(&mut input, &mut buffer, &mut previous_pattern)
            .unwrap();
        assert_eq!(res, Some(Address::line(3)));
    }

    #[test]
    fn eval_simple_comma_addr() {
        let mut input = "1,2p\n".graphemes(true).peekable();
        let res = Address::eval(&mut input, &mut EditBuffer::new(), &mut None)
            .unwrap();
        assert_eq!(res, Some(Address::span(1, 2)));
        assert_eq!(input.next(), Some("p"));
    }

    #[test]
    fn eval_leading_comma_addr() {
        let mut input = ",4p\r\n".graphemes(true).peekable();
        let res = Address::eval(&mut input, &mut EditBuffer::new(), &mut None)
            .unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address::span(1, 4)));
    }

    #[test]
    fn eval_trailing_comma_addr() {
        let mut input = "5,p\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address::line(5)));
    }

    #[test]
    fn eval_comma_only_addr() {
        let mut input = ",p\r\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address::span(1, 6)));
    }

    #[test]
    fn eval_comma_only_chain_addr() {
        let mut input = ",,p\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address::line(6)));
    }

    #[test]
    fn eval_comma_chain_addr() {
        let mut input = ",12, 3+1,p\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address::line(4)));
    }

    #[test]
    fn eval_semicolon_addr_past_end() {
        let mut input = "+;np\n".graphemes(true).peekable();
        let mut buffer =
            EditBuffer::from(vec!["1\r\n", "2", "3", "4", "5", "6"]);
        assert_eq!(buffer.current_line(), 6);
        let res = Address::eval(&mut input, &mut buffer, &mut None)
            .expect_err("invalid address");
        assert!(matches!(res, Error::InvalidAddress));
    }

    #[test]
    fn eval_simple_semicolon_addr() {
        let mut input = "1;2p\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        assert_eq!(buffer.current_line(), 6);
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(res, Some(Address::span(1, 2)));
        assert_eq!(buffer.current_line(), 1);
        assert_eq!(input.next(), Some("p"));
    }

    #[test]
    fn eval_leading_semicolon_addr() {
        let mut input = ";5p\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(3);
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address::span(3, 5)));
        assert_eq!(buffer.current_line(), 3);
    }

    #[test]
    fn eval_trailing_semicolon_addr() {
        let mut input = "5;p\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(3);
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address::line(5)));
        assert_eq!(buffer.current_line(), 5);
    }

    #[test]
    fn eval_semicolon_only_addr() {
        let mut input = ";p\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(3);
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address::span(3, 6)));
        assert_eq!(buffer.current_line(), 3);
    }

    #[test]
    fn eval_semicolon_only_chain_addr() {
        let mut input = ";;p\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(3);
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address::line(6)));
    }

    #[test]
    fn eval_big_before_small_semicolon_chain_addr() {
        let mut input = "4;$;2p\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(3);
        let res = Address::eval(&mut input, &mut buffer, &mut None)
            .expect_err("invalid address");
        assert!(matches!(res, Error::InvalidAddress));
    }

    #[test]
    fn eval_simple_offset_only_addrs() {
        let mut input = "+p\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(3);
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address::line(4)));

        let mut input = "+10p\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(3);
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address::line(13)));

        let mut input = "-p\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(3);
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address::line(2)));

        let mut input = "-2p\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(3);
        let res = Address::eval(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address::line(1)));
    }

    #[test]
    fn eval_too_big_offset_only_addr_overflows() {
        let mut input = "-10p\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(3);
        let res = Address::eval(&mut input, &mut buffer, &mut None)
            .expect_err("offset overflow");
        assert!(matches!(res, Error::OffsetOverflow));
    }

    #[test]
    fn parse_append_cmd_no_addr() {
        let mut input = "a\r\n".as_bytes();
        let (cmd, sfx) =
            Cmd::read(&mut input, &mut EditBuffer::new(), &mut None).unwrap();
        assert!(matches!(cmd, Cmd::Append(None)));
        assert!(sfx.is_none());
    }

    #[test]
    fn parse_delete_cmd_no_addr() {
        let mut input = "d\r\n".as_bytes();
        let (cmd, sfx) =
            Cmd::read(&mut input, &mut EditBuffer::new(), &mut None).unwrap();
        assert!(
            matches!(cmd, Cmd::Delete(None)),
            "{cmd:?} didn't match Cmd::Delete(None)"
        );
        assert!(sfx.is_none());
    }

    #[test]
    fn parse_enumerate_cmd_no_addr() {
        let mut input = "n\r\n".as_bytes();
        let (cmd, sfx) =
            Cmd::read(&mut input, &mut EditBuffer::new(), &mut None).unwrap();
        assert!(
            matches!(cmd, Cmd::Enumerate(None)),
            "{cmd:?} didn't match Cmd::Enumerate(None)"
        );
        assert!(sfx.is_none());
    }

    #[test]
    fn parse_insert_cmd_no_addr() {
        let mut input = "i\r\n".as_bytes();
        let (cmd, sfx) =
            Cmd::read(&mut input, &mut EditBuffer::new(), &mut None).unwrap();
        assert!(matches!(cmd, Cmd::Insert(None)));
        assert!(sfx.is_none());
    }

    #[test]
    fn parse_null_cmd_no_addr() {
        let mut input = "\r\n".as_bytes();
        let (cmd, sfx) =
            Cmd::read(&mut input, &mut EditBuffer::new(), &mut None).unwrap();
        assert!(matches!(cmd, Cmd::Null(None)));
        assert!(sfx.is_none());
    }

    #[test]
    fn parse_print_cmd_no_addr() {
        let mut input = "p\r\n".as_bytes();
        let (cmd, sfx) =
            Cmd::read(&mut input, &mut EditBuffer::new(), &mut None).unwrap();
        assert!(
            matches!(cmd, Cmd::Print(None)),
            "{cmd:?} didn't match Cmd::Print(None)"
        );
        assert!(sfx.is_none());
    }

    #[test]
    fn parse_quit_cmd() {
        let mut input = "q\r\n".as_bytes();
        let (cmd, sfx) =
            Cmd::read(&mut input, &mut EditBuffer::new(), &mut None).unwrap();
        assert!(matches!(cmd, Cmd::Quit), "{cmd:?} didn't match Cmd::Quit");
        assert!(sfx.is_none());
    }

    #[test]
    fn parse_undo_cmd() {
        let mut input = "u\r\n".as_bytes();
        let (cmd, sfx) =
            Cmd::read(&mut input, &mut EditBuffer::new(), &mut None).unwrap();
        assert!(matches!(cmd, Cmd::Undo), "{cmd:?} didn't match Cmd::Undo");
        assert!(sfx.is_none());
    }

    #[test]
    fn parse_redo_cmd() {
        let mut input = "U\r\n".as_bytes();
        let (cmd, sfx) =
            Cmd::read(&mut input, &mut EditBuffer::new(), &mut None).unwrap();
        assert!(matches!(cmd, Cmd::Redo), "{cmd:?} didn't match Cmd::Redo");
        assert!(sfx.is_none());
    }

    #[test]
    fn parse_quit_cmd_invalid_suffix() {
        let mut input = "q/more stuff/\r\n".as_bytes();
        let res = Cmd::read(&mut input, &mut EditBuffer::new(), &mut None)
            .expect_err("invalid sufix");
        assert!(
            matches!(res, Error::InvalidCmdSuffix),
            "{res:?} didn't match Error::InvalidCmdSuffix"
        );
    }

    #[test]
    fn parse_unknown_command() {
        let mut input = "O\n".as_bytes();
        let res = Cmd::read(&mut input, &mut EditBuffer::new(), &mut None)
            .expect_err("unknown cmd");
        assert!(
            matches!(res, Error::Unknown(ref s) if s == "O"),
            "{res:?} didn't match Error::Unknown(\"O\")"
        );
    }

    #[test]
    fn parse_edit_no_print_suffix() {
        let mut cmd_line = " filename.rs".graphemes(true).peekable();
        let (_, sfx) = parse_edit_cmd(&mut cmd_line, None).unwrap();
        assert!(sfx.is_none());
    }

    #[test]
    fn parse_edit_with_address() {
        let mut cmd_line = " filename.rs".graphemes(true).peekable();
        let res = parse_edit_cmd(&mut cmd_line, Some(Address::line(1)))
            .expect_err("unexpected addr");
        assert!(matches!(res, Error::UnexpectedAddress));
    }

    #[test]
    fn parse_edit_no_filename() {
        let mut cmd_line = "\n".graphemes(true);
        let (cmd, sfx) = parse_edit_cmd(&mut cmd_line, None).unwrap();
        assert!(matches!(cmd, Cmd::Edit(None)));
        assert!(sfx.is_none());
    }

    #[test]
    fn parse_edit_bad_filename() {
        let mut cmd_line = " \r\n".graphemes(true);
        let res =
            parse_edit_cmd(&mut cmd_line, None).expect_err("bad filename");
        assert!(matches!(res, Error::InvalidFilename));
    }

    #[test]
    fn parse_edit_with_filename() {
        let mut cmd_line = " a/filename.rs\r\n".graphemes(true);
        let (cmd, sfx) = parse_edit_cmd(&mut cmd_line, None).unwrap();
        assert!(
            matches!(&cmd, Cmd::Edit(Some(f)) if f.to_str().unwrap() == "a/filename.rs")
        );
        assert!(sfx.is_none());
    }

    #[test]
    fn parse_edit_invalid_suffix() {
        let mut cmd_line = "filename.rs\n".graphemes(true);
        let res =
            parse_edit_cmd(&mut cmd_line, None).expect_err("invalid suffix");
        assert!(matches!(res, Error::InvalidCmdSuffix));
    }

    #[test]
    fn parse_file_no_print_suffix() {
        let mut cmd_line = "\n".graphemes(true);
        let (_, sfx) = parse_file_cmd(&mut cmd_line, None).unwrap();
        assert!(sfx.is_none());
    }

    #[test]
    fn parse_file_cmd_with_address() {
        let mut cmd_line = " filename.rs".graphemes(true);
        let res = parse_file_cmd(&mut cmd_line, Some(Address::line(1)))
            .expect_err("unexpected addr");
        assert!(matches!(res, Error::UnexpectedAddress));
    }

    #[test]
    fn parse_file_cmd_no_filename() {
        let mut cmd_line = "\n".graphemes(true);
        let (cmd, sfx) = parse_file_cmd(&mut cmd_line, None).unwrap();
        assert!(matches!(cmd, Cmd::File(None)));
        assert!(sfx.is_none());
    }

    #[test]
    fn parse_file_cmd_bad_filename() {
        let mut cmd_line = " \r\n".graphemes(true);
        let res =
            parse_file_cmd(&mut cmd_line, None).expect_err("bad filename");
        assert!(matches!(res, Error::InvalidFilename));
    }

    #[test]
    fn parse_file_cmd_with_filename() {
        let mut cmd_line = " a/filename.rs\r\n".graphemes(true);
        let (cmd, sfx) = parse_file_cmd(&mut cmd_line, None).unwrap();
        assert!(
            matches!(&cmd, Cmd::File(Some(f)) if f.to_str().unwrap() == "a/filename.rs"),
            "{cmd:?} wasnt Cmd::File(Some('filename.rs'))"
        );
        assert!(sfx.is_none());
    }

    #[test]
    fn parse_file_cmd_invalid_suffix() {
        let mut cmd_line = "filename.rs\n".graphemes(true);
        let res =
            parse_file_cmd(&mut cmd_line, None).expect_err("invalid suffix");
        assert!(matches!(res, Error::InvalidCmdSuffix));
    }

    #[test]
    fn parse_simple_global_cmd() {
        let mut input = "/pat/p\r\n".graphemes(true).peekable();
        let mut prev_pattern = None;
        let (cmd, sfx) = parse_global_cmd(
            &mut input,
            None,
            &mut prev_pattern,
            &mut "".as_bytes(),
        )
        .unwrap();
        assert!(matches!(cmd,
            Cmd::Global(a, p, c) if a.is_none() && p.as_str() == "pat" && c == "p\r\n"));
        assert!(sfx.is_none());
    }

    #[test]
    fn parse_multi_global_cmd() {
        let mut input = "/pat/n\\\r\n".graphemes(true).peekable();
        let mut more_input = "d\r\n".as_bytes();
        let mut prev_pattern = None;
        let (cmd, sfx) = parse_global_cmd(
            &mut input,
            None,
            &mut prev_pattern,
            &mut more_input,
        )
        .unwrap();
        assert!(matches!(cmd,
            Cmd::Global(a, p, c) if a.is_none() && p.as_str() == "pat" && c == "n\r\nd\r\n"));
        assert!(sfx.is_none());
    }

    #[test]
    fn parse_single_substitute() {
        let mut cmd_line = "/[^01]*/./\r\n".graphemes(true).peekable();
        let address = Some(Address::span(1, 10));
        let mut prev_pattern = None;
        let (cmd, sfx) = parse_substitute_cmd(
            &mut cmd_line,
            address,
            &mut prev_pattern,
            &mut "".as_bytes(),
        )
        .unwrap();
        let Cmd::Substitute(a, p, r, SubstitutionScope::Single(1)) = cmd else {
            panic!("not Single(1)!");
        };
        assert_eq!(a, address);
        assert_eq!(p.as_str(), "[^01]*");
        assert_eq!(r, ".");
        assert!(sfx.is_none());
    }

    #[test]
    fn parse_global_substitute() {
        let mut cmd_line = "/[^01]*/./g\r\n".graphemes(true).peekable();
        let address = Some(Address::span(1, 10));
        let mut prev_pattern = None;
        let (cmd, sfx) = parse_substitute_cmd(
            &mut cmd_line,
            address,
            &mut prev_pattern,
            &mut "".as_bytes(),
        )
        .unwrap();
        let Cmd::Substitute(a, p, r, SubstitutionScope::Global) = cmd else {
            panic!("Not Global!");
        };
        assert_eq!(a, address);
        assert_eq!(p.as_str(), "[^01]*");
        assert_eq!(r, ".");
        assert!(sfx.is_none());
    }

    #[test]
    fn parse_global_substitute_escaped_lf() {
        let mut cmd_line = "/, */,\\\n".graphemes(true).peekable();
        let address = Some(Address::span(1, 10));
        let mut prev_pattern = None;
        let (cmd, sfx) = parse_substitute_cmd(
            &mut cmd_line,
            address,
            &mut prev_pattern,
            &mut "/g\n".as_bytes(),
        )
        .unwrap();
        let Cmd::Substitute(a, p, r, s) = cmd else {
            panic!("Expected Cmd::Substitute, got {cmd:?}");
        };
        assert!(
            matches!(s, SubstitutionScope::Global),
            "expected SubstitutionScope::Global, got {s:?}"
        );
        assert_eq!(a, address);
        assert_eq!(p.as_str(), ", *");
        assert_eq!(r, ",\n");
        assert!(sfx.is_none());
    }

    #[test]
    fn parse_indexed_substitute() {
        let mut cmd_line = "/[^01]*/./3\r\n".graphemes(true).peekable();
        let address = Some(Address::span(1, 10));
        let mut prev_pattern = None;
        let (cmd, sfx) = parse_substitute_cmd(
            &mut cmd_line,
            address,
            &mut prev_pattern,
            &mut "".as_bytes(),
        )
        .unwrap();
        let Cmd::Substitute(a, p, r, SubstitutionScope::Single(3)) = cmd else {
            panic!("not Single(3)!");
        };
        assert_eq!(a, address);
        assert_eq!(p.as_str(), "[^01]*");
        assert_eq!(r, ".");
        assert!(sfx.is_none());
    }

    #[test]
    fn parse_substitute_conflicting_flags() {
        let mut cmd_line = "/[^01]*/./g1\r\n".graphemes(true).peekable();
        let address = Some(Address::span(1, 10));
        let mut prev_pattern = None;
        let res = parse_substitute_cmd(
            &mut cmd_line,
            address,
            &mut prev_pattern,
            &mut "".as_bytes(),
        )
        .expect_err("should fail");
        assert!(matches!(res, Error::RepeatedSubstitutionScope));

        let mut cmd_line = "/[^01]*/./4g\n".graphemes(true).peekable();
        let res = parse_substitute_cmd(
            &mut cmd_line,
            address,
            &mut prev_pattern,
            &mut "".as_bytes(),
        )
        .expect_err("should fail");
        assert!(matches!(res, Error::RepeatedSubstitutionScope));
    }

    #[test]
    fn parse_substitute_invalid_flag() {
        let mut cmd_line = "/[^01]*/./q\r\n".graphemes(true).peekable();
        let address = Some(Address::span(1, 10));
        let mut prev_pattern = None;
        let res = parse_substitute_cmd(
            &mut cmd_line,
            address,
            &mut prev_pattern,
            &mut "".as_bytes(),
        )
        .expect_err("should fail");
        assert!(matches!(res, Error::InvalidCmdSuffix));

        let mut cmd_line = "/[^01]*/./gq\n".graphemes(true).peekable();
        let res = parse_substitute_cmd(
            &mut cmd_line,
            address,
            &mut prev_pattern,
            &mut "".as_bytes(),
        )
        .expect_err("should fail");
        assert!(matches!(res, Error::InvalidCmdSuffix));
    }

    #[test]
    fn parse_transfer_cmd_with_destination() {
        let mut cmd_line = " 13\n".graphemes(true).peekable();
        let addr = Address::span(1, 2);
        let dest = Address::line(13);
        let res = parse_transfer_cmd(
            &mut cmd_line,
            &mut EditBuffer::new(),
            &mut None,
            Some(addr),
        )
        .unwrap();
        assert!(
            matches!(res, (Cmd::Transfer(Some(a), t), None) if a == addr && t == dest)
        );
    }

    #[test]
    fn parse_transfer_cmd_no_addr() {
        let mut input = "t42\n".as_bytes();
        let res =
            Cmd::read(&mut input, &mut EditBuffer::new(), &mut None).unwrap();
        assert!(matches!(
            res,
            (Cmd::Transfer(None, Address { start: 42, end: 42 }), None)
        ));
    }

    #[test]
    fn parse_transfer_cmd_no_destination() {
        let mut cmd_line = "\n".graphemes(true).peekable();
        let addr = Address::span(13, 42);
        let res = parse_transfer_cmd(
            &mut cmd_line,
            &mut EditBuffer::new(),
            &mut None,
            Some(addr),
        )
        .expect_err("shoudl fail");
        assert!(matches!(res, Error::MissingDestination));
    }

    #[test]
    fn parse_join_cmd_no_addr() {
        let mut input = "j\r\n".as_bytes();
        let res =
            Cmd::read(&mut input, &mut EditBuffer::new(), &mut None).unwrap();
        assert!(matches!(res, (Cmd::Join(None), None)));
    }

    #[test]
    fn parse_move_cmd_with_destination() {
        let mut cmd_line = " 13\n".graphemes(true).peekable();
        let addr = Address::span(1, 2);
        let dest = Address::line(13);
        let res = parse_move_cmd(
            &mut cmd_line,
            &mut EditBuffer::new(),
            &mut None,
            Some(addr),
        )
        .unwrap();
        assert!(
            matches!(res, (Cmd::Move(Some(a), t), None) if a == addr && t == dest)
        );
    }

    #[test]
    fn parse_move_cmd_no_addr() {
        let mut input = "m42\n".as_bytes();
        let res =
            Cmd::read(&mut input, &mut EditBuffer::new(), &mut None).unwrap();
        assert!(matches!(
            res,
            (Cmd::Move(None, Address { start: 42, end: 42 }), None)
        ));
    }

    #[test]
    fn parse_move_cmd_no_destination() {
        let mut cmd_line = "\n".graphemes(true).peekable();
        let addr = Address::span(13, 42);
        let res = parse_move_cmd(
            &mut cmd_line,
            &mut EditBuffer::new(),
            &mut None,
            Some(addr),
        )
        .expect_err("shoudl fail");
        assert!(matches!(res, Error::MissingDestination));
    }

    #[test]
    fn parse_write_cmd_with_address() {
        let mut cmd_line = " filename.rs".graphemes(true);
        let addr = Address::span(1, 10);
        let res = parse_write_cmd(&mut cmd_line, Some(addr)).unwrap();
        assert!(
            matches!(res, (Cmd::Write(Some(a), Some(f)), None) if a == addr && f.to_str().unwrap() == "filename.rs")
        );
    }

    #[test]
    fn parse_write_cmd_no_filename() {
        let mut cmd_line = "\n".graphemes(true);
        let addr = Address::span(1, 10);
        let res = parse_write_cmd(&mut cmd_line, Some(addr)).unwrap();
        assert!(matches!(res, (Cmd::Write(Some(a), None), None) if a == addr));
    }

    #[test]
    fn parse_write_cmd_bad_filename() {
        let mut cmd_line = " \r\n".graphemes(true);
        let res =
            parse_write_cmd(&mut cmd_line, None).expect_err("bad filename");
        assert!(matches!(res, Error::InvalidFilename));
    }

    #[test]
    fn parse_write_cmd_with_filename() {
        let mut cmd_line = " a/filename.rs\r\n".graphemes(true);
        let res = parse_write_cmd(&mut cmd_line, None).unwrap();
        assert!(
            matches!(&res, (Cmd::Write(None, Some(f)), None) if f.to_str().unwrap() == "a/filename.rs"),
            "{res:?} wasnt Cmd::Write(Some('filename.rs'))"
        );
    }

    #[test]
    fn parse_write_cmd_invalid_suffix() {
        let mut cmd_line = "filename.rs\n".graphemes(true);
        let res =
            parse_write_cmd(&mut cmd_line, None).expect_err("invalid suffix");
        assert!(matches!(res, Error::InvalidCmdSuffix));
    }
}
