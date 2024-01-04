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

#[derive(Debug)]
pub struct Reader<'a, R>
where
    R: BufRead,
{
    line: String,
    input: &'a mut R,
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
    ReadCommand(io::Error),
    MissingEol,
}

#[derive(Debug, PartialEq, Copy, Clone)]
pub struct Address(pub usize, pub usize);

impl std::error::Error for Error {}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        match self {
            Error::UnexpectedAddress => write!(f, "Command takes no line address."),
            Error::Unknown(c) => write!(f, "Unknown command '{c}'"),
            Error::OffsetTooLarge => write!(f, "Offset too large"),
            Error::OffsetOverflow => write!(f, "Offset results in invalid line number"),
            Error::OffsetTooSmall => write!(f, "Offset too small"),
            Error::InvalidAddress => write!(f, "invalid address"),
            Error::Regex(e) => write!(f, "{e}"),
            Error::NoMatchingLine => write!(f, "no matching line"),
            Error::TrailingBackslash => write!(f, "invalid trailing backslash"),
            Error::NoPreviousPattern => write!(f, "no previous pattern"),
            Error::InvalidPatternDelimiter => write!(f, "invalid pattern delimiter"),
            Error::InvalidCmdSuffix => write!(f, "invalid command suffix"),
            Error::InvalidFilename => write!(f, "invalid filename"),
            Error::ReadCommand(e) => write!(f, "{e} reading command input"),
            Error::MissingEol => write!(f, "missing line terminator"),
            Error::NumberParse => write!(f, "invalid numeric string"),
        }
    }
}

impl<'a, R> Reader<'a, R>
where
    R: BufRead,
{
    pub fn new(input: &'a mut R) -> Reader<'a, R> {
        Reader {
            line: String::with_capacity(120),
            input,
        }
    }

    fn clear(&mut self) {
        self.line.clear();
    }

    // Read lines of input into buf, stopping when a '.' alone on a line
    // is read. Clears previous content of buf, but doesn't shrink capacity.
    // Returns number of bytes read or Error::Readlines if an error is
    // encountered.
    pub fn read_lines(&mut self, buf: &mut Vec<String>) -> Result<usize, io::Error> {
        loop {
            self.clear(); // get rid of any old input
            self.input.read_line(&mut self.line)?;
            if self.line == ".\n" || self.line == ".\r\n" {
                return Ok(buf.len());
            }
            buf.push(self.line.clone());
        }
    }

    pub fn read_cmd(
        &mut self,
        buffer: &mut EditBuffer,
        previous_pattern: &mut Option<Regex>,
    ) -> Result<Cmd, Error> {
        self.clear();
        self.input
            .read_line(&mut self.line)
            .map_err(Error::ReadCommand)?;
        let mut graphemes = self.line.as_mut_str().graphemes(true).peekable();
        let address = eval_address(&mut graphemes, buffer, previous_pattern)?;
        match graphemes.next() {
            Some("a") => parse_no_args(&mut graphemes, Cmd::Append(address)),
            Some("d") => parse_no_args(&mut graphemes, Cmd::Delete(address)),
            Some("e") => parse_edit_cmd(&mut graphemes, address),
            Some("f") => parse_file_cmd(&mut graphemes, address),
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

fn parse_write_cmd<'a, I>(graphemes: &mut I, address: Option<Address>) -> Result<Cmd, Error>
where
    I: Iterator<Item = &'a str>,
{
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

fn parse_edit_cmd<'a, I>(graphemes: &mut I, address: Option<Address>) -> Result<Cmd, Error>
where
    I: Iterator<Item = &'a str>,
{
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

fn eval_address<'a, I>(
    graphemes: &mut Peekable<I>,
    buffer: &mut EditBuffer,
    _previous_pattern: &mut Option<Regex>,
) -> Result<Option<Address>, Error>
where
    I: Iterator<Item = &'a str>,
{
    let mut left = None;
    let mut right = None;

    loop {
        match graphemes.peek() {
            Some(&"\r\n" | &"\n") => break,
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
            Some(&"+" | &"-") => todo!(),
            Some(&".") => {
                graphemes.next();
                let offset = eval_offsets(graphemes)?;
                right = Some(
                    buffer
                        .current_line()
                        .checked_add_signed(offset)
                        .ok_or(Error::OffsetOverflow)?,
                );
            }
            Some(&"$") => {
                graphemes.next();
                let offset = eval_offsets(graphemes)?;
                right = Some(
                    buffer
                        .len()
                        .checked_add_signed(offset)
                        .ok_or(Error::OffsetOverflow)?,
                );
            }
            Some(s) if s.is_blank() => {
                graphemes.next();
            }
            Some(s) if s.is_ascii_digit() => {
                let num = parse_number(graphemes)?;
                let offset = eval_offsets(graphemes)?;
                right = Some(
                    num.checked_add_signed(offset)
                        .ok_or(Error::OffsetOverflow)?,
                );
                if left.is_none() {
                    left = right;
                }
            }
            Some(_) => break,
            None => return Err(Error::MissingEol),
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

fn eval_offsets<'a, I>(graphemes: &mut Peekable<I>) -> Result<isize, Error>
where
    I: Iterator<Item = &'a str>,
{
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

fn parse_no_args<'a, I>(graphemes: &mut I, cmd: Cmd) -> Result<Cmd, Error>
where
    I: Iterator<Item = &'a str>,
{
    match graphemes.next() {
        None | Some("\n" | "\r\n") => Ok(cmd),
        _ => Err(Error::InvalidCmdSuffix),
    }
}

fn parse_number<'a, I>(graphemes: &mut Peekable<I>) -> Result<usize, Error>
where
    I: Iterator<Item = &'a str>,
{
    graphemes
        .peeking_take_while(|s| s.chars().next().is_some_and(|c| c.is_ascii_digit()))
        .try_fold(0usize, |acc, s| {
            s.chars()
                .next()
                .and_then(|c| c.to_digit(10))
                .and_then(|d| acc.checked_mul(10).and_then(|n| n.checked_add(d as usize)))
        })
        .ok_or(Error::NumberParse)
    //    let mut acc = first_digit
    //        .chars()
    //        .next()
    //        .map_or(None, |c| c.to_digit(10))
    //        .ok_or(Error::InvalidLineNumber)? as usize;
    //    let mut next = graphemes.next();
    //    while let Some(s) = next {
    //        if !s.is_ascii_digit() {
    //            break;
    //        }
    //        let d = s
    //            .chars()
    //            .next()
    //            .map_or(None, |c| c.to_digit(10))
    //            .ok_or(Error::InvalidLineNumber)? as usize;
    //        acc = acc
    //            .checked_mul(10)
    //            .and_then(|n| n.checked_add(d))
    //            .ok_or(Error::InvalidLineNumber)?;
    //        next = graphemes.next();
    //    }
    //    Ok((acc, next))
}

fn parse_file_cmd<'a, I>(graphemes: &mut I, address: Option<Address>) -> Result<Cmd, Error>
where
    I: Iterator<Item = &'a str>,
{
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
        let res = parse_no_address(None, Cmd::Quit).expect("good parse");
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
        let res = parse_no_args(&mut cmd_line, Cmd::Delete(None)).expect("parse ok");
        assert!(matches!(res, Cmd::Delete(None)));
    }

    #[test]
    fn eval_no_addr_null_cmd() {
        let mut cmd_line = "\r\n".graphemes(true).peekable();
        let address =
            eval_address(&mut cmd_line, &mut EditBuffer::new(), &mut None).expect("good parse");
        assert!(address.is_none());
        assert!(matches!(cmd_line.next(), Some("\r\n")));
        let mut cmd_line = "\n".graphemes(true).peekable();
        let address =
            eval_address(&mut cmd_line, &mut EditBuffer::new(), &mut None).expect("good parse");
        assert!(address.is_none());
        assert!(matches!(cmd_line.next(), Some("\n")));
    }

    #[test]
    fn eval_no_addr_null_cmd_skip_spaces() {
        let mut cmd_line = "\t  \r\n".graphemes(true).peekable();
        let address =
            eval_address(&mut cmd_line, &mut EditBuffer::new(), &mut None).expect("good parse");
        assert!(address.is_none());
        assert!(matches!(cmd_line.next(), Some("\r\n")));
        let mut cmd_line = "\n".graphemes(true).peekable();
        let address =
            eval_address(&mut cmd_line, &mut EditBuffer::new(), &mut None).expect("good parse");
        assert!(address.is_none());
        assert!(matches!(cmd_line.next(), Some("\n")));
    }

    #[test]
    fn eval_positive_offset() {
        let mut input = "3p".graphemes(true).peekable();
        let res = eval_offsets(&mut input).expect("should parse");
        assert_eq!(res, 3);
        assert!(matches!(input.next(), Some("p")));
        let mut input = "+42p".graphemes(true).peekable();
        let res = eval_offsets(&mut input).expect("should parse");
        assert_eq!(res, 42);
        assert!(matches!(input.next(), Some("p")));
    }

    #[test]
    fn eval_negative_offsets() {
        let mut input = "-2p".graphemes(true).peekable();
        let res = eval_offsets(&mut input).expect("should parse");
        assert_eq!(res, -2);
        assert!(matches!(input.next(), Some("p")));
    }

    #[test]
    fn eval_mixed_offsets() {
        let mut input = "2-7+6p".graphemes(true).peekable();
        let res = eval_offsets(&mut input).expect("should parse");
        assert_eq!(res, 1);
        assert!(matches!(input.next(), Some("p")));
    }

    #[test]
    fn eval_offset_overflow() {
        let mut input = "8399999999999999999+839999999999999999+8399999999999999999p"
            .graphemes(true)
            .peekable();
        let res = eval_offsets(&mut input).expect_err("shouldn't parse");
        assert!(matches!(res, Error::OffsetOverflow));

        let mut input = "-839999999999999999-83999999999999999-8399999999999999999p"
            .graphemes(true)
            .peekable();
        let res = eval_offsets(&mut input).expect_err("shouldn't parse");
        assert!(matches!(res, Error::OffsetOverflow));
    }

    #[test]
    fn eval_offset_too_large() {
        let mut input = "999999999999999999999p".graphemes(true).peekable();
        let res = eval_offsets(&mut input).expect_err("shouldn't parse");
        assert!(matches!(res, Error::OffsetTooLarge));
        let mut input = "+999999999999999999999p".graphemes(true).peekable();
        let res = eval_offsets(&mut input).expect_err("shouldn't parse");
        assert!(matches!(res, Error::OffsetTooLarge));
    }

    #[test]
    fn eval_offset_too_small() {
        let mut input = "-999999999999999999999p".graphemes(true).peekable();
        let res = eval_offsets(&mut input).expect_err("shouldn't parse");
        assert!(matches!(res, Error::OffsetTooSmall));
    }

    #[test]
    fn eval_mixed_offsets_with_spaces() {
        let mut input = "   2 -7  6 +1p".graphemes(true).peekable();
        let res = eval_offsets(&mut input).expect("should parse");
        assert_eq!(res, 2);
        assert!(matches!(input.next(), Some("p")));
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
        let address =
            eval_address(&mut cmd_line, &mut EditBuffer::new(), &mut None).expect("good parse");
        assert!(address.is_none());
        assert_eq!(cmd_line.next(), Some("q"));
    }

    #[test]
    fn eval_dot_addr() {
        let mut cmd_line = ".d\r\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3"]);
        buffer.set_current_line(2);
        let address =
            eval_address(&mut cmd_line, &mut buffer, &mut None).expect("should parse successfully");
        assert_eq!(address, Some(Address(2, 2)));
        assert_eq!(cmd_line.next(), Some("d"));
    }

    #[test]
    fn eval_dollar_addr() {
        let mut cmd_line = "$d\r\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\r\n", "2", "3"]);
        buffer.set_current_line(2);
        let address =
            eval_address(&mut cmd_line, &mut buffer, &mut None).expect("should parse successfully");
        assert_eq!(address, Some(Address(3, 3)));
        assert_eq!(cmd_line.next(), Some("d"));
    }

    #[test]
    fn eval_simple_number_addr() {
        let mut cmd_line = "42d\n".graphemes(true).peekable();
        let address = eval_address(&mut cmd_line, &mut EditBuffer::new(), &mut None)
            .expect("should eval line number");
        assert_eq!(cmd_line.next(), Some("d"));
        assert_eq!(address, Some(Address(42, 42)));
    }

    #[test]
    fn eval_simple_comma_addr() {
        let mut input = "1,2p\n".graphemes(true).peekable();
        let res =
            eval_address(&mut input, &mut EditBuffer::new(), &mut None).expect("should succeed");
        assert_eq!(res, Some(Address(1, 2)));
        assert_eq!(input.next(), Some("p"));
    }

    #[test]
    fn eval_leading_comma_addr() {
        let mut input = ",4p\r\n".graphemes(true).peekable();
        let res =
            eval_address(&mut input, &mut EditBuffer::new(), &mut None).expect("should eval ok");
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address(1, 4)));
    }

    #[test]
    fn eval_trailing_comma_addr() {
        let mut input = "5,p\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let res = eval_address(&mut input, &mut buffer, &mut None).expect("should eval ok");
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address(5, 5)));
    }

    #[test]
    fn eval_comma_only_addr() {
        let mut input = ",p\r\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let res = eval_address(&mut input, &mut buffer, &mut None).expect("should eval ok");
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address(1, 6)));
    }

    #[test]
    fn eval_comma_only_chain_addr() {
        let mut input = ",,p\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let res = eval_address(&mut input, &mut buffer, &mut None).expect("should eval ok");
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address(6, 6)));
    }

    #[test]
    fn eval_comma_chain_addr() {
        let mut input = ",12, 3+1,p\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        let res = eval_address(&mut input, &mut buffer, &mut None).expect("should eval ok");
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address(4, 4)));
    }

    #[test]
    fn eval_simple_semicolon_addr() {
        let mut input = "1;2p\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        assert_eq!(buffer.current_line(), 6);
        let res = eval_address(&mut input, &mut buffer, &mut None).expect("should succeed");
        assert_eq!(res, Some(Address(1, 2)));
        assert_eq!(buffer.current_line(), 1);
        assert_eq!(input.next(), Some("p"));
    }

    #[test]
    fn eval_leading_semicolon_addr() {
        let mut input = ";5p\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(3);
        let res = eval_address(&mut input, &mut buffer, &mut None).expect("should eval ok");
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address(3, 5)));
        assert_eq!(buffer.current_line(), 3);
    }

    #[test]
    fn eval_trailing_semicolon_addr() {
        let mut input = "5;p\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(3);
        let res = eval_address(&mut input, &mut buffer, &mut None).expect("should eval ok");
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address(5, 5)));
        assert_eq!(buffer.current_line(), 5);
    }

    #[test]
    fn eval_semicolon_only_addr() {
        let mut input = ";p\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(3);
        let res = eval_address(&mut input, &mut buffer, &mut None).expect("should eval ok");
        assert_eq!(input.next(), Some("p"));
        assert_eq!(res, Some(Address(3, 6)));
        assert_eq!(buffer.current_line(), 3);
    }

    #[test]
    fn eval_semicolon_only_chain_addr() {
        let mut input = ";;p\n".graphemes(true).peekable();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(3);
        let res = eval_address(&mut input, &mut buffer, &mut None).expect("should eval ok");
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
    fn parse_append_cmd_no_addr() {
        let mut input = "a\r\n".as_bytes();
        let mut reader = Reader::new(&mut input);
        let res = reader
            .read_cmd(&mut EditBuffer::new(), &mut None)
            .expect("good parse");
        assert!(matches!(res, Cmd::Append(None)));
    }

    #[test]
    fn parse_delete_cmd_no_addr() {
        let mut input = "d\r\n".as_bytes();
        let mut reader = Reader::new(&mut input);
        let res = reader
            .read_cmd(&mut EditBuffer::new(), &mut None)
            .expect("good parse");
        assert!(
            matches!(res, Cmd::Delete(None)),
            "{res:?} didn't match Cmd::Delete(None)"
        );
    }

    #[test]
    fn parse_enumerate_cmd_no_addr() {
        let mut input = "n\r\n".as_bytes();
        let mut reader = Reader::new(&mut input);
        let res = reader
            .read_cmd(&mut EditBuffer::new(), &mut None)
            .expect("good parse");
        assert!(
            matches!(res, Cmd::Enumerate(None)),
            "{res:?} didn't match Cmd::Enumerate(None)"
        );
    }

    #[test]
    fn parse_null_cmd_no_addr() {
        let mut input = "\r\n".as_bytes();
        let mut reader = Reader::new(&mut input);
        let res = reader
            .read_cmd(&mut EditBuffer::new(), &mut None)
            .expect("good parse");
        assert!(matches!(res, Cmd::Null(None)));
    }

    #[test]
    fn parse_print_cmd_no_addr() {
        let mut input = "p\r\n".as_bytes();
        let mut reader = Reader::new(&mut input);
        let res = reader
            .read_cmd(&mut EditBuffer::new(), &mut None)
            .expect("good parse");
        assert!(
            matches!(res, Cmd::Print(None)),
            "{res:?} didn't match Cmd::Print(None)"
        );
    }

    #[test]
    fn parse_quit_cmd() {
        let mut input = "q\r\n".as_bytes();
        let mut reader = Reader::new(&mut input);
        let res = reader
            .read_cmd(&mut EditBuffer::new(), &mut None)
            .expect("good parse");
        assert!(matches!(res, Cmd::Quit), "{res:?} didn't match Cmd::Quit");
    }

    #[test]
    fn parse_undo_cmd() {
        let mut input = "u\r\n".as_bytes();
        let mut reader = Reader::new(&mut input);
        let res = reader
            .read_cmd(&mut EditBuffer::new(), &mut None)
            .expect("good parse");
        assert!(matches!(res, Cmd::Undo), "{res:?} didn't match Cmd::Undo");
    }

    #[test]
    fn parse_redo_cmd() {
        let mut input = "U\r\n".as_bytes();
        let mut reader = Reader::new(&mut input);
        let res = reader
            .read_cmd(&mut EditBuffer::new(), &mut None)
            .expect("good parse");
        assert!(matches!(res, Cmd::Redo), "{res:?} didn't match Cmd::Redo");
    }

    #[test]
    fn parse_quit_cmd_invalid_suffix() {
        let mut input = "q/more stuff/\r\n".as_bytes();
        let mut reader = Reader::new(&mut input);
        let res = reader
            .read_cmd(&mut EditBuffer::new(), &mut None)
            .expect_err("invalid sufix");
        assert!(
            matches!(res, Error::InvalidCmdSuffix),
            "{res:?} didn't match Error::InvalidCmdSuffix"
        );
    }

    #[test]
    fn parse_unknown_command() {
        let mut input = "O\n".as_bytes();
        let mut reader = Reader::new(&mut input);
        let res = reader
            .read_cmd(&mut EditBuffer::new(), &mut None)
            .expect_err("unknown cmd");
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
        let res = parse_edit_cmd(&mut cmd_line, None).expect("parsed edit cmd");
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
        let res = parse_edit_cmd(&mut cmd_line, None).expect("parsed edit cmd");
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
        let res = parse_file_cmd(&mut cmd_line, None).expect("parsed file cmd");
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
        let res = parse_file_cmd(&mut cmd_line, None).expect("parsed file cmd");
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
    fn parse_write_cmd_with_address() {
        let mut cmd_line = " filename.rs".graphemes(true);
        let addr = Address(1, 10);
        let res = parse_write_cmd(&mut cmd_line, Some(addr)).expect("parsed write cmd");
        assert!(
            matches!(res, Cmd::Write(Some(a), Some(f)) if a == addr && f.to_str().unwrap() == "filename.rs")
        );
    }

    #[test]
    fn parse_write_cmd_no_filename() {
        let mut cmd_line = "\n".graphemes(true);
        let addr = Address(1, 10);
        let res = parse_write_cmd(&mut cmd_line, Some(addr)).expect("parsed file cmd");
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
        let res = parse_write_cmd(&mut cmd_line, None).expect("parsed file cmd");
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
