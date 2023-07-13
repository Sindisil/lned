use std::cmp;
use std::fmt;
use std::fmt::Debug;
use std::iter;
use std::str;

use crate::char_utils::CharUtils;
use crate::edit_buffer::EditBuffer;
use crate::peeking::Peeking;

#[derive(Debug, PartialEq)]
pub enum Cmd {
    Quit,
    Null(Option<Address>),
    Print(Option<Address>),
}

#[derive(Debug, PartialEq)]
pub enum Error {
    Unknown(String),
    UnexpectedAddress,
    OffsetTooLarge,
    OffsetTooSmall,
    OffsetOverflow,
    InvalidLineNumber,
}

impl std::error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::UnexpectedAddress => write!(f, "Command takes no line address."),
            Error::Unknown(s) => write!(f, "Unknown command '{s}'"),
            Error::OffsetTooLarge => write!(f, "Offset too large"),
            Error::OffsetOverflow => write!(f, "Offset results in invalid line number"),
            Error::OffsetTooSmall => write!(f, "Offset too small"),
            Error::InvalidLineNumber => write!(f, "invalid line number"),
        }
    }
}

impl Cmd {
    pub fn parse(
        cmd_chars: &mut iter::Peekable<str::Chars>,
        buffer: &mut EditBuffer,
    ) -> Result<Cmd, Error> {
        // eval address
        let address = eval_address(cmd_chars, buffer)?;
        match cmd_chars.peek() {
            None | Some('\n') | Some('\r') => Ok(Cmd::Null(address)),
            Some('q') => address.map_or(Ok(Cmd::Quit), |_| Err(Error::UnexpectedAddress)),
            _ => Err(Error::Unknown(cmd_chars.collect())),
        }
    }
}

#[derive(Debug, PartialEq)]
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
    cmd_chars: &mut iter::Peekable<str::Chars>,
    buffer: &mut EditBuffer,
) -> Result<Option<Address>, Error> {
    let addr = eval_line_addr(cmd_chars, buffer)?;
    let separator = parse_separator(cmd_chars);
    match separator {
        None => Ok(addr.map(Address::Line)),
        Some(sep) => Ok(Some(eval_addr_chain(cmd_chars, buffer, addr, sep)?)),
    }
}

fn eval_addr_chain(
    cmd_chars: &mut iter::Peekable<str::Chars>,
    buffer: &mut EditBuffer,
    left: Option<usize>,
    separator: Separator,
) -> Result<Address, Error> {
    // set current_line if left has a value
    match left {
        Some(left) if separator == Separator::Semicolon => buffer
            .set_current_line(left)
            .map_err(|_| Error::InvalidLineNumber)?,
        _ => (),
    }

    let right =
        eval_line_addr(cmd_chars, buffer)?.unwrap_or_else(|| left.unwrap_or_else(|| buffer.len()));
    let left = left.unwrap_or_else(|| match separator {
        Separator::Semicolon => buffer.current_line(),
        Separator::Comma => 1,
    });

    let next_separator = parse_separator(cmd_chars);

    Ok(match next_separator {
        None => Address::Span(left, right),
        Some(separator) => eval_addr_chain(cmd_chars, buffer, Some(right), separator)?,
    })
}

fn parse_separator(cmd_chars: &mut iter::Peekable<str::Chars>) -> Option<Separator> {
    match cmd_chars.peeking_skip_while(|c| c.is_blank()).peek() {
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
    cmd_chars: &mut iter::Peekable<str::Chars>,
    buffer: &EditBuffer,
) -> Result<Option<usize>, Error> {
    match cmd_chars.peeking_skip_while(|c| c.is_blank()).peek() {
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
            cmd_chars.next();
            let _re: String = cmd_chars.by_ref().take_while(|c| *c != '/').collect();
            let _offset = eval_addr_offsets(cmd_chars)?;
            todo!()
        }
        Some('?') => {
            cmd_chars.next();
            let _re: String = cmd_chars.by_ref().take_while(|c| *c != '?').collect();
            let _offset = eval_addr_offsets(cmd_chars)?;
            todo!()
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

fn eval_addr_offsets(cmd_chars: &mut iter::Peekable<str::Chars>) -> Result<isize, Error> {
    let mut total_offset = 0isize;
    while let Some(c) = cmd_chars.peek() {
        let offset = match c {
            ' ' | 't' => {
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
        let mut buffer = EditBuffer::new();
        let res = Cmd::parse(&mut input, &mut buffer)
            .err()
            .expect("an error indicating an unknown command");
        assert!(matches!(res, Error::Unknown(_)));
    }

    #[test]
    fn blank_cmd_line() {
        let mut buffer = EditBuffer::from(vec!["1", "2", "3"]);
        buffer.set_current_line(2).expect("current line set");
        let mut input = "\n".chars().peekable();
        let res = Cmd::parse(&mut input, &mut buffer).expect("a successful parse");
        assert_eq!(Cmd::Null(None), res);
    }

    #[test]
    fn blank_cmd_line_crlf() {
        let mut input = "\r\n".chars().peekable();
        let mut buffer = EditBuffer::from(vec!["1", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(2).expect("current line set");
        let res = Cmd::parse(&mut input, &mut buffer).expect("parsed command");
        assert_eq!(Cmd::Null(None), res);
    }

    #[test]
    fn offset_only_cmd() {
        let mut input = "-\n".chars().peekable();
        let mut buffer = EditBuffer::from(vec!["1", "2", "3"]);
        assert_eq!(3, buffer.current_line());
        let res = Cmd::parse(&mut input, &mut buffer).expect("parsed command");
        assert_eq!(Cmd::Null(Some(Address::Line(2))), res);
    }

    #[test]
    fn quit() {
        let mut buffer = EditBuffer::new();
        let mut input = "q\n".chars().peekable();
        let res = Cmd::parse(&mut input, &mut buffer).expect("a successful parse");
        assert_eq!(Cmd::Quit, res);
    }

    #[test]
    fn quit_with_illegal_addr() {
        let mut input = "2,3q\n".chars().peekable();
        let mut buffer = EditBuffer::from(vec!["1", "2", "3", "4"]);
        let res = Cmd::parse(&mut input, &mut buffer).expect_err("unexpected addr on quit");
        assert_eq!(Error::UnexpectedAddress, res);
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
        let res = eval_line_addr(&mut input, &buffer).expect("successful line addr eval");
        assert_eq!(Some(buffer.current_line()), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn dot_line_addr_with_offset() {
        let mut input = ".+2n".chars().peekable();
        let mut buffer = EditBuffer::from(vec!["one", "two", "three", "four"]);
        buffer.set_current_line(2usize).expect("no error");
        let res = eval_line_addr(&mut input, &buffer).expect("no error");
        assert_eq!(Some(4usize), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn dollar_line_addr_empty_buffer() {
        let buffer = EditBuffer::new();
        let mut input = "$n".chars().peekable();
        let res = eval_line_addr(&mut input, &buffer).expect("successful eval of line addr");
        assert_eq!(Some(0usize), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn dollar_line_addr_() {
        let buffer = EditBuffer::from(vec!["one", "two", "three", "four", "five", "six"]);
        let mut input = "$n".chars().peekable();
        let res = eval_line_addr(&mut input, &buffer).expect("successful eval of line addr");
        assert_eq!(Some(6usize), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn dollar_line_addr_with_offset() {
        let mut buffer = EditBuffer::from(vec!["one", "two", "three", "four", "five", "six"]);
        buffer.set_current_line(3).expect("current line set");
        let mut input = "$-2n".chars().peekable();
        let res = eval_line_addr(&mut input, &buffer).expect("successful eval of line addr");
        assert_eq!(Some(4usize), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    #[ignore = "todo!"]
    fn regex_line_addr() {
        todo!();
    }

    #[test]
    #[ignore = "todo!"]
    fn rev_regex_line_addr() {
        todo!();
    }

    #[test]
    #[ignore = "todo!"]
    fn regex_line_addr_with_offset() {
        todo!();
    }

    #[test]
    #[ignore = "todo!"]
    fn rev_regex_line_addr_with_offset() {
        todo!();
    }

    #[test]
    fn plus_line_addr() {
        let mut input = "+n".chars().peekable();
        let mut buffer = EditBuffer::from(vec!["one", "two", "three"]);
        buffer.set_current_line(2).expect("no error");
        let res = eval_line_addr(&mut input, &buffer).expect("successful line addr eval");
        assert_eq!(Some(3usize), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn plus_line_addr_overflow() {
        let mut input = "+n".chars().peekable();
        let buffer = EditBuffer::from(vec!["one", "two", "three"]);
        let _res = eval_line_addr(&mut input, &buffer).expect_err("should overflow");
        assert!(matches!(Error::InvalidLineNumber, _res));
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn plus_num_line_addr() {
        let mut buffer = EditBuffer::from(vec!["one", "two", "three"]);
        buffer.set_current_line(1usize).expect("no error");
        let mut input = "+2n".chars().peekable();
        let res = eval_line_addr(&mut input, &buffer).expect("successful line addr eval");
        assert_eq!(Some(3usize), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn plus_line_addr_with_offsets() {
        let mut input = "+++3n".chars().peekable();
        let mut buffer =
            EditBuffer::from(vec!["one", "two", "three", "four", "five", "six", "seven"]);
        buffer
            .set_current_line(2usize)
            .expect("no error setting current line");
        let res = eval_line_addr(&mut input, &buffer).expect("successful line_addr eval");
        assert_eq!(Some(7usize), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn plus_num_line_addr_with_offsets() {
        let buffer = EditBuffer::from(vec!["one", "two", "three"]);
        let mut input = "+2--1n".chars().peekable();
        let res = eval_line_addr(&mut input, &buffer).expect("successful line addr eval");
        assert_eq!(Some(3usize), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn minus_line_addr_with_offsets() {
        let buffer = EditBuffer::from(vec!["one", "two", "three", "four", "five", "six"]);
        let mut input = "---2n".chars().peekable();
        let res = eval_line_addr(&mut input, &buffer).expect("successful line addr eval");
        assert_eq!(Some(2usize), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn minus_num_line_addr_with_offsets() {
        let buffer = EditBuffer::from(vec!["one", "two", "three"]);
        let mut input = "-2-+1n".chars().peekable();
        let res = eval_line_addr(&mut input, &buffer).expect("successful line addr eval");
        assert_eq!(Some(1usize), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn num_line_addr() {
        let buffer = EditBuffer::from(vec!["one", "two", "three"]);
        let mut input = "2n".chars().peekable();
        let res = eval_line_addr(&mut input, &buffer).expect("successful eval of line addr");
        assert_eq!(Some(2usize), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn num_line_addr_with_offsets() {
        let buffer = EditBuffer::new();
        let mut input = "2++5-n".chars().peekable();
        let res = eval_line_addr(&mut input, &buffer).expect("successful eval of line addr");
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
    fn empty_addr_chain() {
        let mut input = "n".chars().peekable();
        let mut buffer = EditBuffer::from(vec!["one", "two", "three"]);
        let res = eval_address(&mut input, &mut buffer).expect("evaluated address");
        assert!(res.is_none());
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn comma_addr_chain() {
        let mut input = ",n".chars().peekable();
        let mut buffer = EditBuffer::from(vec!["one", "two", "three"]);
        buffer.set_current_line(2).expect("current line set");
        let res = eval_address(&mut input, &mut buffer).expect("evaluated address");
        assert_eq!(2, buffer.current_line());
        assert_eq!(Some(Address::Span(1, 3)), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn comma_left_addr_chain() {
        let mut input = "3,n".chars().peekable();
        let mut buffer = EditBuffer::from(vec!["1", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(2).expect("current line set");
        let res = eval_address(&mut input, &mut buffer).expect("evaluated address");
        assert_eq!(2, buffer.current_line());
        assert_eq!(Some(Address::Span(3, 3)), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn comma_right_addr_chain() {
        let mut input = ",5n".chars().peekable();
        let mut buffer = EditBuffer::from(vec!["1", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(4).expect("current line set");
        let res = eval_address(&mut input, &mut buffer).expect("evaluated address");
        assert_eq!(4, buffer.current_line());
        assert_eq!(Some(Address::Span(1, 5)), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn comma_full_addr_chain() {
        let mut input = "2,5n".chars().peekable();
        let mut buffer = EditBuffer::from(vec!["1", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(4).expect("current line set");
        let res = eval_address(&mut input, &mut buffer).expect("evaluated address");
        assert_eq!(4, buffer.current_line());
        assert_eq!(Some(Address::Span(2, 5)), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn semicolon_addr_chain() {
        let mut input = ";n".chars().peekable();
        let mut buffer = EditBuffer::from(vec!["one", "two", "three"]);
        buffer.set_current_line(2).expect("current line set");
        let res = eval_address(&mut input, &mut buffer).expect("evaluated address");
        assert_eq!(2, buffer.current_line());
        assert_eq!(Some(Address::Span(2, 3)), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn semicolon_addr_chain_current_line_last() {
        let mut input = ";n".chars().peekable();
        let mut buffer = EditBuffer::from(vec!["one", "two", "three"]);
        let res = eval_address(&mut input, &mut buffer).expect("evaluated address");
        assert_eq!(3, buffer.current_line());
        assert_eq!(Some(Address::Span(3, 3)), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn semicolon_addr_chain_with_offsets() {
        let mut input = "$-4;+3n".chars().peekable();
        let mut buffer = EditBuffer::from(vec!["1", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(5).expect("current line set");
        let res = eval_address(&mut input, &mut buffer).expect("evaluated address");
        assert_eq!(2, buffer.current_line());
        assert_eq!(Some(Address::Span(2, 5)), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn semicolon_left_addr_chain() {
        let mut input = "3;n".chars().peekable();
        let mut buffer = EditBuffer::from(vec!["1", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(2).expect("current line set");
        let res = eval_address(&mut input, &mut buffer).expect("evaluated address");
        assert_eq!(3, buffer.current_line());
        assert_eq!(Some(Address::Span(3, 3)), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn semicolon_right_addr_chain() {
        let mut input = ";5n".chars().peekable();
        let mut buffer = EditBuffer::from(vec!["1", "2", "3", "4", "5", "6"]);
        buffer.set_current_line(3).expect("current line set");
        let res = eval_address(&mut input, &mut buffer).expect("evaluated address");
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
        let res = eval_address(&mut input, &mut buffer).expect("successful address eval");
        assert_eq!(2usize, buffer.current_line());
        assert_eq!(Some(Address::Span(2usize, 10usize)), res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn offset_too_large() {
        let mut input = "999999999999999999999999999".chars().peekable();
        let _res = eval_addr_offsets(&mut input)
            .err()
            .expect("should be an error");
    }

    #[test]
    fn offset_too_small() {
        let mut input = "-999999999999999999999999999".chars().peekable();
        let _res = eval_addr_offsets(&mut input).err().expect("an error");
        assert!(matches!(Error::OffsetTooSmall, _res));
    }

    #[test]
    fn eval_line_addr_propegates_errors() {
        let mut input = ".+9999999999999999999999999999n".chars().peekable();
        let mut buffer = EditBuffer::from(vec!["one", "two", "three"]);
        buffer.set_current_line(1).expect("valid line number");
        let res = eval_line_addr(&mut input, &buffer).expect_err("OffsetTooLarge");
        assert_eq!(Error::OffsetTooLarge, res);
    }

    #[test]
    fn eval_addr_chain_propegates_errors() {
        let mut buffer = EditBuffer::from(vec!["one", "two", "three"]);
        buffer.set_current_line(1usize).expect("no error");
        let mut input = ".+9999999999999999999999999999n".chars().peekable();
        let res = eval_addr_chain(&mut input, &mut buffer, None, Separator::Comma)
            .expect_err("OffsetTooLarge");
        assert_eq!(Error::OffsetTooLarge, res);
    }
}
