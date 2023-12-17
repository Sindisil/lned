use core::fmt::{self, Debug, Display, Formatter};
use std::io::{self, BufRead};
use std::iter::Iterator;
use std::path::PathBuf;

use unicode_segmentation::UnicodeSegmentation;

use crate::edit_buffer::EditBuffer;

use regex::Regex;

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
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub enum Address {
    Line(usize),
    Span(usize, usize),
}

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
            //            Some('e') => parse_edit_cmd(cmd_line, address),
            //            Some('f') => parse_file_cmd(mut cmd_line, address),
            Some('n') => parse_no_args_cmd(cmd_line, Cmd::Enumerate(address)),
            None => Ok(Cmd::Null(address)),
            Some('p') => parse_no_args_cmd(cmd_line, Cmd::Print(address)),
            Some('q') => parse_lone_cmd(cmd_line, address, Cmd::Quit),
            Some('u') => parse_lone_cmd(cmd_line, address, Cmd::Undo),
            Some('U') => parse_lone_cmd(cmd_line, address, Cmd::Redo),
            //            Some('w') => parse_write_cmd(cmd_line, address),
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

fn eval_address<'a, 'b, I>(
    _cmd_line: &'a mut I,
    _buffer: &mut EditBuffer,
    _previous_pattern: &mut Option<Regex>,
) -> Result<(Option<Address>, Option<char>), Error>
where
    I: Iterator<Item = &'b str>,
{
    todo!();
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
//fn parse_file_cmd(cmd_chars: &mut Peekable<Chars>, address: Option<Address>) -> Result<Cmd, Error> {
//    if address.is_some() {
//        return Err(Error::UnexpectedAddress);
//    }
//    match cmd_chars.peek() {
//        None | Some('\n' | '\r') => Ok(Cmd::File(None)),
//        Some(c) if c.is_blank() => {
//            let filename = parse_filename(cmd_chars);
//            if filename.is_empty() {
//                Err(Error::InvalidFilename)
//            } else {
//                Ok(Cmd::File(Some(PathBuf::from(filename))))
//            }
//        }
//        _ => Err(Error::InvalidCmdSuffix),
//    }
//}
//
//fn parse_filename(cmd_chars: &mut Peekable<Chars>) -> String {
//    let mut filename = String::new();
//    while let Some(c) = cmd_chars.next_if(|c| *c != '\n') {
//        filename.push(c);
//    }
//
//    filename.trim().to_owned()
//}
//
//fn parse_write_cmd(
//    cmd_chars: &mut Peekable<Chars>,
//    address: Option<Address>,
//) -> Result<Cmd, Error> {
//    match cmd_chars.peek() {
//        None | Some('\n' | '\r') => Ok(Cmd::Write(address, None)),
//        Some(c) if c.is_blank() => {
//            let filename = parse_filename(cmd_chars);
//            if filename.is_empty() {
//                Err(Error::InvalidFilename)
//            } else {
//                Ok(Cmd::Write(address, Some(PathBuf::from(filename))))
//            }
//        }
//        _ => Err(Error::InvalidCmdSuffix),
//    }
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
//}
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
        let res = parse_lone_cmd(&mut cmd_line, Some(Address::Line(1)), Cmd::Quit)
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
}
