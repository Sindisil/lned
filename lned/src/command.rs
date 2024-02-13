use core::cmp;
use core::fmt::{self, Debug, Display, Formatter};
use std::io::{self, BufRead};
use std::iter::{Iterator, Peekable};
use std::path::PathBuf;

use crate::edit_buffer::EditBuffer;
use crate::iter_utils::Peeking;
use crate::str_utils::StrUtils;

use regex::Regex;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Debug, Clone)]
pub enum Cmd {
    Append(Option<Address>),
    Change(Option<Address>),
    Delete(Option<Address>),
    Edit(Option<PathBuf>),
    Enumerate(Option<Address>),
    File(Option<PathBuf>),
    Global(Option<Address>, Regex, String),
    Insert(Option<Address>),
    Null(Option<Address>),
    Print(Option<Address>),
    Quit,
    Redo,
    Undo,
    Write(Option<Address>, Option<PathBuf>),
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
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub struct Address(pub usize, pub usize);

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
            | Error::MissingEol => None,
            Error::ReadCommand { ref source } => Some(source),
        }
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Error::UnexpectedAddress => write!(f, "command takes no line address."),
            Error::Unknown(c) => write!(f, "unknown command '{c}'"),
            Error::OffsetTooLarge => write!(f, "offset too large"),
            Error::OffsetOverflow => write!(f, "offset results in invalid line number"),
            Error::OffsetTooSmall => write!(f, "offset too small"),
            Error::InvalidAddress => write!(f, "invalid address"),
            Error::Regex(e) => write!(f, "{e}"),
            Error::NoMatchingLine => write!(f, "no matching line"),
            Error::TrailingBackslash => write!(f, "invalid trailing backslash"),
            Error::NoPreviousPattern => write!(f, "no previous pattern"),
            Error::InvalidPatternDelimiter => write!(f, "invalid pattern delimiter"),
            Error::InvalidCmdSuffix => write!(f, "invalid command suffix"),
            Error::InvalidFilename => write!(f, "invalid filename"),
            Error::ReadCommand { .. } => write!(f, "error reading command input"),
            Error::MissingEol => write!(f, "missing line terminator"),
            Error::NumberParse => write!(f, "invalid numeric string"),
        }
    }
}

impl Cmd {
    // Read lines of input into buf, stopping when a '.' alone on a line
    // is read. Clears previous content of buf, but doesn't shrink capacity.
    // Returns number of bytes read or Error::Readlines if an error is
    // encountered.
    pub fn read_lines(input: &mut impl BufRead, buf: &mut Vec<String>) -> Result<usize, io::Error> {
        buf.clear();
        loop {
            let mut line = String::new();
            let n = input.read_line(&mut line)?;
            if n == 0 || line == ".\n" || line == ".\r\n" {
                return Ok(buf.len());
            }
            buf.push(line);
        }
    }

    /// Read input, parsing into a Cmd
    pub fn read(
        input: &mut impl BufRead,
        buffer: &mut EditBuffer,
        previous_pattern: &mut Option<Regex>,
    ) -> Result<Cmd, Error> {
        let mut line = String::with_capacity(120);
        input
            .read_line(&mut line)
            .map_err(|source| Error::ReadCommand { source })?;
        let mut graphemes = line.as_mut_str().graphemes(true).peekable();
        let address = eval_address(&mut graphemes, buffer, previous_pattern)?;
        match graphemes.next() {
            Some("a") => parse_no_args(&mut graphemes, Cmd::Append(address)),
            Some("c") => parse_no_args(&mut graphemes, Cmd::Change(address)),
            Some("d") => parse_no_args(&mut graphemes, Cmd::Delete(address)),
            Some("e") => parse_edit_cmd(&mut graphemes, address),
            Some("f") => parse_file_cmd(&mut graphemes, address),
            Some("g") => parse_global_cmd(&mut graphemes, address, previous_pattern, input),
            Some("i") => parse_no_args(&mut graphemes, Cmd::Insert(address)),
            Some("n") => parse_no_args(&mut graphemes, Cmd::Enumerate(address)),
            None | Some("\n" | "\r\n") => Ok(Cmd::Null(address)),
            Some("p") => parse_no_args(&mut graphemes, Cmd::Print(address)),
            Some("q") => parse_no_address(address, Cmd::Quit)
                .and_then(|cmd| parse_no_args(&mut graphemes, cmd)),
            Some("u") => parse_no_address(address, Cmd::Undo)
                .and_then(|cmd| parse_no_args(&mut graphemes, cmd)),
            Some("U") => parse_no_address(address, Cmd::Redo)
                .and_then(|cmd| parse_no_args(&mut graphemes, cmd)),
            Some("w") => parse_write_cmd(&mut graphemes, address),
            Some(s) => Err(Error::Unknown(s.to_owned())),
        }
    }
}

fn parse_write_cmd<'a>(
    graphemes: &mut impl Iterator<Item = &'a str>,
    address: Option<Address>,
) -> Result<Cmd, Error> {
    match graphemes.next() {
        None | Some("\n" | "\r\n") => Ok(Cmd::Write(address, None)),
        Some(s) if s.is_blank() => {
            let filename = graphemes
                .take_while(|s| *s != "\n" && *s != "\r\n")
                .collect::<String>()
                .trim()
                .to_owned();
            if filename.is_empty() {
                Err(Error::InvalidFilename)
            } else {
                Ok(Cmd::Write(address, Some(PathBuf::from(filename))))
            }
        }
        _ => Err(Error::InvalidCmdSuffix),
    }
}

fn parse_edit_cmd<'a>(
    graphemes: &mut impl Iterator<Item = &'a str>,
    address: Option<Address>,
) -> Result<Cmd, Error> {
    if address.is_some() {
        return Err(Error::UnexpectedAddress);
    }
    match graphemes.next() {
        None | Some("\n" | "\r\n") => Ok(Cmd::Edit(None)),
        Some(s) if s.is_blank() => {
            let filename = graphemes
                .take_while(|s| *s != "\n" && *s != "\r\n")
                .collect::<String>()
                .trim()
                .to_owned();
            if filename.is_empty() {
                Err(Error::InvalidFilename)
            } else {
                Ok(Cmd::Edit(Some(PathBuf::from(filename))))
            }
        }
        _ => Err(Error::InvalidCmdSuffix),
    }
}

fn eval_address<'a>(
    graphemes: &mut Peekable<impl Iterator<Item = &'a str>>,
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
                    Some(r) => {
                        buffer.set_current_line(r);
                        r
                    }
                    None => buffer.current_line(),
                });
                right = right.or_else(|| Some(buffer.len()));
            }
            Some(&"+" | &"-") => {
                right = Some(eval_line_number(graphemes, buffer.current_line())?);
            }
            Some(&".") => {
                graphemes.next();
                right = Some(eval_line_number(graphemes, buffer.current_line())?);
            }
            Some(&"$") => {
                graphemes.next();
                right = Some(eval_line_number(graphemes, buffer.len())?);
            }
            Some(&"/") => {
                let pattern = parse_pattern(graphemes)?;
                if !pattern.is_empty() {
                    *previous_pattern = Some(Regex::new(&pattern).map_err(Error::Regex)?);
                }
                let re = previous_pattern.as_ref().ok_or(Error::NoPreviousPattern)?;
                let line = if buffer.current_line() == buffer.len() {
                    (1..=buffer.len()).find(|&i| re.is_match(&buffer[i]))
                } else {
                    (buffer.current_line() + 1..=buffer.len())
                        .find(|&i| re.is_match(&buffer[i]))
                        .or_else(|| (1..=buffer.current_line()).find(|&i| re.is_match(&buffer[i])))
                }
                .ok_or(Error::NoMatchingLine)?;
                right = Some(eval_line_number(graphemes, line)?);
            }
            Some(&"?") => {
                let pattern = parse_pattern(graphemes)?;
                if !pattern.is_empty() {
                    *previous_pattern = Some(Regex::new(&pattern).map_err(Error::Regex)?);
                }
                let re = previous_pattern.as_ref().ok_or(Error::NoPreviousPattern)?;
                let line = if buffer.current_line() == 1 {
                    (1..=buffer.len()).rev().find(|&i| re.is_match(&buffer[i]))
                } else {
                    (1..buffer.current_line())
                        .rev()
                        .find(|&i| re.is_match(&buffer[i]))
                        .or_else(|| {
                            (buffer.current_line()..=buffer.len())
                                .rev()
                                .find(|&i| re.is_match(&buffer[i]))
                        })
                }
                .ok_or(Error::NoMatchingLine)?;
                right = Some(eval_line_number(graphemes, line)?);
            }
            Some(s) if s.is_blank() => {
                graphemes.next();
            }
            Some(s) if s.is_ascii_digit() => {
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

    let address = right.map(|r| Address(left.map_or(r, |l| l), r));
    address.map_or_else(
        || Ok(None),
        |a| {
            if a.0 > a.1 {
                Err(Error::InvalidAddress)
            } else {
                Ok(Some(a))
            }
        },
    )
}

fn eval_line_number<'a>(
    graphemes: &mut Peekable<impl Iterator<Item = &'a str>>,
    line: usize,
) -> Result<usize, Error> {
    let offset = compute_line_offset(graphemes)?;
    line.checked_add_signed(offset).ok_or(Error::OffsetOverflow)
}

fn parse_pattern<'a>(
    graphemes: &mut Peekable<impl Iterator<Item = &'a str>>,
) -> Result<String, Error> {
    let delimiter = graphemes
        .next_if(|gr| *gr != "\n" && *gr != "\r\n" && *gr != " ")
        .ok_or(Error::InvalidPatternDelimiter)?;
    let mut pattern = String::new();
    while let Some(gr) = graphemes.next_if(|gr| *gr != "\n" && *gr != "\r\n") {
        if gr == delimiter {
            break;
        } else if gr != "\\" {
            pattern.push_str(gr);
        } else {
            let escaped_gr = graphemes
                .next_if(|gr| *gr != "\n" && *gr != "\r\n")
                .ok_or(Error::TrailingBackslash)?;
            if escaped_gr != delimiter {
                pattern.push('\\');
            }
            pattern.push_str(escaped_gr);
        }
    }
    Ok(pattern)
}

fn compute_line_offset<'a>(
    graphemes: &mut Peekable<impl Iterator<Item = &'a str>>,
) -> Result<isize, Error> {
    let mut total_offset = 0isize;
    while let Some(s) = graphemes.peek() {
        match *s {
            s if s.is_blank() => {
                graphemes.next();
            }
            s if s.is_ascii_digit() => {
                total_offset = parse_number(graphemes)
                    .and_then(|o| o.try_into().map_err(|_| Error::OffsetTooLarge))
                    .and_then(|o| total_offset.checked_add(o).ok_or(Error::OffsetTooLarge))
                    .map_err(|_| Error::OffsetTooLarge)?;
            }
            "+" => {
                graphemes.next();
                total_offset = parse_number(graphemes)
                    .map_err(|_| Error::OffsetTooLarge)
                    .and_then(|o| o.try_into().map_err(|_| Error::OffsetTooLarge))
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
                    .and_then(|o| o.try_into().map_err(|_| Error::OffsetTooSmall))
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

fn parse_no_args<'a>(
    graphemes: &mut impl Iterator<Item = &'a str>,
    cmd: Cmd,
) -> Result<Cmd, Error> {
    match graphemes.next() {
        None | Some("\n" | "\r\n") => Ok(cmd),
        _ => Err(Error::InvalidCmdSuffix),
    }
}

fn parse_number<'a>(
    graphemes: &mut Peekable<impl Iterator<Item = &'a str>>,
) -> Result<usize, Error> {
    graphemes
        .peeking_take_while(|s| s.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .try_fold(0usize, |acc, s| {
            s.chars()
                .next()
                .and_then(|c| c.to_digit(10))
                .and_then(|d| acc.checked_mul(10).and_then(|n| n.checked_add(d as usize)))
        })
        .ok_or(Error::NumberParse)
}

fn parse_file_cmd<'a>(
    graphemes: &mut impl Iterator<Item = &'a str>,
    address: Option<Address>,
) -> Result<Cmd, Error> {
    if address.is_some() {
        return Err(Error::UnexpectedAddress);
    }
    match graphemes.next() {
        None | Some("\n" | "\r\n") => Ok(Cmd::File(None)),
        Some(s) if s.is_blank() => {
            let filename = graphemes
                .take_while(|s| *s != "\n" && *s != "\r\n")
                .collect::<String>()
                .trim()
                .to_owned();
            if filename.is_empty() {
                Err(Error::InvalidFilename)
            } else {
                Ok(Cmd::File(Some(PathBuf::from(filename))))
            }
        }
        _ => Err(Error::InvalidCmdSuffix),
    }
}

fn parse_global_cmd<'a>(
    graphemes: &mut Peekable<impl Iterator<Item = &'a str>>,
    address: Option<Address>,
    previous_pattern: &mut Option<Regex>,
    input: &mut impl BufRead,
) -> Result<Cmd, Error> {
    let pattern = parse_pattern(graphemes)?;
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

    // if the EOL was escaped, use read_lines() to read in rest of command list
    if more_lines {
        let mut lines = Vec::new();
        if Cmd::read_lines(input, &mut lines).map_err(|source| Error::ReadCommand { source })? > 0 {
            for line in lines {
                commands.push_str(&line);
            }
        }
    }

    Ok(Cmd::Global(address, pattern, commands))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_lone_cmd() {
        let res = parse_no_address(None, Cmd::Quit).unwrap();
        assert!(matches!(res, Cmd::Quit));
    }

    #[test]
    fn parse_no_address_error_with_address() {
        let res = parse_no_address(Some(Address(1, 1)), Cmd::Quit).expect_err("unexpected address");
        assert!(matches!(res, Error::UnexpectedAddress));
    }

    #[test]
    fn parse_no_args_error_with_extra_chars() {
        let mut cmd_line = "extra\n".graphemes(true);
        let res = parse_no_args(&mut cmd_line, Cmd::Delete(None)).expect_err("invalid suffix");
        assert!(matches!(res, Error::InvalidCmdSuffix));
    }

    #[test]
    fn parse_no_args_both_line_terminators_valid() {
        let mut cmd_line = "\n".graphemes(true);
        let res = parse_no_args(&mut cmd_line, Cmd::Delete(None)).unwrap();
        assert!(matches!(res, Cmd::Delete(None)));
    }

    #[test]
    fn eval_no_addr_null_cmd() {
        let mut cmd_line = "\r\n".graphemes(true).peekable();
        let address = eval_address(&mut cmd_line, &mut EditBuffer::new(), &mut None).unwrap();
        assert!(address.is_none());
        assert!(matches!(cmd_line.next(), Some("\r\n")));
        let mut cmd_line = "\n".graphemes(true).peekable();
        let address = eval_address(&mut cmd_line, &mut EditBuffer::new(), &mut None).unwrap();
        assert!(address.is_none());
        assert!(matches!(cmd_line.next(), Some("\n")));
    }

    #[test]
    fn eval_no_addr_null_cmd_skip_spaces() {
        let mut cmd_line = "\t  \r\n".graphemes(true).peekable();
        let address = eval_address(&mut cmd_line, &mut EditBuffer::new(), &mut None).unwrap();
        assert!(address.is_none());
        assert!(matches!(cmd_line.next(), Some("\r\n")));
        let mut cmd_line = "\n".graphemes(true).peekable();
        let address = eval_address(&mut cmd_line, &mut EditBuffer::new(), &mut None).unwrap();
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
        let mut input = "8399999999999999999+839999999999999999+8399999999999999999p"
            .graphemes(true)
            .peekable();
        let res = compute_line_offset(&mut input).expect_err("shouldn't parse");
        assert!(matches!(res, Error::OffsetOverflow));

        let mut input = "-839999999999999999-83999999999999999-8399999999999999999p"
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
    fn parse_pattern_invalid_delimiter() {
        let mut input = " stuff + other_stuff. \n".graphemes(true).peekable();
        let res = parse_pattern(&mut input);
        assert!(matches!(res, Err(Error::InvalidPatternDelimiter)));
    }

    #[test]
    fn parse_pattern_trailing_backslash() {
        let mut input = "/stuff + other_stuff.\\\n".graphemes(true).peekable();
        let res = parse_pattern(&mut input).expect_err("trailing backslash");
        assert!(matches!(res, Error::TrailingBackslash));
        let mut input = "/stuff + other_stuff.\\".graphemes(true).peekable();
        let res = parse_pattern(&mut input).expect_err("trailing backslash");
        assert!(matches!(res, Error::TrailingBackslash));
    }

    #[test]
    fn parse_pattern_no_terminating_delimiter() {
        let mut input = "/stuff\\/other_stuff.\n".graphemes(true).peekable();
        let res = parse_pattern(&mut input).unwrap();
        assert_eq!("stuff/other_stuff.".to_owned(), res);
    }

    #[test]
    fn parse_pattern_escaped_terminator() {
        let mut input = "/stuff\\/other_stuff./\n".graphemes(true).peekable();
        let res = parse_pattern(&mut input).unwrap();
        assert_eq!("stuff/other_stuff.".to_owned(), res);
    }

    #[test]
    fn parse_pattern_escaped_chars() {
        let mut input = "?stuff \\+ other_stuff\\.?\n".graphemes(true).peekable();
        let res = parse_pattern(&mut input).unwrap();
        assert_eq!("stuff \\+ other_stuff\\.".to_owned(), res);
    }

    #[test]
    fn parse_pattern_no_escaped_chars() {
        let mut input = "/stuff + other_stuff./\n".graphemes(true).peekable();
        let res = parse_pattern(&mut input).unwrap();
        assert_eq!("stuff + other_stuff.".to_owned(), res);
    }

    #[test]
    fn eval_addr_no_eol() {
        let mut cmd_line = "".graphemes(true).peekable();
        let res = eval_address(&mut cmd_line, &mut EditBuffer::new(), &mut None)
            .expect_err("mising line terminator");
        assert!(matches!(res, Error::MissingEol));
    }

    #[test]
    fn eval_no_addr() {
        let mut cmd_line = "q\n".graphemes(true).peekable();
        let address = eval_address(&mut cmd_line, &mut EditBuffer::new(), &mut None).unwrap();
        assert!(address.is_none());
        assert_eq!(cmd_line.next(), Some("q"));
    }

    #[test]
    fn eval_dot_addr() {
        let mut cmd_line = ".d\r\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3"]);
        buffer.set_current_line(2);
        let address = eval_address(&mut cmd_line, &mut buffer, &mut None).unwrap();
        assert_eq!(address, Some(Address(2, 2)));
        assert_eq!(cmd_line.next(), Some("d"));
    }

    #[test]
    fn eval_dollar_addr() {
        let mut cmd_line = "$d\r\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3"]);
        buffer.set_current_line(2);
        let address = eval_address(&mut cmd_line, &mut buffer, &mut None).unwrap();
        assert_eq!(address, Some(Address(3, 3)));
        assert_eq!(cmd_line.next(), Some("d"));
    }

    #[test]
    fn eval_simple_number_addr() {
        let mut cmd_line = "42d\n".graphemes(true).peekable();
        let address = eval_address(&mut cmd_line, &mut EditBuffer::new(), &mut None).unwrap();
        assert_eq!(cmd_line.next(), Some("d"));
        assert_eq!(address, Some(Address(42, 42)));
    }

    #[test]
    fn regex_line_addr_regex_syntax() {
        let mut input = "/\\lo.+/n\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["one\r\n", "two", "three", "four", "five", "six"]);
        buffer.set_current_line(2);
        let mut previous_pattern: Option<Regex> = None;
        let res =
            eval_address(&mut input, &mut buffer, &mut previous_pattern).expect_err("bad pattern");
        assert!(matches!(res, Error::Regex(_)));
    }

    #[test]
    fn rev_regex_line_addr_regex_syntax() {
        let mut input = "?\\lo.+?n\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["one\r\n", "two", "three", "four", "five", "six"]);
        buffer.set_current_line(2);
        let mut previous_pattern: Option<Regex> = None;
        let res =
            eval_address(&mut input, &mut buffer, &mut previous_pattern).expect_err("bad pattern");
        assert!(matches!(res, Error::Regex(_)));
    }

    #[test]
    fn regex_line_addr_embedded_delim() {
        let mut input = "/o.+\\//n\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["one/\n", "two", "three", "four", "five", "six"]);
        buffer.set_current_line(2);
        let mut previous_pattern: Option<Regex> = None;
        let res = eval_address(&mut input, &mut buffer, &mut previous_pattern).unwrap();
        assert_eq!(res, Some(Address(1, 1)));
    }

    #[test]
    fn regex_line_addr_no_final_delimiter() {
        let mut input = "/o.+\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["one\r\n", "two", "three", "four", "five", "six"]);
        buffer.set_current_line(2);
        let mut previous_pattern: Option<Regex> = None;
        let res = eval_address(&mut input, &mut buffer, &mut previous_pattern).unwrap();
        assert_eq!(res, Some(Address(4, 4)));
    }

    #[test]
    fn regex_line_addr_needle_in_first_half_of_split_range() {
        let mut input = "/o.+/n\n".graphemes(true).peekable();
        let mut previous_pattern: Option<Regex> = None;
        let mut buffer = EditBuffer::from(vec!["one\r\n", "two", "three", "four", "five", "six"]);
        buffer.set_current_line(2);
        let res = eval_address(&mut input, &mut buffer, &mut previous_pattern).unwrap();
        assert_eq!(res, Some(Address(4, 4)));
    }

    #[test]
    fn regex_line_addr_needle_in_second_half_of_split_range() {
        let mut input = "/on.+/n\n".graphemes(true).peekable();
        let mut previous_pattern: Option<Regex> = None;
        let mut buffer = EditBuffer::from(vec!["one\r\n", "two", "three", "four", "five", "six"]);
        buffer.set_current_line(4);
        let res = eval_address(&mut input, &mut buffer, &mut previous_pattern).unwrap();
        assert_eq!(res, Some(Address(1, 1)));
    }

    #[test]
    fn regex_line_addr_contiguous_search_range() {
        let mut input = "/o.+/n\n".graphemes(true).peekable();
        let mut previous_pattern: Option<Regex> = None;
        let mut buffer = EditBuffer::from(vec!["one\r\n", "two", "three", "four", "five", "six"]);
        buffer.set_current_line(6);
        let res = eval_address(&mut input, &mut buffer, &mut previous_pattern).unwrap();
        assert_eq!(res, Some(Address(1, 1)));
    }

    #[test]
    fn rev_regex_line_addr_needle_in_first_half_of_split_range() {
        let mut input = "?o.+?n\n".graphemes(true).peekable();
        let mut previous_pattern: Option<Regex> = None;
        let mut buffer = EditBuffer::from(vec!["one\r\n", "two", "three", "four", "five", "six"]);
        buffer.set_current_line(2);
        let res = eval_address(&mut input, &mut buffer, &mut previous_pattern).unwrap();
        assert_eq!(res, Some(Address(1, 1)));
    }

    #[test]
    fn rev_regex_line_addr_needle_in_second_half_of_split_range() {
        let mut input = "?ou.+?n\n".graphemes(true).peekable();
        let mut previous_pattern: Option<Regex> = None;
        let mut buffer = EditBuffer::from(vec!["one\r\n", "two", "three", "four", "five", "six"]);
        buffer.set_current_line(4);
        let res = eval_address(&mut input, &mut buffer, &mut previous_pattern).unwrap();
        assert_eq!(res, Some(Address(4, 4)));
    }

    #[test]
    fn rev_regex_line_addr_contiguous_search_range() {
        let mut input = "?o.+?n\n".graphemes(true).peekable();
        let mut previous_pattern: Option<Regex> = None;
        let mut buffer = EditBuffer::from(vec!["one\r\n", "two", "three", "four", "five", "six"]);
        buffer.set_current_line(1);
        let res = eval_address(&mut input, &mut buffer, &mut previous_pattern).unwrap();
        assert_eq!(res, Some(Address(4, 4)));
    }

    #[test]
    fn regex_line_addr_with_offset() {
        let mut input = "/o.+/+2\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["one\r\n", "two", "three", "four", "five", "six"]);
        buffer.set_current_line(2);
        let mut previous_pattern: Option<Regex> = None;
        let res = eval_address(&mut input, &mut buffer, &mut previous_pattern).unwrap();
        assert_eq!(res, Some(Address(6, 6)));
    }

    #[test]
    fn rev_regex_line_addr_with_offset() {
        let mut input = "?o.+?+2\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["one\r\n", "two", "three", "four", "five", "six"]);
        buffer.set_current_line(2);
        let mut previous_pattern: Option<Regex> = None;
        let res = eval_address(&mut input, &mut buffer, &mut previous_pattern).unwrap();
        assert_eq!(res, Some(Address(3, 3)));
    }

    #[test]
    fn eval_simple_comma_addr() {
        let mut input = "1,2p\n".graphemes(true).peekable();
        let res = eval_address(&mut input, &mut EditBuffer::new(), &mut None).unwrap();
        assert_eq!(res, Some(Address(1, 2)));
        assert_eq!(input.next(), Some("p"));
    }

    #[test]
    fn eval_leading_comma_addr() {
        let mut input = ",4p\r\n".graphemes(true).peekable();
        let res = eval_address(&mut input, &mut EditBuffer::new(), &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address(1, 4)));
    }

    #[test]
    fn eval_trailing_comma_addr() {
        let mut input = "5,p\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let res = eval_address(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address(5, 5)));
    }

    #[test]
    fn eval_comma_only_addr() {
        let mut input = ",p\r\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let res = eval_address(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address(1, 6)));
    }

    #[test]
    fn eval_comma_only_chain_addr() {
        let mut input = ",,p\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let res = eval_address(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address(6, 6)));
    }

    #[test]
    fn eval_comma_chain_addr() {
        let mut input = ",12, 3+1,p\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let res = eval_address(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address(4, 4)));
    }

    #[test]
    fn eval_simple_semicolon_addr() {
        let mut input = "1;2p\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        assert_eq!(buffer.current_line(), 6);
        let res = eval_address(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(res, Some(Address(1, 2)));
        assert_eq!(buffer.current_line(), 1);
        assert_eq!(input.next(), Some("p"));
    }

    #[test]
    fn eval_leading_semicolon_addr() {
        let mut input = ";5p\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(3);
        let res = eval_address(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address(3, 5)));
        assert_eq!(buffer.current_line(), 3);
    }

    #[test]
    fn eval_trailing_semicolon_addr() {
        let mut input = "5;p\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(3);
        let res = eval_address(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address(5, 5)));
        assert_eq!(buffer.current_line(), 5);
    }

    #[test]
    fn eval_semicolon_only_addr() {
        let mut input = ";p\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(3);
        let res = eval_address(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address(3, 6)));
        assert_eq!(buffer.current_line(), 3);
    }

    #[test]
    fn eval_semicolon_only_chain_addr() {
        let mut input = ";;p\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(3);
        let res = eval_address(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address(6, 6)));
    }

    #[test]
    fn eval_big_before_small_semicolon_chain_addr() {
        let mut input = "4;$;2p\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(3);
        let res = eval_address(&mut input, &mut buffer, &mut None).expect_err("invalid address");
        assert!(matches!(res, Error::InvalidAddress));
    }

    #[test]
    fn eval_simple_offset_only_addrs() {
        let mut input = "+p\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(3);
        let res = eval_address(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address(4, 4)));

        let mut input = "+10p\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(3);
        let res = eval_address(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address(13, 13)));

        let mut input = "-p\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(3);
        let res = eval_address(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address(2, 2)));

        let mut input = "-2p\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(3);
        let res = eval_address(&mut input, &mut buffer, &mut None).unwrap();
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address(1, 1)));
    }

    #[test]
    fn eval_too_big_offset_only_addr_overflows() {
        let mut input = "-10p\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(3);
        let res = eval_address(&mut input, &mut buffer, &mut None).expect_err("offset overflow");
        assert!(matches!(res, Error::OffsetOverflow));
    }

    #[test]
    fn parse_append_cmd_no_addr() {
        let mut input = "a\r\n".as_bytes();
        let res = Cmd::read(&mut input, &mut EditBuffer::new(), &mut None).unwrap();
        assert!(matches!(res, Cmd::Append(None)));
    }

    #[test]
    fn parse_delete_cmd_no_addr() {
        let mut input = "d\r\n".as_bytes();
        let res = Cmd::read(&mut input, &mut EditBuffer::new(), &mut None).unwrap();
        assert!(
            matches!(res, Cmd::Delete(None)),
            "{res:?} didn't match Cmd::Delete(None)"
        );
    }

    #[test]
    fn parse_enumerate_cmd_no_addr() {
        let mut input = "n\r\n".as_bytes();
        let res = Cmd::read(&mut input, &mut EditBuffer::new(), &mut None).unwrap();
        assert!(
            matches!(res, Cmd::Enumerate(None)),
            "{res:?} didn't match Cmd::Enumerate(None)"
        );
    }

    #[test]
    fn parse_insert_cmd_no_addr() {
        let mut input = "i\r\n".as_bytes();
        let res = Cmd::read(&mut input, &mut EditBuffer::new(), &mut None).unwrap();
        assert!(matches!(res, Cmd::Insert(None)));
    }

    #[test]
    fn parse_null_cmd_no_addr() {
        let mut input = "\r\n".as_bytes();
        let res = Cmd::read(&mut input, &mut EditBuffer::new(), &mut None).unwrap();
        assert!(matches!(res, Cmd::Null(None)));
    }

    #[test]
    fn parse_print_cmd_no_addr() {
        let mut input = "p\r\n".as_bytes();
        let res = Cmd::read(&mut input, &mut EditBuffer::new(), &mut None).unwrap();
        assert!(
            matches!(res, Cmd::Print(None)),
            "{res:?} didn't match Cmd::Print(None)"
        );
    }

    #[test]
    fn parse_quit_cmd() {
        let mut input = "q\r\n".as_bytes();
        let res = Cmd::read(&mut input, &mut EditBuffer::new(), &mut None).unwrap();
        assert!(matches!(res, Cmd::Quit), "{res:?} didn't match Cmd::Quit");
    }

    #[test]
    fn parse_undo_cmd() {
        let mut input = "u\r\n".as_bytes();
        let res = Cmd::read(&mut input, &mut EditBuffer::new(), &mut None).unwrap();
        assert!(matches!(res, Cmd::Undo), "{res:?} didn't match Cmd::Undo");
    }

    #[test]
    fn parse_redo_cmd() {
        let mut input = "U\r\n".as_bytes();
        let res = Cmd::read(&mut input, &mut EditBuffer::new(), &mut None).unwrap();
        assert!(matches!(res, Cmd::Redo), "{res:?} didn't match Cmd::Redo");
    }

    #[test]
    fn parse_quit_cmd_invalid_suffix() {
        let mut input = "q/more stuff/\r\n".as_bytes();
        let res =
            Cmd::read(&mut input, &mut EditBuffer::new(), &mut None).expect_err("invalid sufix");
        assert!(
            matches!(res, Error::InvalidCmdSuffix),
            "{res:?} didn't match Error::InvalidCmdSuffix"
        );
    }

    #[test]
    fn parse_unknown_command() {
        let mut input = "O\n".as_bytes();
        let res =
            Cmd::read(&mut input, &mut EditBuffer::new(), &mut None).expect_err("unknown cmd");
        assert!(
            matches!(res, Error::Unknown(ref s) if s == "O"),
            "{res:?} didn't match Error::Unknown(\"O\")"
        );
    }

    #[test]
    fn parse_edit_with_address() {
        let mut cmd_line = " filename.rs".graphemes(true);
        let res = parse_edit_cmd(&mut cmd_line, Some(Address(1, 1))).expect_err("unexpected addr");
        assert!(matches!(res, Error::UnexpectedAddress));
    }

    #[test]
    fn parse_edit_no_filename() {
        let mut cmd_line = "\n".graphemes(true);
        let res = parse_edit_cmd(&mut cmd_line, None).unwrap();
        assert!(matches!(res, Cmd::Edit(None)));
    }

    #[test]
    fn parse_edit_bad_filename() {
        let mut cmd_line = " \r\n".graphemes(true);
        let res = parse_edit_cmd(&mut cmd_line, None).expect_err("bad filename");
        assert!(matches!(res, Error::InvalidFilename));
    }

    #[test]
    fn parse_edit_with_filename() {
        let mut cmd_line = " a/filename.rs\r\n".graphemes(true);
        let res = parse_edit_cmd(&mut cmd_line, None).unwrap();
        assert!(matches!(&res, Cmd::Edit(Some(f)) if f.to_str().unwrap() == "a/filename.rs"));
    }

    #[test]
    fn parse_edit_invalid_suffix() {
        let mut cmd_line = "filename.rs\n".graphemes(true);
        let res = parse_edit_cmd(&mut cmd_line, None).expect_err("invalid suffix");
        assert!(matches!(res, Error::InvalidCmdSuffix));
    }

    #[test]
    fn parse_file_cmd_with_address() {
        let mut cmd_line = " filename.rs".graphemes(true);
        let res = parse_file_cmd(&mut cmd_line, Some(Address(1, 1))).expect_err("unexpected addr");
        assert!(matches!(res, Error::UnexpectedAddress));
    }

    #[test]
    fn parse_file_cmd_no_filename() {
        let mut cmd_line = "\n".graphemes(true);
        let res = parse_file_cmd(&mut cmd_line, None).unwrap();
        assert!(matches!(res, Cmd::File(None)));
    }

    #[test]
    fn parse_file_cmd_bad_filename() {
        let mut cmd_line = " \r\n".graphemes(true);
        let res = parse_file_cmd(&mut cmd_line, None).expect_err("bad filename");
        assert!(matches!(res, Error::InvalidFilename));
    }

    #[test]
    fn parse_file_cmd_with_filename() {
        let mut cmd_line = " a/filename.rs\r\n".graphemes(true);
        let res = parse_file_cmd(&mut cmd_line, None).unwrap();
        assert!(
            matches!(&res, Cmd::File(Some(f)) if f.to_str().unwrap() == "a/filename.rs"),
            "{res:?} wasnt Cmd::File(Some('filename.rs'))"
        );
    }

    #[test]
    fn parse_file_cmd_invalid_suffix() {
        let mut cmd_line = "filename.rs\n".graphemes(true);
        let res = parse_file_cmd(&mut cmd_line, None).expect_err("invalid suffix");
        assert!(matches!(res, Error::InvalidCmdSuffix));
    }

    #[test]
    fn parse_simple_global_cmd() {
        let mut input = "/pat/p\r\n".graphemes(true).peekable();
        let mut prev_pattern = None;
        let res =
            parse_global_cmd(&mut input, None, &mut prev_pattern, &mut "".as_bytes()).unwrap();
        assert!(matches!(res,
            Cmd::Global(a, p, c) if a.is_none() && p.as_str() == "pat" && c == "p\r\n"));
    }

    #[test]
    fn parse_multi_global_cmd() {
        let mut input = "/pat/n\\\r\n".graphemes(true).peekable();
        let mut more_input = "d\r\n".as_bytes();
        let mut prev_pattern = None;
        let res = parse_global_cmd(&mut input, None, &mut prev_pattern, &mut more_input).unwrap();
        assert!(matches!(res,
            Cmd::Global(a, p, c) if a.is_none() && p.as_str() == "pat" && c == "n\r\nd\r\n"));
    }

    #[test]
    fn parse_write_cmd_with_address() {
        let mut cmd_line = " filename.rs".graphemes(true);
        let addr = Address(1, 10);
        let res = parse_write_cmd(&mut cmd_line, Some(addr)).unwrap();
        assert!(
            matches!(res, Cmd::Write(Some(a), Some(f)) if a == addr && f.to_str().unwrap() == "filename.rs")
        );
    }

    #[test]
    fn parse_write_cmd_no_filename() {
        let mut cmd_line = "\n".graphemes(true);
        let addr = Address(1, 10);
        let res = parse_write_cmd(&mut cmd_line, Some(addr)).unwrap();
        assert!(matches!(res, Cmd::Write(Some(a), None) if a == addr));
    }

    #[test]
    fn parse_write_cmd_bad_filename() {
        let mut cmd_line = " \r\n".graphemes(true);
        let res = parse_write_cmd(&mut cmd_line, None).expect_err("bad filename");
        assert!(matches!(res, Error::InvalidFilename));
    }

    #[test]
    fn parse_write_cmd_with_filename() {
        let mut cmd_line = " a/filename.rs\r\n".graphemes(true);
        let res = parse_write_cmd(&mut cmd_line, None).unwrap();
        assert!(
            matches!(&res, Cmd::Write(None, Some(f)) if f.to_str().unwrap() == "a/filename.rs"),
            "{res:?} wasnt Cmd::Write(Some('filename.rs'))"
        );
    }

    #[test]
    fn parse_write_cmd_invalid_suffix() {
        let mut cmd_line = "filename.rs\n".graphemes(true);
        let res = parse_write_cmd(&mut cmd_line, None).expect_err("invalid suffix");
        assert!(matches!(res, Error::InvalidCmdSuffix));
    }
}
