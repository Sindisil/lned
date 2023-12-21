use core::fmt::{self, Debug, Display, Formatter};
use std::io::{self, BufRead};
use std::iter::Iterator;
use std::path::PathBuf;

use crate::edit_buffer::EditBuffer;
use crate::str_utils::StrUtils;

use regex::Regex;
use unicode_segmentation::UnicodeSegmentation;

#[derive(Debug)]
pub struct Parser {
    line: String,
}

#[derive(Debug, Clone)]
pub enum Cmd {
    Append(Option<Address>),
    Delete(Option<Address>),
    Edit(Option<PathBuf>),
    Enumerate(Option<Address>),
    File(Option<PathBuf>),
    Null(Option<Address>),
    Print(Option<Address>),
    Quit,
    Redo,
    Undo,
    Write(Option<Address>, Option<PathBuf>),
}

#[derive(Debug)]
pub enum Error {
    Unknown(char),
    UnexpectedAddress,
    OffsetTooLarge,
    OffsetTooSmall,
    OffsetOverflow,
    InvalidLineNumber,
    Regex(regex::Error),
    NoMatchingLine,
    NoPreviousPattern,
    TrailingBackslash,
    InvalidPatternDelimiter,
    InvalidCmdSuffix,
    InvalidFilename,
    ReadCommand(io::Error),
    MissingEol,
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub struct Address(pub usize, pub usize);

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum Separator {
    Comma,
    Semicolon,
}

impl std::error::Error for Error {}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            Error::UnexpectedAddress => write!(f, "Command takes no line address."),
            Error::Unknown(c) => write!(f, "Unknown command '{c}'"),
            Error::OffsetTooLarge => write!(f, "Offset too large"),
            Error::OffsetOverflow => write!(f, "Offset results in invalid line number"),
            Error::OffsetTooSmall => write!(f, "Offset too small"),
            Error::InvalidLineNumber => write!(f, "invalid line number"),
            Error::Regex(e) => write!(f, "{e}"),
            Error::NoMatchingLine => write!(f, "no matching line"),
            Error::TrailingBackslash => write!(f, "invalid trailing backslash"),
            Error::NoPreviousPattern => write!(f, "no previous pattern"),
            Error::InvalidPatternDelimiter => write!(f, "invalid pattern delimiter"),
            Error::InvalidCmdSuffix => write!(f, "invalid command suffix"),
            Error::InvalidFilename => write!(f, "invalid filename"),
            Error::ReadCommand(e) => write!(f, "{e} reading command input"),
            Error::MissingEol => write!(f, "missing line terminator"),
        }
    }
}

impl Parser {
    pub fn new() -> Parser {
        Parser {
            line: String::with_capacity(120),
        }
    }

    pub fn parse(
        &mut self,
        input: &mut impl BufRead,
        buffer: &mut EditBuffer,
        previous_pattern: &mut Option<Regex>,
    ) -> Result<Cmd, Error> {
        input
            .read_line(&mut self.line)
            .map_err(Error::ReadCommand)?;
        let mut cmd_line = self.line.as_mut_str().graphemes(true);
        let (address, cmd) = eval_address(&mut cmd_line, buffer, previous_pattern)?;
        match cmd {
            Some('a') => parse_no_args_cmd(cmd_line, Cmd::Append(address)),
            Some('d') => parse_no_args_cmd(cmd_line, Cmd::Delete(address)),
            Some('e') => parse_edit_cmd(cmd_line, address),
            Some('f') => parse_file_cmd(cmd_line, address),
            Some('n') => parse_no_args_cmd(cmd_line, Cmd::Enumerate(address)),
            None => Ok(Cmd::Null(address)),
            Some('p') => parse_no_args_cmd(cmd_line, Cmd::Print(address)),
            Some('q') => parse_lone_cmd(cmd_line, address, Cmd::Quit),
            Some('u') => parse_lone_cmd(cmd_line, address, Cmd::Undo),
            Some('U') => parse_lone_cmd(cmd_line, address, Cmd::Redo),
            Some('w') => parse_write_cmd(cmd_line, address),
            Some(c) => Err(Error::Unknown(c)),
        }
    }
}

fn parse_lone_cmd<'a, I>(cmd_line: I, address: Option<Address>, cmd: Cmd) -> Result<Cmd, Error>
where
    I: Iterator<Item = &'a str>,
{
    if address.is_some() {
        Err(Error::UnexpectedAddress)
    } else {
        parse_no_args_cmd(cmd_line, cmd)
    }
}

fn parse_no_args_cmd<'a, I>(mut cmd_line: I, cmd: Cmd) -> Result<Cmd, Error>
where
    I: Iterator<Item = &'a str>,
{
    match cmd_line.next() {
        None | Some("\n" | "\r\n") => Ok(cmd),
        _ => Err(Error::InvalidCmdSuffix),
    }
}

fn parse_file_cmd<'a, I>(mut cmd_line: I, address: Option<Address>) -> Result<Cmd, Error>
where
    I: Iterator<Item = &'a str>,
{
    if address.is_some() {
        return Err(Error::UnexpectedAddress);
    }
    match cmd_line.next() {
        None | Some("\n" | "\r\n") => Ok(Cmd::File(None)),
        Some(s) if s.is_blank() => {
            let filename = cmd_line
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

fn parse_write_cmd<'a, I>(mut cmd_line: I, address: Option<Address>) -> Result<Cmd, Error>
where
    I: Iterator<Item = &'a str>,
{
    match cmd_line.next() {
        None | Some("\n" | "\r\n") => Ok(Cmd::Write(address, None)),
        Some(s) if s.is_blank() => {
            let filename = cmd_line
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

fn parse_edit_cmd<'a, I>(mut cmd_line: I, address: Option<Address>) -> Result<Cmd, Error>
where
    I: Iterator<Item = &'a str>,
{
    if address.is_some() {
        return Err(Error::UnexpectedAddress);
    }
    match cmd_line.next() {
        None | Some("\n" | "\r\n") => Ok(Cmd::Edit(None)),
        Some(s) if s.is_blank() => {
            let filename = cmd_line
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

fn eval_address<'a, 'b, I>(
    cmd_line: &'a mut I,
    buffer: &mut EditBuffer,
    _previous_pattern: &mut Option<Regex>,
) -> Result<(Option<Address>, Option<char>), Error>
where
    I: Iterator<Item = &'b str>,
{
    let mut left = None;
    let mut right = None;
    let mut cmd_chr = None;

    let mut next = cmd_line.next();
    loop {
        match next {
            Some("\r\n" | "\n") => break,
            Some(".") => {
                right = Some(buffer.current_line());
                next = cmd_line.next();
            }
            Some("$") => {
                right = Some(buffer.len());
                next = cmd_line.next();
            }
            Some(s) if s.is_blank() => next = cmd_line.next(),
            Some(s) if s.is_ascii_digit() => (right, next) = parse_number(cmd_line, s)?,
            Some(s) => {
                cmd_chr = s.chars().next();
                break;
            }
            None => return Err(Error::MissingEol),
        }
    }

    let address = right.map(|r| Address(left.map_or(r, |l| l), r));

    Ok((address, cmd_chr))
}

fn parse_number<'a, 'b, I>(
    cmd_line: &'a mut I,
    first_digit: &'b str,
) -> Result<(Option<usize>, Option<&'b str>), Error>
where
    I: Iterator<Item = &'b str>,
{
    let mut acc = first_digit
        .chars()
        .next()
        .map_or(None, |c| c.to_digit(10))
        .ok_or(Error::InvalidLineNumber)? as usize;
    let mut next = cmd_line.next();
    while let Some(s) = next {
        if !s.is_ascii_digit() {
            break;
        }
        let d = s
            .chars()
            .next()
            .map_or(None, |c| c.to_digit(10))
            .ok_or(Error::InvalidLineNumber)? as usize;
        acc = acc
            .checked_mul(10)
            .and_then(|n| n.checked_add(d))
            .ok_or(Error::InvalidLineNumber)?;
        next = cmd_line.next();
    }
    Ok((Some(acc), next))
}

//impl Cmd {
//fn parse_edit_cmd(cmd_chars: &mut Peekable<Chars>, address: Option<Address>) -> Result<Cmd, Error> {
//    if address.is_some() {
//        return Err(Error::UnexpectedAddress);
//    }
//    match cmd_chars.peek() {
//        None | Some('\n' | '\r') => Ok(Cmd::Edit(None)),
//        Some(c) if c.is_blank() => {
//            let filename = parse_filename(cmd_chars);
//            if filename.is_empty() {
//                Err(Error::InvalidFilename)
//            } else {
//                Ok(Cmd::Edit(Some(PathBuf::from(filename))))
//            }
//        }
//        _ => Err(Error::InvalidCmdSuffix),
//    }
//}
//
//
//fn parse_filename(cmd_chars: &mut Peekable<Chars>) -> String {
//    let mut filename = String::new();
//    while let Some(c) = cmd_chars.next_if(|c| *c != '\n') {
//        filename.push(c);
//    }
//
//    filename.trim().to_owned()
//}
//pub fn eval_address<'a>(
//    cmd_line: &'a mut impl Iterator,
//    buffer: &mut EditBuffer,
//    previous_pattern: &mut Option<Regex>,
//) -> Result<(Option<Address>, Option<&'a str>), Error> {
//    let (addr, next_char) = eval_line_addr(cmd_line, buffer, previous_pattern)?;
//    let separator = parse_separator(cmd_line);
//    match separator {
//        None => Ok(addr.map(Address::Line)),
//        Some(sep) => Ok(Some(eval_addr_chain(
//            cmd_chars,
//            buffer,
//            addr,
//            sep,
//            previous_pattern,
//        )?)),
//    }
//}
//
//fn eval_addr_chain(
//    cmd_chars: &mut Peekable<Chars>,
//    buffer: &mut EditBuffer,
//    left: Option<usize>,
//    separator: Separator,
//    previous_pattern: &mut Option<Regex>,
//) -> Result<Address, Error> {
//    // set current_line if left has a value
//    if let Some(left) = left {
//        if separator == Separator::Semicolon {
//            if left == 0 || left > buffer.len() {
//                return Err(Error::InvalidLineNumber);
//            }
//            buffer.set_current_line(left);
//        }
//    }
//
//    let right = eval_line_addr(cmd_chars, buffer, previous_pattern)?
//        .unwrap_or_else(|| left.unwrap_or_else(|| buffer.len()));
//    let left = left.unwrap_or_else(|| match separator {
//        Separator::Semicolon => buffer.current_line(),
//        Separator::Comma => 1,
//    });
//
//    let next_separator = parse_separator(cmd_chars);
//
//    Ok(match next_separator {
//        None => Address::Span(left, right),
//        Some(separator) => {
//            eval_addr_chain(cmd_chars, buffer, Some(right), separator, previous_pattern)?
//        }
//    })
//}
//
//fn parse_separator(cmd_chars: &mut Peekable<Chars>) -> Option<Separator> {
//    match cmd_chars.peek() {
//        Some(c) if c.is_blank() => {
//            cmd_chars.next();
//            parse_separator(cmd_chars)
//        }
//        Some(',') => {
//            cmd_chars.next();
//            Some(Separator::Comma)
//        }
//        Some(';') => {
//            cmd_chars.next();
//            Some(Separator::Semicolon)
//        }
//        _ => None,
//    }
//
//fn eval_line_addr(
//    cmd_line: &mut impl Iterator,
//    buffer: &EditBuffer,
//    previous_pattern: &mut Option<Regex>,
//) -> Result<Option<usize>, Error> {
//    match cmd_line.next() {
//        Some(s) if c.is_blank() => eval_line_addr(cmd_chars, buffer, previous_pattern),
//        Some(".") => {
//            let offset = eval_addr_offsets(cmd_chars)?;
//            let line = buffer
//                .current_line()
//                .checked_add_signed(offset)
//                .ok_or(Error::OffsetOverflow)?;
//            Ok(Some(line))
//        }
//        Some('$') => {
//            cmd_chars.next();
//            let offset = eval_addr_offsets(cmd_chars)?;
//            let line = buffer
//                .len()
//                .checked_add_signed(offset)
//                .ok_or(Error::OffsetOverflow)?;
//            Ok(Some(line))
//        }
//        Some('/') => {
//            let pattern = parse_pattern(cmd_chars)?;
//            if !pattern.is_empty() {
//                *previous_pattern = Some(Regex::new(&pattern).map_err(Error::Regex)?);
//            }
//            let re = previous_pattern.as_ref().ok_or(Error::NoPreviousPattern)?;
//            let offset = eval_addr_offsets(cmd_chars)?;
//            let line = if buffer.current_line() == buffer.len() {
//                (1..=buffer.len()).find(|&i| re.is_match(&buffer[i]))
//            } else {
//                (buffer.current_line() + 1..=buffer.len())
//                    .find(|&i| re.is_match(&buffer[i]))
//                    .or_else(|| (1..=buffer.current_line()).find(|&i| re.is_match(&buffer[i])))
//            }
//            .ok_or(Error::NoMatchingLine)?;
//            let line = line
//                .checked_add_signed(offset)
//                .ok_or(Error::OffsetOverflow)?;
//            Ok(Some(line))
//        }
//        Some('?') => {
//            let pattern = parse_pattern(cmd_chars)?;
//            if !pattern.is_empty() {
//                *previous_pattern = Some(Regex::new(&pattern).map_err(Error::Regex)?);
//            }
//            let re = previous_pattern.as_ref().ok_or(Error::NoPreviousPattern)?;
//            let offset = eval_addr_offsets(cmd_chars)?;
//            let line = if buffer.current_line() == 1 {
//                (1..=buffer.len()).rev().find(|&i| re.is_match(&buffer[i]))
//            } else {
//                (1..buffer.current_line())
//                    .rev()
//                    .find(|&i| re.is_match(&buffer[i]))
//                    .or_else(|| {
//                        (buffer.current_line()..=buffer.len())
//                            .rev()
//                            .find(|&i| re.is_match(&buffer[i]))
//                    })
//            }
//            .ok_or(Error::NoMatchingLine)?;
//            let line = line
//                .checked_add_signed(offset)
//                .ok_or(Error::OffsetOverflow)?;
//            Ok(Some(line))
//        }
//        Some('0'..='9') => {
//            let num = cmd_chars
//                .peeking_take_while(char::is_ascii_digit)
//                .try_fold(0usize, |acc, c| {
//                    c.to_digit(10)
//                        .and_then(|d| acc.checked_mul(10).and_then(|n| n.checked_add(d as usize)))
//                })
//                .ok_or(Error::InvalidLineNumber)?;
//            let offset = eval_addr_offsets(cmd_chars)?;
//            let line = num
//                .checked_add_signed(offset)
//                .ok_or(Error::OffsetOverflow)?;
//            Ok(Some(line))
//        }
//        Some('+' | '-') => {
//            let offset = eval_addr_offsets(cmd_chars)?;
//            let line = buffer
//                .current_line()
//                .checked_add_signed(offset)
//                .ok_or(Error::InvalidLineNumber)?;
//            if line > buffer.len() {
//                Err(Error::InvalidLineNumber)
//            } else {
//                Ok(Some(line))
//            }
//        }
//        _ => Ok(None),
//    }
//}
//
//fn parse_pattern(cmd_chars: &mut Peekable<Chars>) -> Result<String, Error> {
//    let delimiter = cmd_chars
//        .next_if(|c| *c != '\n' && *c != '\r' && *c != ' ')
//        .ok_or(Error::InvalidPatternDelimiter)?;
//    let mut pattern = String::new();
//    while let Some(c) = cmd_chars.next_if(|c| *c != '\n' && *c != '\r') {
//        if c == delimiter {
//            break;
//        } else if c != '\\' {
//            pattern.push(c);
//        } else {
//            let escaped_c = cmd_chars
//                .next_if(|c| *c != 'r' && *c != '\n')
//                .ok_or(Error::TrailingBackslash)?;
//            if escaped_c != delimiter {
//                pattern.push('\\');
//            }
//            pattern.push(escaped_c);
//        }
//    }
//    Ok(pattern)
//}
//
//fn eval_addr_offsets(cmd_chars: &mut Peekable<Chars>) -> Result<isize, Error> {
//    let mut total_offset = 0isize;
//    while let Some(c) = cmd_chars.peek() {
//        let offset = match c {
//            ' ' | '\t' => {
//                cmd_chars.next();
//                None
//            }
//            '+' => {
//                cmd_chars.next();
//                Some(cmp::max(
//                    1,
//                    cmd_chars
//                        .peeking_take_while(char::is_ascii_digit)
//                        .try_fold(0isize, |acc, c| {
//                            c.to_digit(10).and_then(|d| {
//                                acc.checked_mul(10)
//                                    .and_then(|n| n.checked_add_unsigned(d.try_into().unwrap()))
//                            })
//                        })
//                        .ok_or(Error::OffsetTooLarge)?,
//                ))
//            }
//            '-' => {
//                cmd_chars.next();
//                Some(cmp::min(
//                    -1,
//                    cmd_chars
//                        .peeking_take_while(char::is_ascii_digit)
//                        .try_fold(0isize, |acc, c| {
//                            c.to_digit(10).and_then(|d| {
//                                acc.checked_mul(10)
//                                    .and_then(|n| n.checked_sub_unsigned(d.try_into().unwrap()))
//                            })
//                        })
//                        .ok_or(Error::OffsetTooSmall)?,
//                ))
//            }
//            '0'..='9' => Some(
//                cmd_chars
//                    .peeking_take_while(char::is_ascii_digit)
//                    .try_fold(0isize, |acc, c| {
//                        c.to_digit(10).and_then(|d| {
//                            acc.checked_mul(10)
//                                .and_then(|n| n.checked_add_unsigned(d.try_into().unwrap()))
//                        })
//                    })
//                    .ok_or(Error::OffsetTooLarge)?,
//            ),
//            _ => break,
//        };
//        if let Some(offset) = offset {
//            total_offset = total_offset
//                .checked_add(offset)
//                .ok_or(Error::OffsetOverflow)?;
//        }
//    }
//    Ok(total_offset)
//}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_lone_cmd() {
        let mut cmd_line = "\n".graphemes(true);
        let res = parse_lone_cmd(&mut cmd_line, None, Cmd::Quit).expect("good parse");
        assert!(matches!(res, Cmd::Quit));
    }

    #[test]
    fn parse_lone_cmd_error_with_address() {
        let mut cmd_line = "\n".graphemes(true);
        let res = parse_lone_cmd(&mut cmd_line, Some(Address(1, 1)), Cmd::Quit)
            .expect_err("unexpected address");
        assert!(matches!(res, Error::UnexpectedAddress));
    }

    #[test]
    fn parse_no_args_cmd_error_with_extra_chars() {
        let mut cmd_line = "extra\n".graphemes(true);
        let res = parse_no_args_cmd(&mut cmd_line, Cmd::Delete(None)).expect_err("invalid suffix");
        assert!(matches!(res, Error::InvalidCmdSuffix));
        let mut cmd_line = "extra\n".graphemes(true);
        let res = parse_no_args_cmd(&mut cmd_line, Cmd::Delete(None)).expect_err("invalid suffix");
        assert!(matches!(res, Error::InvalidCmdSuffix));
    }

    #[test]
    fn parse_no_args_cmd_both_line_terminators_valid() {
        let mut cmd_line = "\n".graphemes(true);
        let res = parse_no_args_cmd(&mut cmd_line, Cmd::Delete(None)).expect("parse ok");
        assert!(matches!(res, Cmd::Delete(None)));
        let mut cmd_line = "\n".graphemes(true);
        let res = parse_no_args_cmd(&mut cmd_line, Cmd::Delete(None)).expect("parse ok");
        assert!(matches!(res, Cmd::Delete(None)));
    }

    #[test]
    fn eval_no_addr_null_cmd() {
        let mut cmd_line = "\r\n".graphemes(true);
        let (address, cmd) =
            eval_address(&mut cmd_line, &mut EditBuffer::new(), &mut None).expect("good parse");
        assert!(address.is_none());
        assert!(cmd.is_none());
        let mut cmd_line = "\n".graphemes(true);
        let (address, cmd) =
            eval_address(&mut cmd_line, &mut EditBuffer::new(), &mut None).expect("good parse");
        assert!(address.is_none());
        assert!(cmd.is_none());
    }

    #[test]
    fn eval_no_addr_null_cmd_skip_spaces() {
        let mut cmd_line = "\t  \r\n".graphemes(true);
        let (address, cmd) =
            eval_address(&mut cmd_line, &mut EditBuffer::new(), &mut None).expect("good parse");
        assert!(address.is_none());
        assert!(cmd.is_none());
        let mut cmd_line = "\n".graphemes(true);
        let (address, cmd) =
            eval_address(&mut cmd_line, &mut EditBuffer::new(), &mut None).expect("good parse");
        assert!(address.is_none());
        assert!(cmd.is_none());
    }

    #[test]
    fn eval_addr_no_eol() {
        let mut cmd_line = "".graphemes(true);
        let res = eval_address(&mut cmd_line, &mut EditBuffer::new(), &mut None)
            .expect_err("mising line terminator");
        assert!(matches!(res, Error::MissingEol));
    }

    #[test]
    fn eval_no_addr() {
        let mut cmd_line = "q\n".graphemes(true);
        let (address, cmd) =
            eval_address(&mut cmd_line, &mut EditBuffer::new(), &mut None).expect("good parse");
        assert!(address.is_none());
        assert!(matches!(cmd, Some('q')));
    }

    #[test]
    fn eval_dot_addr() {
        let mut cmd_line = ".d\r\n".graphemes(true);
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3"]);
        buffer.set_current_line(2);
        let (address, cmd) =
            eval_address(&mut cmd_line, &mut buffer, &mut None).expect("should parse successfully");
        assert_eq!(address, Some(Address(2, 2)));
        assert_eq!(cmd, Some('d'));
    }

    #[test]
    fn eval_dollar_addr() {
        let mut cmd_line = "$d\r\n".graphemes(true);
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3"]);
        buffer.set_current_line(2);
        let (address, cmd) =
            eval_address(&mut cmd_line, &mut buffer, &mut None).expect("should parse successfully");
        assert_eq!(address, Some(Address(3, 3)));
        assert_eq!(cmd, Some('d'));
    }

    #[test]
    fn eval_simple_number_addr() {
        let mut cmd_line = "42d\n".graphemes(true);
        let (address, cmd) = eval_address(&mut cmd_line, &mut EditBuffer::new(), &mut None)
            .expect("should eval line number");
        assert_eq!(cmd, Some('d'));
        assert_eq!(address, Some(Address(42, 42)));
    }

    #[test]
    fn parse_append_cmd_no_addr() {
        let mut input = "a\r\n".as_bytes();
        let mut parser = Parser::new();
        let res = parser
            .parse(&mut input, &mut EditBuffer::new(), &mut None)
            .expect("good parse");
        assert!(matches!(res, Cmd::Append(None)));
    }

    #[test]
    fn parse_delete_cmd_no_addr() {
        let mut input = "d\r\n".as_bytes();
        let mut parser = Parser::new();
        let res = parser
            .parse(&mut input, &mut EditBuffer::new(), &mut None)
            .expect("good parse");
        assert!(
            matches!(res, Cmd::Delete(None)),
            "{res:?} didn't match Cmd::Delete(None)"
        );
    }

    #[test]
    fn parse_enumerate_cmd_no_addr() {
        let mut input = "n\r\n".as_bytes();
        let mut parser = Parser::new();
        let res = parser
            .parse(&mut input, &mut EditBuffer::new(), &mut None)
            .expect("good parse");
        assert!(
            matches!(res, Cmd::Enumerate(None)),
            "{res:?} didn't match Cmd::Enumerate(None)"
        );
    }

    #[test]
    fn parse_null_cmd_no_addr() {
        let mut input = "\r\n".as_bytes();
        let mut parser = Parser::new();
        let res = parser
            .parse(&mut input, &mut EditBuffer::new(), &mut None)
            .expect("good parse");
        assert!(matches!(res, Cmd::Null(None)));
    }

    #[test]
    fn parse_print_cmd_no_addr() {
        let mut input = "p\r\n".as_bytes();
        let mut parser = Parser::new();
        let res = parser
            .parse(&mut input, &mut EditBuffer::new(), &mut None)
            .expect("good parse");
        assert!(
            matches!(res, Cmd::Print(None)),
            "{res:?} didn't match Cmd::Print(None)"
        );
    }

    #[test]
    fn parse_quit_cmd() {
        let mut input = "q\r\n".as_bytes();
        let mut parser = Parser::new();
        let res = parser
            .parse(&mut input, &mut EditBuffer::new(), &mut None)
            .expect("good parse");
        assert!(matches!(res, Cmd::Quit), "{res:?} didn't match Cmd::Quit");
    }

    #[test]
    fn parse_undo_cmd() {
        let mut input = "u\r\n".as_bytes();
        let mut parser = Parser::new();
        let res = parser
            .parse(&mut input, &mut EditBuffer::new(), &mut None)
            .expect("good parse");
        assert!(matches!(res, Cmd::Undo), "{res:?} didn't match Cmd::Undo");
    }

    #[test]
    fn parse_redo_cmd() {
        let mut input = "U\r\n".as_bytes();
        let mut parser = Parser::new();
        let res = parser
            .parse(&mut input, &mut EditBuffer::new(), &mut None)
            .expect("good parse");
        assert!(matches!(res, Cmd::Redo), "{res:?} didn't match Cmd::Redo");
    }

    #[test]
    fn parse_quit_cmd_invalid_suffix() {
        let mut input = "q/more stuff/\r\n".as_bytes();
        let mut parser = Parser::new();
        let res = parser
            .parse(&mut input, &mut EditBuffer::new(), &mut None)
            .expect_err("invalid sufix");
        assert!(
            matches!(res, Error::InvalidCmdSuffix),
            "{res:?} didn't match Error::InvalidCmdSuffix"
        );
    }

    #[test]
    fn parse_unknown_command() {
        let mut input = "O\n".as_bytes();
        let mut parser = Parser::new();
        let res = parser
            .parse(&mut input, &mut EditBuffer::new(), &mut None)
            .expect_err("unknown cmd");
        assert!(
            matches!(res, Error::Unknown('O')),
            "{res:?} didn't match Error::Unknown('O')"
        );
    }

    #[test]
    fn parse_edit_with_address() {
        let cmd_line = " filename.rs".graphemes(true);
        let res = parse_edit_cmd(cmd_line, Some(Address(1, 1))).expect_err("unexpected addr");
        assert!(matches!(res, Error::UnexpectedAddress));
    }

    #[test]
    fn parse_edit_no_filename() {
        let cmd_line = "\n".graphemes(true);
        let res = parse_edit_cmd(cmd_line, None).expect("parsed edit cmd");
        assert!(matches!(res, Cmd::Edit(None)));
    }

    #[test]
    fn parse_edit_bad_filename() {
        let cmd_line = " \r\n".graphemes(true);
        let res = parse_edit_cmd(cmd_line, None).expect_err("bad filename");
        assert!(matches!(res, Error::InvalidFilename));
    }

    #[test]
    fn parse_edit_with_filename() {
        let cmd_line = " a/filename.rs\r\n".graphemes(true);
        let res = parse_edit_cmd(cmd_line, None).expect("parsed edit cmd");
        assert!(matches!(&res, Cmd::Edit(Some(f)) if f.to_str().unwrap() == "a/filename.rs"));
    }

    #[test]
    fn parse_edit_invalid_suffix() {
        let cmd_line = "filename.rs\n".graphemes(true);
        let res = parse_edit_cmd(cmd_line, None).expect_err("invalid suffix");
        assert!(matches!(res, Error::InvalidCmdSuffix));
    }

    #[test]
    fn parse_file_cmd_with_address() {
        let cmd_line = " filename.rs".graphemes(true);
        let res = parse_file_cmd(cmd_line, Some(Address(1, 1))).expect_err("unexpected addr");
        assert!(matches!(res, Error::UnexpectedAddress));
    }

    #[test]
    fn parse_file_cmd_no_filename() {
        let cmd_line = "\n".graphemes(true);
        let res = parse_file_cmd(cmd_line, None).expect("parsed file cmd");
        assert!(matches!(res, Cmd::File(None)));
    }

    #[test]
    fn parse_file_cmd_bad_filename() {
        let cmd_line = " \r\n".graphemes(true);
        let res = parse_file_cmd(cmd_line, None).expect_err("bad filename");
        assert!(matches!(res, Error::InvalidFilename));
    }

    #[test]
    fn parse_file_cmd_with_filename() {
        let cmd_line = " a/filename.rs\r\n".graphemes(true);
        let res = parse_file_cmd(cmd_line, None).expect("parsed file cmd");
        assert!(
            matches!(&res, Cmd::File(Some(f)) if f.to_str().unwrap() == "a/filename.rs"),
            "{res:?} wasnt Cmd::File(Some('filename.rs'))"
        );
    }

    #[test]
    fn parse_file_cmd_invalid_suffix() {
        let cmd_line = "filename.rs\n".graphemes(true);
        let res = parse_file_cmd(cmd_line, None).expect_err("invalid suffix");
        assert!(matches!(res, Error::InvalidCmdSuffix));
    }

    #[test]
    fn parse_write_cmd_with_address() {
        let cmd_line = " filename.rs".graphemes(true);
        let addr = Address(1, 10);
        let res = parse_write_cmd(cmd_line, Some(addr)).expect("parsed write cmd");
        assert!(
            matches!(res, Cmd::Write(Some(a), Some(f)) if a == addr && f.to_str().unwrap() == "filename.rs")
        );
    }

    #[test]
    fn parse_write_cmd_no_filename() {
        let cmd_line = "\n".graphemes(true);
        let addr = Address(1, 10);
        let res = parse_write_cmd(cmd_line, Some(addr)).expect("parsed file cmd");
        assert!(matches!(res, Cmd::Write(Some(a), None) if a == addr));
    }

    #[test]
    fn parse_write_cmd_bad_filename() {
        let cmd_line = " \r\n".graphemes(true);
        let res = parse_write_cmd(cmd_line, None).expect_err("bad filename");
        assert!(matches!(res, Error::InvalidFilename));
    }

    #[test]
    fn parse_write_cmd_with_filename() {
        let cmd_line = " a/filename.rs\r\n".graphemes(true);
        let res = parse_write_cmd(cmd_line, None).expect("parsed file cmd");
        assert!(
            matches!(&res, Cmd::Write(None, Some(f)) if f.to_str().unwrap() == "a/filename.rs"),
            "{res:?} wasnt Cmd::Write(Some('filename.rs'))"
        );
    }

    #[test]
    fn parse_write_cmd_invalid_suffix() {
        let cmd_line = "filename.rs\n".graphemes(true);
        let res = parse_write_cmd(cmd_line, None).expect_err("invalid suffix");
        assert!(matches!(res, Error::InvalidCmdSuffix));
    }
}
