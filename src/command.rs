use core::cmp;
use core::fmt::{self, Debug, Display, Formatter};
use core::iter::Peekable;
use core::str::Chars;

use crate::char_utils::CharUtils;
use crate::edit_buffer::EditBuffer;
use crate::iter_utils::Peeking;

use regex::Regex;

#[derive(Debug, PartialEq, Clone, Hash)]
pub enum Cmd {
    Quit,
    Null(Option<Address>),
    Print(Option<Address>),
    Append(Option<Address>, Vec<String>),
    Delete(Option<Address>),
    Undo,
}

#[derive(Debug, PartialEq)]
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
        }
    }
}

impl Cmd {
    pub fn parse(
        cmd_chars: &mut Peekable<Chars>,
        buffers: &mut [EditBuffer],
        current_buffer: usize,
        previous_pattern: &mut Option<Regex>,
    ) -> Result<Cmd, Error> {
        let address = eval_address(cmd_chars, &mut buffers[current_buffer], previous_pattern)?;
        parse_cmd(cmd_chars, previous_pattern, address)
    }
}

fn parse_cmd(
    cmd_chars: &mut Peekable<Chars>,
    _previous_pattern: &mut Option<Regex>,
    address: Option<Address>,
) -> Result<Cmd, Error> {
    let cmd = cmd_chars.next_if(|c| *c != '\r' && *c != '\n');
    match cmd {
        None => Ok(Cmd::Null(address)),
        Some('q') => parse_quit_cmd(cmd_chars, address),
        Some('p') => parse_print_cmd(cmd_chars, address),
        Some('a') => parse_append_cmd(cmd_chars, address),
        Some('d') => parse_delete_cmd(cmd_chars, address),
        Some('u') => parse_undo_cmd(cmd_chars, address),
        Some(c) => Err(Error::Unknown(c)),
    }
}

fn parse_quit_cmd(cmd_chars: &mut Peekable<Chars>, address: Option<Address>) -> Result<Cmd, Error> {
    address.map_or_else(
        || match cmd_chars.peek() {
            None | Some('\n') | Some('\r') => Ok(Cmd::Quit),
            _ => Err(Error::InvalidCmdSuffix),
        },
        |_| Err(Error::UnexpectedAddress),
    )
}

fn parse_print_cmd(
    cmd_chars: &mut Peekable<Chars>,
    address: Option<Address>,
) -> Result<Cmd, Error> {
    match cmd_chars.peek() {
        None | Some('\n') => Ok(Cmd::Print(address)),
        Some('\r') => {
            cmd_chars.next();
            parse_print_cmd(cmd_chars, address)
        }
        _ => Err(Error::InvalidCmdSuffix),
    }
}

fn parse_append_cmd(
    cmd_chars: &mut Peekable<Chars>,
    address: Option<Address>,
) -> Result<Cmd, Error> {
    match cmd_chars.peek() {
        None | Some('\n') => Ok(Cmd::Append(address, Vec::new())),
        Some('\r') => {
            cmd_chars.next();
            parse_append_cmd(cmd_chars, address)
        }
        _ => Err(Error::InvalidCmdSuffix),
    }
}

fn parse_delete_cmd(
    cmd_chars: &mut Peekable<Chars>,
    address: Option<Address>,
) -> Result<Cmd, Error> {
    match cmd_chars.peek() {
        None | Some('\n') => Ok(Cmd::Delete(address)),
        Some('\r') => {
            cmd_chars.next();
            parse_delete_cmd(cmd_chars, address)
        }
        _ => Err(Error::InvalidCmdSuffix),
    }
}

fn parse_undo_cmd(cmd_chars: &mut Peekable<Chars>, address: Option<Address>) -> Result<Cmd, Error> {
    address.map_or_else(
        || match cmd_chars.peek() {
            None | Some('\n') | Some('\r') => Ok(Cmd::Undo),
            _ => Err(Error::InvalidCmdSuffix),
        },
        |_| Err(Error::UnexpectedAddress),
    )
}

#[derive(Debug, PartialEq, Copy, Clone, Hash)]
pub enum Address {
    Line(usize),
    Span(usize, usize),
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum Separator {
    Comma,
    Semicolon,
}

pub fn eval_address(
    cmd_chars: &mut Peekable<Chars>,
    buffer: &mut EditBuffer,
    previous_pattern: &mut Option<Regex>,
) -> Result<Option<Address>, Error> {
    let addr = eval_line_addr(cmd_chars, buffer, previous_pattern)?;
    let separator = parse_separator(cmd_chars);
    match separator {
        None => Ok(addr.map(Address::Line)),
        Some(sep) => Ok(Some(eval_addr_chain(
            cmd_chars,
            buffer,
            addr,
            sep,
            previous_pattern,
        )?)),
    }
}

fn eval_addr_chain(
    cmd_chars: &mut Peekable<Chars>,
    buffer: &mut EditBuffer,
    left: Option<usize>,
    separator: Separator,
    previous_pattern: &mut Option<Regex>,
) -> Result<Address, Error> {
    // set current_line if left has a value
    if let Some(left) = left {
        if separator == Separator::Semicolon {
            if left == 0 || left > buffer.len() {
                return Err(Error::InvalidLineNumber);
            } else {
                buffer.set_current_line(left);
            }
        }
    }

    let right = eval_line_addr(cmd_chars, buffer, previous_pattern)?
        .unwrap_or_else(|| left.unwrap_or_else(|| buffer.len()));
    let left = left.unwrap_or_else(|| match separator {
        Separator::Semicolon => buffer.current_line(),
        Separator::Comma => 1,
    });

    let next_separator = parse_separator(cmd_chars);

    Ok(match next_separator {
        None => Address::Span(left, right),
        Some(separator) => {
            eval_addr_chain(cmd_chars, buffer, Some(right), separator, previous_pattern)?
        }
    })
}

fn parse_separator(cmd_chars: &mut Peekable<Chars>) -> Option<Separator> {
    match cmd_chars.peek() {
        Some(c) if c.is_blank() => {
            cmd_chars.next();
            parse_separator(cmd_chars)
        }
        Some(',') => {
            cmd_chars.next();
            Some(Separator::Comma)
        }
        Some(';') => {
            cmd_chars.next();
            Some(Separator::Semicolon)
        }
        _ => None,
    }
}

fn eval_line_addr(
    cmd_chars: &mut Peekable<Chars>,
    buffer: &EditBuffer,
    previous_pattern: &mut Option<Regex>,
) -> Result<Option<usize>, Error> {
    match cmd_chars.peek() {
        Some(c) if c.is_blank() => {
            cmd_chars.next();
            eval_line_addr(cmd_chars, buffer, previous_pattern)
        }
        Some('.') => {
            cmd_chars.next();
            let offset = eval_addr_offsets(cmd_chars)?;
            let line = buffer
                .current_line()
                .checked_add_signed(offset)
                .ok_or(Error::OffsetOverflow)?;
            Ok(Some(line))
        }
        Some('$') => {
            cmd_chars.next();
            let offset = eval_addr_offsets(cmd_chars)?;
            let line = buffer
                .len()
                .checked_add_signed(offset)
                .ok_or(Error::OffsetOverflow)?;
            Ok(Some(line))
        }
        Some('/') => {
            let pattern = parse_pattern(cmd_chars)?;
            if !pattern.is_empty() {
                *previous_pattern = Some(Regex::new(&pattern).map_err(Error::Regex)?);
            }
            let re = previous_pattern.as_ref().ok_or(Error::NoPreviousPattern)?;
            let offset = eval_addr_offsets(cmd_chars)?;
            let line = if buffer.current_line() == buffer.len() {
                (1..=buffer.len()).find(|&i| re.is_match(&buffer[i]))
            } else {
                (buffer.current_line() + 1..=buffer.len())
                    .find(|&i| re.is_match(&buffer[i]))
                    .or_else(|| (1..=buffer.current_line()).find(|&i| re.is_match(&buffer[i])))
            }
            .ok_or(Error::NoMatchingLine)?;
            let line = line
                .checked_add_signed(offset)
                .ok_or(Error::OffsetOverflow)?;
            Ok(Some(line))
        }
        Some('?') => {
            let pattern = parse_pattern(cmd_chars)?;
            if !pattern.is_empty() {
                *previous_pattern = Some(Regex::new(&pattern).map_err(Error::Regex)?);
            }
            let re = previous_pattern.as_ref().ok_or(Error::NoPreviousPattern)?;
            let offset = eval_addr_offsets(cmd_chars)?;
            let line = if buffer.current_line() == 1 {
                (1..=buffer.len()).rev().find(|&i| re.is_match(&buffer[i]))
            } else {
                (1..=buffer.current_line() - 1)
                    .rev()
                    .find(|&i| re.is_match(&buffer[i]))
                    .or_else(|| {
                        (buffer.current_line()..=buffer.len())
                            .rev()
                            .find(|&i| re.is_match(&buffer[i]))
                    })
            }
            .ok_or(Error::NoMatchingLine)?;
            let line = line
                .checked_add_signed(offset)
                .ok_or(Error::OffsetOverflow)?;
            Ok(Some(line))
        }
        Some('0'..='9') => {
            let num = cmd_chars
                .peeking_take_while(|c| c.is_ascii_digit())
                .try_fold(0usize, |acc, c| {
                    c.to_digit(10)
                        .and_then(|d| acc.checked_mul(10).and_then(|n| n.checked_add(d as usize)))
                })
                .ok_or(Error::InvalidLineNumber)?;
            let offset = eval_addr_offsets(cmd_chars)?;
            let line = num
                .checked_add_signed(offset)
                .ok_or(Error::OffsetOverflow)?;
            Ok(Some(line))
        }
        Some('+' | '-') => {
            let offset = eval_addr_offsets(cmd_chars)?;
            let line = buffer
                .current_line()
                .checked_add_signed(offset)
                .ok_or(Error::InvalidLineNumber)?;
            if line > buffer.len() {
                Err(Error::InvalidLineNumber)
            } else {
                Ok(Some(line))
            }
        }
        _ => Ok(None),
    }
}

fn parse_pattern(cmd_chars: &mut Peekable<Chars>) -> Result<String, Error> {
    let delimiter = cmd_chars
        .next_if(|c| *c != '\n' && *c != '\r' && *c != ' ')
        .ok_or(Error::InvalidPatternDelimiter)?;
    let mut pattern = String::new();
    while let Some(c) = cmd_chars.next_if(|c| *c != '\n' && *c != '\r') {
        if c == delimiter {
            break;
        } else if c != '\\' {
            pattern.push(c);
        } else {
            let escaped_c = cmd_chars
                .next_if(|c| *c != 'r' && *c != '\n')
                .ok_or(Error::TrailingBackslash)?;
            if escaped_c != delimiter {
                pattern.push('\\');
            }
            pattern.push(escaped_c);
        }
    }
    Ok(pattern)
}

fn eval_addr_offsets(cmd_chars: &mut Peekable<Chars>) -> Result<isize, Error> {
    let mut total_offset = 0isize;
    while let Some(c) = cmd_chars.peek() {
        let offset = match c {
            ' ' | '\t' => {
                cmd_chars.next();
                None
            }
            '+' => {
                cmd_chars.next();
                Some(cmp::max(
                    1,
                    cmd_chars
                        .peeking_take_while(|c| c.is_ascii_digit())
                        .try_fold(0isize, |acc, c| {
                            c.to_digit(10).and_then(|d| {
                                acc.checked_mul(10).and_then(|n| n.checked_add(d as isize))
                            })
                        })
                        .ok_or(Error::OffsetTooLarge)?,
                ))
            }
            '-' => {
                cmd_chars.next();
                Some(cmp::min(
                    -1,
                    cmd_chars
                        .peeking_take_while(|c| c.is_ascii_digit())
                        .try_fold(0isize, |acc, c| {
                            c.to_digit(10).and_then(|d| {
                                acc.checked_mul(10).and_then(|n| n.checked_sub(d as isize))
                            })
                        })
                        .ok_or(Error::OffsetTooSmall)?,
                ))
            }
            '0'..='9' => Some(
                cmd_chars
                    .peeking_take_while(|c| c.is_ascii_digit())
                    .try_fold(0isize, |acc, c| {
                        c.to_digit(10).and_then(|d| {
                            acc.checked_mul(10).and_then(|n| n.checked_add(d as isize))
                        })
                    })
                    .ok_or(Error::OffsetTooLarge)?,
            ),
            _ => break,
        };
        if let Some(offset) = offset {
            total_offset = total_offset
                .checked_add(offset)
                .ok_or(Error::OffsetOverflow)?;
        }
    }
    Ok(total_offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_command_gives_error() {
        let mut input = "~n".chars().peekable();
        let mut buffers = vec![EditBuffer::new()];
        let mut previous_pattern: Option<Regex> = None;
        let res = Cmd::parse(&mut input, &mut buffers, 0, &mut previous_pattern)
            .expect_err("an error indicating an unknown command");
        assert!(matches!(res, Error::Unknown(_)));
    }

    #[test]
    fn null_cmd() {
        let mut buffers = vec![EditBuffer::from(vec!["1", "2", "3"])];
        buffers[0].set_current_line(2);
        let mut previous_pattern: Option<Regex> = None;
        let mut input = "\n".chars().peekable();
        let res = Cmd::parse(&mut input, &mut buffers, 0, &mut previous_pattern)
            .expect("a successful parse");
        assert_eq!(Cmd::Null(None), res);
    }

    #[test]
    fn null_cmd_crlf() {
        let mut input = "\r\n".chars().peekable();
        let mut buffers = vec![EditBuffer::from(vec!["1", "2", "3", "4", "5", "6"])];
        let mut previous_pattern: Option<Regex> = None;
        buffers[0].set_current_line(2);
        let res =
            Cmd::parse(&mut input, &mut buffers, 0, &mut previous_pattern).expect("parsed command");
        assert_eq!(Cmd::Null(None), res);
    }

    #[test]
    fn offset_only_null_cmd() {
        let mut input = "-\n".chars().peekable();
        let mut buffers = vec![EditBuffer::from(vec!["1", "2", "3"])];
        let mut previous_pattern: Option<Regex> = None;
        assert_eq!(3, buffers[0].current_line());
        let res =
            Cmd::parse(&mut input, &mut buffers, 0, &mut previous_pattern).expect("parsed command");
        assert_eq!(Cmd::Null(Some(Address::Line(2))), res);
    }

    #[test]
    fn quit() {
        let mut buffers = vec![EditBuffer::new()];
        let mut input = "q\n".chars().peekable();
        let mut previous_pattern: Option<Regex> = None;
        let res = Cmd::parse(&mut input, &mut buffers, 0, &mut previous_pattern)
            .expect("a successful parse");
        assert_eq!(Cmd::Quit, res);
    }

    #[test]
    fn quit_with_illegal_addr() {
        let mut input = "2,3q\n".chars().peekable();
        let mut buffers = vec![EditBuffer::from(vec!["1", "2", "3", "4"])];
        let mut previous_pattern: Option<Regex> = None;
        let res = Cmd::parse(&mut input, &mut buffers, 0, &mut previous_pattern)
            .expect_err("unexpected addr on quit");
        assert_eq!(Error::UnexpectedAddress, res);
    }

    #[test]
    fn quit_with_invalid_suffix() {
        let mut input = "q/more/\n".chars().peekable();
        let mut buffers = vec![EditBuffer::from(vec!["1", "2", "3", "4"])];
        let mut previous_pattern: Option<Regex> = None;
        let res = Cmd::parse(&mut input, &mut buffers, 0, &mut previous_pattern)
            .expect_err("invalid command suffix");
        assert_eq!(Error::InvalidCmdSuffix, res);
    }

    #[test]
    fn print_cmd() {
        let mut input = "p\r\n".chars().peekable();
        let mut buffers = vec![EditBuffer::from(vec!["1", "2", "3"])];
        let mut previous_pattern: Option<Regex> = None;
        let res = Cmd::parse(&mut input, &mut buffers, 0, &mut previous_pattern)
            .expect("parsed print cmd");
        assert_eq!(Cmd::Print(None), res);
    }

    #[test]
    fn print_cmd_with_invald_suffix() {
        let mut input = "p/more/\r\n".chars().peekable();
        let mut buffers = vec![EditBuffer::from(vec!["1", "2", "3"])];
        let mut previous_pattern: Option<Regex> = None;
        let res = Cmd::parse(&mut input, &mut buffers, 0, &mut previous_pattern)
            .expect_err("invalid suffix");
        assert_eq!(Error::InvalidCmdSuffix, res);
    }

    #[test]
    fn append_cmd() {
        let mut input = "a\r\n".chars().peekable();
        let mut buffers = vec![EditBuffer::from(vec!["1\r\n", "2", "3"])];
        let mut previous_pattern: Option<Regex> = None;
        let res =
            Cmd::parse(&mut input, &mut buffers, 0, &mut previous_pattern).expect("parsed cmd");
        assert_eq!(Cmd::Append(None, Vec::new()), res);
    }

    #[test]
    fn append_cmd_with_invalid_suffix() {
        let mut input = "a/this is invalid/\n".chars().peekable();
        let mut buffers = vec![EditBuffer::from(vec!["1\r\n", "2", "3"])];
        let mut previous_pattern: Option<Regex> = None;
        let res = Cmd::parse(&mut input, &mut buffers, 0, &mut previous_pattern)
            .expect_err("invalid suffix");
        assert_eq!(Error::InvalidCmdSuffix, res)
    }

    #[test]
    fn delete_cmd() {
        let mut input = "d\r\n".chars().peekable();
        let mut buffers = vec![EditBuffer::from(vec!["1\r\n", "2", "3"])];
        let mut previous_pattern: Option<Regex> = None;
        let res =
            Cmd::parse(&mut input, &mut buffers, 0, &mut previous_pattern).expect("parsed cmd");
        assert_eq!(Cmd::Delete(None), res);
    }

    #[test]
    fn delete_cmd_with_invalid_suffix() {
        let mut input = "d/this is invalid/\n".chars().peekable();
        let mut buffers = vec![EditBuffer::from(vec!["1\r\n", "2", "3"])];
        let mut previous_pattern: Option<Regex> = None;
        let res = Cmd::parse(&mut input, &mut buffers, 0, &mut previous_pattern)
            .expect_err("invalid suffix");
        assert_eq!(Error::InvalidCmdSuffix, res)
    }

    #[test]
    fn undo_cmd() {
        let mut input = "u\r\n".chars().peekable();
        let mut buffers = vec![EditBuffer::from(vec!["1\r\n", "2", "3"])];
        let mut previous_pattern: Option<Regex> = None;
        let res =
            Cmd::parse(&mut input, &mut buffers, 0, &mut previous_pattern).expect("parsed cmd");
        assert_eq!(Cmd::Undo, res);
    }

    #[test]
    fn undo_cmd_with_invalid_suffix() {
        let mut input = "u/this is invalid/\n".chars().peekable();
        let mut buffers = vec![EditBuffer::from(vec!["1\r\n", "2", "3"])];
        let mut previous_pattern: Option<Regex> = None;
        let res = Cmd::parse(&mut input, &mut buffers, 0, &mut previous_pattern)
            .expect_err("invalid suffix");
        assert_eq!(Error::InvalidCmdSuffix, res)
    }

    #[test]
    fn single_addr_offset() {
        let mut input = "2n".chars().peekable();
        let res = eval_addr_offsets(&mut input).unwrap();
        assert_eq!(2, res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn single_plus_addr_offset() {
        let mut input = "+3n".chars().peekable();
        let res = eval_addr_offsets(&mut input).unwrap();
        assert_eq!(3, res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn single_negative_addr_offset() {
        let mut input = "-4n".chars().peekable();
        let res = eval_addr_offsets(&mut input).unwrap();
        assert_eq!(-4, res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn multiple_negative_addr_offset() {
        let mut input = "---2n".chars().peekable();
        let res = eval_addr_offsets(&mut input).unwrap();
        assert_eq!(-4, res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn combined_addr_offsets() {
        let mut input = " +4++ 5 6-6   -7 +8---n".chars().peekable();
        let res = eval_addr_offsets(&mut input).unwrap();
        assert_eq!(9, res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn minus_num_addr_offsets() {
        let mut input = "-2-+1n".chars().peekable();
        let res = eval_addr_offsets(&mut input).unwrap();
        assert_eq!(-2, res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn addr_offsets_trailing_minus() {
        let mut input = "-4-n".chars().peekable();
        let res = eval_addr_offsets(&mut input).unwrap();
        assert_eq!(-5, res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn dot_line_addr() {
        let buffer = EditBuffer::from(vec!["one", "two", "three"]);
        let mut input = ".n".chars().peekable();
        let mut previous_pattern: Option<Regex> = None;
        let res = eval_line_addr(&mut input, &buffer, &mut previous_pattern)
            .expect("successful line addr eval");
        assert_eq!(Some(buffer.current_line()), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn dot_line_addr_with_spaces() {
        let buffer = EditBuffer::from(vec!["one", "two", "three"]);
        let mut input = "   .  n".chars().peekable();
        let mut previous_pattern: Option<Regex> = None;
        let res = eval_line_addr(&mut input, &buffer, &mut previous_pattern)
            .expect("successful line addr eval");
        assert_eq!(Some(buffer.current_line()), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn dot_line_addr_with_offset() {
        let mut input = ".+2n".chars().peekable();
        let mut buffer = EditBuffer::from(vec!["one", "two", "three", "four"]);
        buffer.set_current_line(2usize);
        let mut previous_pattern: Option<Regex> = None;
        let res = eval_line_addr(&mut input, &buffer, &mut previous_pattern).expect("no error");
        assert_eq!(Some(4usize), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn dollar_line_addr_empty_buffer() {
        let buffer = EditBuffer::new();
        let mut input = "$n".chars().peekable();
        let mut previous_pattern: Option<Regex> = None;
        let res = eval_line_addr(&mut input, &buffer, &mut previous_pattern)
            .expect("successful eval of line addr");
        assert_eq!(Some(0usize), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn dollar_line_addr_() {
        let buffer = EditBuffer::from(vec!["one", "two", "three", "four", "five", "six"]);
        let mut input = "$n".chars().peekable();
        let mut previous_pattern: Option<Regex> = None;
        let res = eval_line_addr(&mut input, &buffer, &mut previous_pattern)
            .expect("successful eval of line addr");
        assert_eq!(Some(6usize), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn dollar_line_addr_with_offset() {
        let mut buffer = EditBuffer::from(vec!["one", "two", "three", "four", "five", "six"]);
        buffer.set_current_line(3);
        let mut input = "$-2n".chars().peekable();
        let mut previous_pattern: Option<Regex> = None;
        let res = eval_line_addr(&mut input, &buffer, &mut previous_pattern)
            .expect("successful eval of line addr");
        assert_eq!(Some(4usize), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn regex_line_addr_regex_syntax() {
        let mut input = "/\\lo.+/n\n".chars().peekable();
        let mut buffer = EditBuffer::from(vec!["one", "two", "three", "four", "five", "six"]);
        buffer.set_current_line(2);
        let mut previous_pattern: Option<Regex> = None;
        let _res =
            eval_line_addr(&mut input, &buffer, &mut previous_pattern).expect_err("bad pattern");
        assert!(matches!(Error::Regex, _res));
    }

    #[test]
    fn rev_regex_line_addr_regex_syntax() {
        let mut input = "?\\lo.+?n\n".chars().peekable();
        let mut buffer = EditBuffer::from(vec!["one", "two", "three", "four", "five", "six"]);
        buffer.set_current_line(2);
        let mut previous_pattern: Option<Regex> = None;
        let _res =
            eval_line_addr(&mut input, &buffer, &mut previous_pattern).expect_err("bad pattern");
        assert!(matches!(Error::Regex, _res));
    }

    #[test]
    fn regex_line_addr_embedded_delim() {
        let mut input = "/o.+\\//n\n".chars().peekable();
        let mut buffer = EditBuffer::from(vec!["one/", "two", "three", "four", "five", "six"]);
        buffer.set_current_line(2);
        let mut previous_pattern: Option<Regex> = None;
        let res =
            eval_line_addr(&mut input, &buffer, &mut previous_pattern).expect("pattern found");
        assert_eq!(Some(1), res);
    }

    #[test]
    fn regex_line_addr_no_final_delimiter() {
        let mut input = "/o.+\n".chars().peekable();
        let mut buffer = EditBuffer::from(vec!["one", "two", "three", "four", "five", "six"]);
        buffer.set_current_line(2);
        let mut previous_pattern: Option<Regex> = None;
        let res =
            eval_line_addr(&mut input, &buffer, &mut previous_pattern).expect("pattern found");
        assert_eq!(Some(4), res);
    }

    #[test]
    fn regex_line_addr_needle_in_first_half_of_split_range() {
        let mut input = "/o.+/n\n".chars().peekable();
        let mut previous_pattern: Option<Regex> = None;
        let mut buffer = EditBuffer::from(vec!["one", "two", "three", "four", "five", "six"]);
        buffer.set_current_line(2);
        let res =
            eval_line_addr(&mut input, &buffer, &mut previous_pattern).expect("pattern found");
        assert_eq!(Some(4), res);
    }

    #[test]
    fn regex_line_addr_needle_in_second_half_of_split_range() {
        let mut input = "/on.+/n\n".chars().peekable();
        let mut previous_pattern: Option<Regex> = None;
        let mut buffer = EditBuffer::from(vec!["one", "two", "three", "four", "five", "six"]);
        buffer.set_current_line(4);
        let res =
            eval_line_addr(&mut input, &buffer, &mut previous_pattern).expect("pattern found");
        assert_eq!(Some(1), res);
    }

    #[test]
    fn regex_line_addr_contiguous_search_range() {
        let mut input = "/o.+/n\n".chars().peekable();
        let mut previous_pattern: Option<Regex> = None;
        let mut buffer = EditBuffer::from(vec!["one", "two", "three", "four", "five", "six"]);
        buffer.set_current_line(6);
        let res =
            eval_line_addr(&mut input, &buffer, &mut previous_pattern).expect("pattern found");
        assert_eq!(Some(1), res);
    }

    #[test]
    fn rev_regex_line_addr_needle_in_first_half_of_split_range() {
        let mut input = "?o.+?n\n".chars().peekable();
        let mut previous_pattern: Option<Regex> = None;
        let mut buffer = EditBuffer::from(vec!["one", "two", "three", "four", "five", "six"]);
        buffer.set_current_line(2);
        let res =
            eval_line_addr(&mut input, &buffer, &mut previous_pattern).expect("pattern found");
        assert_eq!(Some(1), res);
    }

    #[test]
    fn rev_regex_line_addr_needle_in_second_half_of_split_range() {
        let mut input = "?ou.+?n\n".chars().peekable();
        let mut previous_pattern: Option<Regex> = None;
        let mut buffer = EditBuffer::from(vec!["one", "two", "three", "four", "five", "six"]);
        buffer.set_current_line(4);
        let res =
            eval_line_addr(&mut input, &buffer, &mut previous_pattern).expect("pattern found");
        assert_eq!(Some(4), res);
    }

    #[test]
    fn rev_regex_line_addr_contiguous_search_range() {
        let mut input = "?o.+?n\n".chars().peekable();
        let mut previous_pattern: Option<Regex> = None;
        let mut buffer = EditBuffer::from(vec!["one", "two", "three", "four", "five", "six"]);
        buffer.set_current_line(1);
        let res =
            eval_line_addr(&mut input, &buffer, &mut previous_pattern).expect("pattern found");
        assert_eq!(Some(4), res);
    }

    #[test]
    fn regex_line_addr_with_offset() {
        let mut input = "/o.+/+2\n".chars().peekable();
        let mut buffer = EditBuffer::from(vec!["one", "two", "three", "four", "five", "six"]);
        buffer.set_current_line(2);
        let mut previous_pattern: Option<Regex> = None;
        let res =
            eval_line_addr(&mut input, &buffer, &mut previous_pattern).expect("pattern found");
        assert_eq!(Some(6), res);
    }

    #[test]
    fn rev_regex_line_addr_with_offset() {
        let mut input = "?o.+?+2\n".chars().peekable();
        let mut buffer = EditBuffer::from(vec!["one", "two", "three", "four", "five", "six"]);
        buffer.set_current_line(2);
        let mut previous_pattern: Option<Regex> = None;
        let res =
            eval_line_addr(&mut input, &buffer, &mut previous_pattern).expect("pattern found");
        assert_eq!(Some(3), res);
    }

    #[test]
    fn plus_line_addr() {
        let mut input = "+n".chars().peekable();
        let mut buffer = EditBuffer::from(vec!["one", "two", "three"]);
        buffer.set_current_line(2);
        let mut previous_pattern: Option<Regex> = None;
        let res = eval_line_addr(&mut input, &buffer, &mut previous_pattern)
            .expect("successful line addr eval");
        assert_eq!(Some(3usize), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn plus_line_addr_overflow() {
        let mut input = "+n".chars().peekable();
        let buffer = EditBuffer::from(vec!["one", "two", "three"]);
        let mut previous_pattern: Option<Regex> = None;
        let _res = eval_line_addr(&mut input, &buffer, &mut previous_pattern)
            .expect_err("should overflow");
        assert!(matches!(Error::InvalidLineNumber, _res));
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn plus_num_line_addr() {
        let mut buffer = EditBuffer::from(vec!["one", "two", "three"]);
        buffer.set_current_line(1usize);
        let mut input = "+2n".chars().peekable();
        let mut previous_pattern: Option<Regex> = None;
        let res = eval_line_addr(&mut input, &buffer, &mut previous_pattern)
            .expect("successful line addr eval");
        assert_eq!(Some(3usize), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn plus_line_addr_with_offsets() {
        let mut input = "+++3n".chars().peekable();
        let mut buffer =
            EditBuffer::from(vec!["one", "two", "three", "four", "five", "six", "seven"]);
        buffer.set_current_line(2usize);
        let mut previous_pattern: Option<Regex> = None;
        let res = eval_line_addr(&mut input, &buffer, &mut previous_pattern)
            .expect("successful line_addr eval");
        assert_eq!(Some(7usize), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn plus_num_line_addr_with_offsets() {
        let buffer = EditBuffer::from(vec!["one", "two", "three"]);
        let mut input = "+2--1n".chars().peekable();
        let mut previous_pattern: Option<Regex> = None;
        let res = eval_line_addr(&mut input, &buffer, &mut previous_pattern)
            .expect("successful line addr eval");
        assert_eq!(Some(3usize), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn minus_line_addr_with_offsets() {
        let buffer = EditBuffer::from(vec!["one", "two", "three", "four", "five", "six"]);
        let mut input = "---2n".chars().peekable();
        let mut previous_pattern: Option<Regex> = None;
        let res = eval_line_addr(&mut input, &buffer, &mut previous_pattern)
            .expect("successful line addr eval");
        assert_eq!(Some(2usize), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn minus_num_line_addr_with_offsets() {
        let buffer = EditBuffer::from(vec!["one", "two", "three"]);
        let mut input = "-2-+1n".chars().peekable();
        let mut previous_pattern: Option<Regex> = None;
        let res = eval_line_addr(&mut input, &buffer, &mut previous_pattern)
            .expect("successful line addr eval");
        assert_eq!(Some(1usize), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn num_line_addr() {
        let buffer = EditBuffer::from(vec!["one", "two", "three"]);
        let mut input = "2n".chars().peekable();
        let mut previous_pattern: Option<Regex> = None;
        let res = eval_line_addr(&mut input, &buffer, &mut previous_pattern)
            .expect("successful eval of line addr");
        assert_eq!(Some(2usize), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn num_line_addr_with_offsets() {
        let buffer = EditBuffer::new();
        let mut input = "2++5-n".chars().peekable();
        let mut previous_pattern: Option<Regex> = None;
        let res = eval_line_addr(&mut input, &buffer, &mut previous_pattern)
            .expect("successful eval of line addr");
        assert_eq!(Some(7usize), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn comma_addr_separator() {
        let mut input = ",n".chars().peekable();
        let _res = parse_separator(&mut input);
        assert_eq!(Some(Separator::Comma), _res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn semicolon_addr_separator() {
        let mut input = ";n".chars().peekable();
        let _res = parse_separator(&mut input);
        assert_eq!(Some(Separator::Semicolon), _res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn semicolon_addr_separator_with_spaces() {
        let mut input = "  ;n".chars().peekable();
        let _res = parse_separator(&mut input);
        assert_eq!(Some(Separator::Semicolon), _res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn empty_addr_chain() {
        let mut input = "p\n".chars().peekable();
        let mut buffer = EditBuffer::from(vec!["one", "two", "three"]);
        let mut previous_pattern: Option<Regex> = None;
        let res = eval_address(&mut input, &mut buffer, &mut previous_pattern)
            .expect("evaluated address");
        assert!(res.is_none());
        assert_eq!("p\n", input.collect::<String>());
    }

    #[test]
    fn multi_spearator_addr_chain() {
        let mut input = " 1,2 ; $n".chars().peekable();
        let mut buffer = EditBuffer::from(vec!["1\n", "2", "3"]);
        let mut previous_pattern: Option<Regex> = None;
        let res = eval_address(&mut input, &mut buffer, &mut previous_pattern)
            .expect("evaluated address");
        assert_eq!(2, buffer.current_line());
        assert_eq!(Some(Address::Span(2, 3)), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn comma_addr_chain() {
        let mut input = ",n".chars().peekable();
        let mut buffer = EditBuffer::from(vec!["one", "two", "three"]);
        buffer.set_current_line(2);
        let mut previous_pattern: Option<Regex> = None;
        let res = eval_address(&mut input, &mut buffer, &mut previous_pattern)
            .expect("evaluated address");
        assert_eq!(2, buffer.current_line());
        assert_eq!(Some(Address::Span(1, 3)), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn comma_left_addr_chain() {
        let mut input = "3,n".chars().peekable();
        let mut buffer = EditBuffer::from(vec!["1", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(2);
        let mut previous_pattern: Option<Regex> = None;
        let res = eval_address(&mut input, &mut buffer, &mut previous_pattern)
            .expect("evaluated address");
        assert_eq!(2, buffer.current_line());
        assert_eq!(Some(Address::Span(3, 3)), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn comma_right_addr_chain() {
        let mut input = ",5n".chars().peekable();
        let mut buffer = EditBuffer::from(vec!["1", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(4);
        let mut previous_pattern: Option<Regex> = None;
        let res = eval_address(&mut input, &mut buffer, &mut previous_pattern)
            .expect("evaluated address");
        assert_eq!(4, buffer.current_line());
        assert_eq!(Some(Address::Span(1, 5)), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn comma_full_addr_chain() {
        let mut input = "2,5n".chars().peekable();
        let mut buffer = EditBuffer::from(vec!["1", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(4);
        let mut previous_pattern: Option<Regex> = None;
        let res = eval_address(&mut input, &mut buffer, &mut previous_pattern)
            .expect("evaluated address");
        assert_eq!(4, buffer.current_line());
        assert_eq!(Some(Address::Span(2, 5)), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn semicolon_addr_chain() {
        let mut input = ";n".chars().peekable();
        let mut buffer = EditBuffer::from(vec!["one", "two", "three"]);
        buffer.set_current_line(2);
        let mut previous_pattern: Option<Regex> = None;
        let res = eval_address(&mut input, &mut buffer, &mut previous_pattern)
            .expect("evaluated address");
        assert_eq!(2, buffer.current_line());
        assert_eq!(Some(Address::Span(2, 3)), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn semicolon_addr_chain_current_line_last() {
        let mut input = ";n".chars().peekable();
        let mut buffer = EditBuffer::from(vec!["one", "two", "three"]);
        let mut previous_pattern: Option<Regex> = None;
        let res = eval_address(&mut input, &mut buffer, &mut previous_pattern)
            .expect("evaluated address");
        assert_eq!(3, buffer.current_line());
        assert_eq!(Some(Address::Span(3, 3)), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn semicolon_addr_chain_with_offsets() {
        let mut input = "$-4;+3n".chars().peekable();
        let mut buffer = EditBuffer::from(vec!["1", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(5);
        let mut previous_pattern: Option<Regex> = None;
        let res = eval_address(&mut input, &mut buffer, &mut previous_pattern)
            .expect("evaluated address");
        assert_eq!(2, buffer.current_line());
        assert_eq!(Some(Address::Span(2, 5)), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn semicolon_left_addr_chain() {
        let mut input = "3;n".chars().peekable();
        let mut buffer = EditBuffer::from(vec!["1", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(2);
        let mut previous_pattern: Option<Regex> = None;
        let res = eval_address(&mut input, &mut buffer, &mut previous_pattern)
            .expect("evaluated address");
        assert_eq!(3, buffer.current_line());
        assert_eq!(Some(Address::Span(3, 3)), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn semicolon_left_addr_chain_line_zero() {
        let mut input = "0;n".chars().peekable();
        let mut buffer = EditBuffer::from(vec!["1", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(2);
        let mut previous_pattern: Option<Regex> = None;
        let res = eval_address(&mut input, &mut buffer, &mut previous_pattern)
            .expect_err("invalid line number");
        assert_eq!(Error::InvalidLineNumber, res);
    }

    #[test]
    fn semicolon_right_addr_chain() {
        let mut input = ";5n".chars().peekable();
        let mut buffer = EditBuffer::from(vec!["1", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(3);
        let mut previous_pattern: Option<Regex> = None;
        let res = eval_address(&mut input, &mut buffer, &mut previous_pattern)
            .expect("evaluated address");
        assert_eq!(3, buffer.current_line());
        assert_eq!(Some(Address::Span(3, 5)), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn semicolon_full_addr_chain() {
        let mut input = "2;10n".chars().peekable();
        let mut buffer = EditBuffer::from(vec![
            "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten", "11",
        ]);
        assert_eq!(11usize, buffer.current_line());
        let mut previous_pattern: Option<Regex> = None;
        let res = eval_address(&mut input, &mut buffer, &mut previous_pattern)
            .expect("successful address eval");
        assert_eq!(2usize, buffer.current_line());
        assert_eq!(Some(Address::Span(2usize, 10usize)), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn offset_too_large() {
        let mut input = "999999999999999999999999999".chars().peekable();
        let _res = eval_addr_offsets(&mut input).expect_err("should be an error");
    }

    #[test]
    fn offset_too_small() {
        let mut input = "-999999999999999999999999999".chars().peekable();
        let _res = eval_addr_offsets(&mut input).expect_err("an error");
        assert!(matches!(Error::OffsetTooSmall, _res));
    }

    #[test]
    fn eval_line_addr_propegates_errors() {
        let mut input = ".+9999999999999999999999999999n".chars().peekable();
        let mut buffer = EditBuffer::from(vec!["one", "two", "three"]);
        buffer.set_current_line(1);
        let mut previous_pattern: Option<Regex> = None;
        let res =
            eval_line_addr(&mut input, &buffer, &mut previous_pattern).expect_err("OffsetTooLarge");
        assert_eq!(Error::OffsetTooLarge, res);
    }

    #[test]
    fn eval_addr_chain_propegates_errors() {
        let mut buffer = EditBuffer::from(vec!["one", "two", "three"]);
        buffer.set_current_line(1usize);
        let mut input = ".+9999999999999999999999999999n".chars().peekable();
        let mut previous_pattern: Option<Regex> = None;
        let res = eval_addr_chain(
            &mut input,
            &mut buffer,
            None,
            Separator::Comma,
            &mut previous_pattern,
        )
        .expect_err("OffsetTooLarge");
        assert_eq!(Error::OffsetTooLarge, res);
    }

    /////
    // parse_pattern tests

    #[test]
    fn parse_pattern_invalid_delimiter() {
        let mut input = " stuff + other_stuff. \n".chars().peekable();
        let res = parse_pattern(&mut input);
        assert_eq!(Err(Error::InvalidPatternDelimiter), res);
    }

    #[test]
    fn parse_pattern_trailing_backslash() {
        let mut input = "/stuff + other_stuff.\\\n".chars().peekable();
        let res = parse_pattern(&mut input);
        assert_eq!(Err(Error::TrailingBackslash), res);
        let mut input = "/stuff + other_stuff.\\".chars().peekable();
        let res = parse_pattern(&mut input);
        assert_eq!(Err(Error::TrailingBackslash), res);
    }

    #[test]
    fn parse_pattern_no_terminating_delimiter() {
        let mut input = "/stuff\\/other_stuff.\n".chars().peekable();
        let res = parse_pattern(&mut input).expect("parsed pattern");
        assert_eq!("stuff/other_stuff.".to_owned(), res);
    }

    #[test]
    fn parse_pattern_escaped_terminator() {
        let mut input = "/stuff\\/other_stuff./\n".chars().peekable();
        let res = parse_pattern(&mut input).expect("parsed pattern");
        assert_eq!("stuff/other_stuff.".to_owned(), res);
    }

    #[test]
    fn parse_pattern_escaped_chars() {
        let mut input = "?stuff \\+ other_stuff\\.?\n".chars().peekable();
        let res = parse_pattern(&mut input).expect("parsed pattern");
        assert_eq!("stuff \\+ other_stuff\\.".to_owned(), res);
    }

    #[test]
    fn parse_pattern_no_escaped_chars() {
        let mut input = "/stuff + other_stuff./\n".chars().peekable();
        let res = parse_pattern(&mut input).expect("parsed pattern");
        assert_eq!("stuff + other_stuff.".to_owned(), res);
    }
}
