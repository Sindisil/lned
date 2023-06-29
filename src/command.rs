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
    LineNumberTooLarge,
}

impl std::error::Error for Error {}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Error::UnexpectedAddress => write!(f, "Command takes no line address."),
            Error::Unknown(s) => write!(f, "Unknown command '{s}'"),
            Error::OffsetTooLarge => write!(f, "Offset too large"),
            Error::OffsetTooSmall => write!(f, "Offset too small"),
            Error::LineNumberTooLarge => write!(f, "Line number too large"),
        }
    }
}

impl Cmd {
    pub fn parse(
        cmd_chars: &mut iter::Peekable<str::Chars>,
        _buffer: &EditBuffer,
        address: Option<Address>,
    ) -> Result<Cmd, Error> {
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

#[derive(Debug, PartialEq, Clone, Default)]
pub struct AddrChain {
    left: Option<LineAddr>,
    separator: Option<AddrSeparator>,
    right: Option<Box<AddrChain>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LineAddr {
    Dot(Vec<isize>),
    Dollar(Vec<isize>),
    Num(usize, Vec<isize>),
    Regex(String, Vec<isize>),
    RevRegex(String, Vec<isize>),
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum AddrSeparator {
    Comma,
    Semicolon,
}

pub fn eval_address(
    _cmd_chars: &mut iter::Peekable<str::Chars>,
    _buffer: &mut EditBuffer,
) -> Result<Option<Address>, Error> {
    todo!();
}

fn parse_addr_chain(
    cmd_chars: &mut iter::Peekable<str::Chars>,
) -> Result<Option<AddrChain>, Error> {
    let left = parse_line_addr(cmd_chars)?;
    let separator = parse_addr_separator(cmd_chars);
    if separator.is_none() {
        if left.is_none() {
            Ok(None)
        } else {
            Ok(Some(AddrChain {
                left,
                separator,
                right: None,
            }))
        }
    } else {
        let right = parse_addr_chain(cmd_chars)?;
        let right = right.map(Box::new);
        Ok(Some(AddrChain {
            left,
            separator,
            right,
        }))
    }
}

fn parse_addr_separator(cmd_chars: &mut iter::Peekable<str::Chars>) -> Option<AddrSeparator> {
    match cmd_chars.peeking_skip_while(|c| c.is_blank()).peek() {
        Some(',') => {
            cmd_chars.next();
            Some(AddrSeparator::Comma)
        }
        Some(';') => {
            cmd_chars.next();
            Some(AddrSeparator::Semicolon)
        }
        _ => None,
    }
}

fn parse_line_addr(cmd_chars: &mut iter::Peekable<str::Chars>) -> Result<Option<LineAddr>, Error> {
    match cmd_chars.peeking_skip_while(|c| c.is_blank()).peek() {
        Some('.') => {
            cmd_chars.next();
            let offsets = parse_addr_offsets(cmd_chars)?;
            Ok(Some(LineAddr::Dot(offsets)))
        }
        Some('$') => {
            cmd_chars.next();
            let offsets = parse_addr_offsets(cmd_chars)?;
            Ok(Some(LineAddr::Dollar(offsets)))
        }
        Some('/') => {
            cmd_chars.next();
            let re = cmd_chars.by_ref().take_while(|c| *c != '/').collect();
            let offsets = parse_addr_offsets(cmd_chars)?;
            Ok(Some(LineAddr::Regex(re, offsets)))
        }
        Some('?') => {
            cmd_chars.next();
            let re = cmd_chars.by_ref().take_while(|c| *c != '?').collect();
            let offsets = parse_addr_offsets(cmd_chars)?;
            Ok(Some(LineAddr::RevRegex(re, offsets)))
        }
        Some('0'..='9') => {
            let num = cmd_chars
                .peeking_take_while(|c| c.is_ascii_digit())
                .try_fold(0usize, |acc, c| {
                    c.to_digit(10)
                        .and_then(|d| acc.checked_mul(10).and_then(|n| n.checked_add(d as usize)))
                })
                .ok_or(Error::LineNumberTooLarge)?;
            let offsets = parse_addr_offsets(cmd_chars)?;
            Ok(Some(LineAddr::Num(num, offsets)))
        }
        Some('+' | '-') => {
            let offsets = parse_addr_offsets(cmd_chars)?;
            Ok(Some(LineAddr::Dot(offsets)))
        }
        _ => Ok(None),
    }
}

fn parse_addr_offsets(cmd_chars: &mut iter::Peekable<str::Chars>) -> Result<Vec<isize>, Error> {
    let mut offsets = Vec::new();
    while let Some(c) = cmd_chars.peek() {
        match c {
            '0'..='9' => {
                let offset = cmd_chars
                    .peeking_take_while(|c| c.is_ascii_digit())
                    .try_fold(0isize, |acc, c| {
                        c.to_digit(10).and_then(|d| {
                            acc.checked_mul(10).and_then(|n| n.checked_add(d as isize))
                        })
                    })
                    .ok_or(Error::OffsetTooLarge)?;
                offsets.push(offset);
            }
            '+' => {
                cmd_chars.next();
                let offset = cmd_chars
                    .peeking_take_while(|c| c.is_ascii_digit())
                    .try_fold(0isize, |acc, c| {
                        c.to_digit(10).and_then(|d| {
                            acc.checked_mul(10).and_then(|n| n.checked_add(d as isize))
                        })
                    })
                    .ok_or(Error::OffsetTooLarge)?;
                offsets.push(cmp::max(1, offset));
            }
            '-' => {
                cmd_chars.next();
                let offset = cmd_chars
                    .peeking_take_while(|c| c.is_ascii_digit())
                    .try_fold(0isize, |acc, c| {
                        c.to_digit(10).and_then(|d| {
                            acc.checked_mul(10).and_then(|n| n.checked_sub(d as isize))
                        })
                    })
                    .ok_or(Error::OffsetTooSmall)?;
                offsets.push(cmp::min(-1, offset));
            }
            ' ' | 't' => {
                cmd_chars.next();
            }
            _ => break,
        }
    }
    Ok(offsets)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_command_gives_error() {
        let mut input = "o\n".chars().peekable();
        let mut buffer = EditBuffer::new();
        let res = Cmd::parse(&mut input, &mut buffer, None)
            .err()
            .expect("an error indicating an unknown command");
        assert!(matches!(res, Error::Unknown(_)));
    }

    #[test]
    fn blank_cmd_line() {
    let mut buffer = EditBuffer::new();
  let mut input = "\n".chars().peekable();
  let res = Cmd::parse(&mut input, &mut buffer, None).expect("a successful parse");
  assert_eq!(Cmd::Null(None), res);
    }

    #[test]
    fn blank_cmd_line_crlf() {
        let input = "\r\n";
        todo!();
    }

    #[test]
    fn offset_only_cmd() {
        let input = "-\n";
        todo!();
    }

    #[test]
    fn quit() {
        let mut buffer = EditBuffer::new();
        let mut input = "q\n".chars().peekable();
        let res = Cmd::parse(&mut input, &mut buffer, None).expect("a successful parse");
        assert_eq!(Cmd::Quit, res);
    }

    #[test]
    fn quit_with_illegal_addr() {
        todo!();
    }

    #[test]
    fn single_addr_offset() {
        let mut input = "2n".chars().peekable();
        let _res = parse_addr_offsets(&mut input).unwrap();
        assert_eq!(vec![2,], _res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn single_plus_addr_offset() {
        let mut input = "+3n".chars().peekable();
        let _res = parse_addr_offsets(&mut input).unwrap();
        assert_eq!(vec![3,], _res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn single_negative_addr_offset() {
        let mut input = "-4n".chars().peekable();
        let _res = parse_addr_offsets(&mut input).unwrap();
        assert_eq!(vec![-4,], _res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn combined_addr_offsets() {
        let mut input = " +4++ 5 6-6   -7 +8---n".chars().peekable();
        let _res = parse_addr_offsets(&mut input).unwrap();
        assert_eq!(vec![4, 1, 1, 5, 6, -6, -7, 8, -1, -1, -1,], _res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn addr_offsets_trailing_minus() {
        let mut input = "-4-n".chars().peekable();
        let _res = parse_addr_offsets(&mut input).unwrap();
        assert_eq!(vec![-4, -1,], _res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn dot_line_addr() {
        let mut input = ".n".chars().peekable();
        let _res = parse_line_addr(&mut input).unwrap().unwrap();
        assert_eq!(LineAddr::Dot(Vec::new()), _res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn dot_line_addr_with_offset() {
        let mut input = ".+2n".chars().peekable();
        let _res = parse_line_addr(&mut input);
        assert_eq!(Ok(Some(LineAddr::Dot(vec![2,]))), _res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn dollar_line_addr() {
        let mut input = "$n".chars().peekable();
        let _res = parse_line_addr(&mut input);
        assert_eq!(Ok(Some(LineAddr::Dollar(Vec::new()))), _res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn dollar_line_addr_with_offset() {
        let mut input = "$-27n".chars().peekable();
        let _res = parse_line_addr(&mut input);
        assert_eq!(Ok(Some(LineAddr::Dollar(vec![-27,]))), _res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn regex_line_addr() {
        let mut input = "/fn name/n".chars().peekable();
        let _res = parse_line_addr(&mut input);
        assert_eq!(
            Ok(Some(LineAddr::Regex("fn name".to_string(), Vec::new()))),
            _res
        );
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn rev_regex_line_addr() {
        let mut input = "?fn name?n".chars().peekable();
        let _res = parse_line_addr(&mut input);
        assert_eq!(
            Ok(Some(LineAddr::RevRegex("fn name".to_string(), Vec::new()))),
            _res
        );
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn regex_line_addr_with_offset() {
        let mut input = "/fn name/+12n".chars().peekable();
        let _res = parse_line_addr(&mut input);
        assert_eq!(
            Ok(Some(LineAddr::Regex("fn name".to_string(), vec![12,]))),
            _res
        );
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn rev_regex_line_addr_with_offset() {
        let mut input = "?fn name?+12n".chars().peekable();
        let _res = parse_line_addr(&mut input);
        assert_eq!(
            Ok(Some(LineAddr::RevRegex("fn name".to_string(), vec![12,]))),
            _res
        );
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn plus_line_addr() {
        let mut input = "+n".chars().peekable();
        let _res = parse_line_addr(&mut input);
        assert_eq!(Ok(Some(LineAddr::Dot(vec![1,]))), _res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn plus_num_line_addr() {
        let mut input = "+2n".chars().peekable();
        let _res = parse_line_addr(&mut input);
        assert_eq!(Ok(Some(LineAddr::Dot(vec![2,]))), _res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn plus_line_addr_with_offsets() {
        let mut input = "+++5n".chars().peekable();
        let _res = parse_line_addr(&mut input);
        assert_eq!(Ok(Some(LineAddr::Dot(vec![1, 1, 5,]))), _res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn plus_num_line_addr_with_offsets() {
        let mut input = "+2--1n".chars().peekable();
        let _res = parse_line_addr(&mut input);
        assert_eq!(Ok(Some(LineAddr::Dot(vec![2, -1, -1,]))), _res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn minus_line_addr_with_offsets() {
        let mut input = "---2n".chars().peekable();
        let _res = parse_line_addr(&mut input);
        assert_eq!(Ok(Some(LineAddr::Dot(vec![-1, -1, -2,]))), _res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn minus_num_line_addr_with_offsets() {
        let mut input = "-2-+1n".chars().peekable();
        let _res = parse_line_addr(&mut input);
        assert_eq!(Ok(Some(LineAddr::Dot(vec![-2, -1, 1,]))), _res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn num_line_addr() {
        let mut input = "2n".chars().peekable();
        let _res = parse_line_addr(&mut input);
        assert_eq!(Ok(Some(LineAddr::Num(2, Vec::new()))), _res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn num_line_addr_with_offsets() {
        let mut input = "2++5-n".chars().peekable();
        let _res = parse_line_addr(&mut input);
        assert_eq!(Ok(Some(LineAddr::Num(2, vec![1, 5, -1,]))), _res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn comma_addr_separator() {
        let mut input = ",n".chars().peekable();
        let _res = parse_addr_separator(&mut input);
        assert_eq!(Some(AddrSeparator::Comma), _res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn semicolon_addr_separator() {
        let mut input = ";n".chars().peekable();
        let _res = parse_addr_separator(&mut input);
        assert_eq!(Some(AddrSeparator::Semicolon), _res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn empty_addr_chain() {
        let mut input = "n".chars().peekable();
        let _res = parse_addr_chain(&mut input);
        assert_eq!(Ok(None), _res);
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn comma_addr_chain() {
        let mut input = ",n".chars().peekable();
        let _res = parse_addr_chain(&mut input);
        assert_eq!(
            Ok(Some(AddrChain {
                left: None,
                separator: Some(AddrSeparator::Comma),
                right: None
            })),
            _res
        );
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn comma_left_addr_chain() {
        let mut input = "10,n".chars().peekable();
        let _res = parse_addr_chain(&mut input);
        assert_eq!(
            Ok(Some(AddrChain {
                left: Some(LineAddr::Num(10, Vec::new())),
                separator: Some(AddrSeparator::Comma),
                right: None
            })),
            _res
        );
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn comma_right_addr_chain() {
        let mut input = ",10n".chars().peekable();
        let _res = parse_addr_chain(&mut input);
        assert_eq!(
            Ok(Some(AddrChain {
                left: None,
                separator: Some(AddrSeparator::Comma),
                right: Some(Box::new(AddrChain {
                    left: Some(LineAddr::Num(10, Vec::new())),
                    separator: None,
                    right: None,
                })),
            })),
            _res
        );
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn comma_full_addr_chain() {
        let mut input = "2,10n".chars().peekable();
        let _res = parse_addr_chain(&mut input);
        assert_eq!(
            Ok(Some(AddrChain {
                left: Some(LineAddr::Num(2, Vec::new())),
                separator: Some(AddrSeparator::Comma),
                right: Some(Box::new(AddrChain {
                    left: Some(LineAddr::Num(10, Vec::new())),
                    separator: None,
                    right: None,
                })),
            })),
            _res
        );
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn semicolon_addr_chain() {
        let mut input = ";n".chars().peekable();
        let _res = parse_addr_chain(&mut input);
        assert_eq!(
            Ok(Some(AddrChain {
                left: None,
                separator: Some(AddrSeparator::Semicolon),
                right: None,
            })),
            _res
        );
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn semicolon_addr_chain_with_offsets() {
        let mut input = "$-50;+32n".chars().peekable();
        let _res = parse_addr_chain(&mut input);
        assert_eq!(
            Ok(Some(AddrChain {
                left: Some(LineAddr::Dollar(vec![-50,])),
                separator: Some(AddrSeparator::Semicolon),
                right: Some(Box::new(AddrChain {
                    left: Some(LineAddr::Dot(vec![32,])),
                    separator: None,
                    right: None
                })),
            })),
            _res
        );
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn semicolon_left_addr_chain() {
        let mut input = "10;n".chars().peekable();
        let _res = parse_addr_chain(&mut input);
        assert_eq!(
            Ok(Some(AddrChain {
                left: Some(LineAddr::Num(10, Vec::new())),
                separator: Some(AddrSeparator::Semicolon),
                right: None
            })),
            _res
        );
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn semicolon_right_addr_chain() {
        let mut input = ";10n".chars().peekable();
        let _res = parse_addr_chain(&mut input);
        assert_eq!(
            Ok(Some(AddrChain {
                left: None,
                separator: Some(AddrSeparator::Semicolon),
                right: Some(Box::new(AddrChain {
                    left: Some(LineAddr::Num(10, Vec::new())),
                    separator: None,
                    right: None,
                })),
            })),
            _res
        );
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn semicolon_full_addr_chain() {
        let mut input = "2;10n".chars().peekable();
        let _res = parse_addr_chain(&mut input);
        assert_eq!(
            Ok(Some(AddrChain {
                left: Some(LineAddr::Num(2, Vec::new())),
                separator: Some(AddrSeparator::Semicolon),
                right: Some(Box::new(AddrChain {
                    left: Some(LineAddr::Num(10, Vec::new())),
                    separator: None,
                    right: None,
                })),
            })),
            _res
        );
        assert_eq!("n", input.collect::<String>());
    }

    #[test]
    fn offset_too_large() {
        let mut input = "999999999999999999999999999".chars().peekable();
        let _res = parse_addr_offsets(&mut input)
            .err()
            .expect("should be an error");
    }

    #[test]
    fn offset_too_small() {
        let mut input = "-999999999999999999999999999".chars().peekable();
        let _res = parse_addr_offsets(&mut input)
            .err()
            .expect("should be an error");
        assert_eq!(Error::OffsetTooSmall, _res);
    }

    #[test]
    fn parse_line_addr_propegates_errors() {
        let mut input = ".+9999999999999999999999999999n".chars().peekable();
        let _res = parse_line_addr(&mut input)
            .err()
            .expect("should be an error");
        assert_eq!(Error::OffsetTooLarge, _res);
    }

    #[test]
    fn parse_addr_chain_propegates_errors() {
        let mut input = ".+9999999999999999999999999999n".chars().peekable();
        let _res = parse_addr_chain(&mut input)
            .err()
            .expect("should be an error");
        assert_eq!(Error::OffsetTooLarge, _res);
    }
}
