use std::cmp;
use std::fmt;
use std::iter;
use std::str;

trait CharUtils {
    fn is_blank(&self) -> bool;
}

impl CharUtils for char {
    fn is_blank(&self) -> bool {
        *self == ' ' || *self == '\t'
    }
}

#[derive(Debug)]
struct PeekingTakeWhile<I, P> {
    iter: I,
    pred: P,
}

impl<I, P> Iterator for PeekingTakeWhile<&mut iter::Peekable<I>, P>
where
    I: Iterator,
    P: Fn(&I::Item) -> bool,
{
    type Item = I::Item;

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next_if(&self.pred)
    }
}

trait Peeking: Iterator + Sized {
    fn peeking_take_while<P>(self, pred: P) -> PeekingTakeWhile<Self, P>
    where
        P: Fn(&Self::Item) -> bool;
}

impl<I> Peeking for &mut iter::Peekable<I>
where
    I: Iterator,
{
    fn peeking_take_while<P>(self, pred: P) -> PeekingTakeWhile<Self, P>
    where
        P: Fn(&Self::Item) -> bool,
    {
        PeekingTakeWhile { iter: self, pred }
    }
}

#[derive(Debug)]
pub enum Cmd {
    Quit,
}

//#[derive(Debug, PartialEq, Clone)]
//pub enum AddrChain {
//    Address(LineAddr),
//    Chain(Option<LineAddr>, AddrSeparator, Option<Box<AddrChain>>),
//}

#[derive(Debug, PartialEq, Clone)]
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

#[derive(Debug, PartialEq)]
pub enum ParseError {
    Unknown(String),
    UnexpectedAddress,
    EarlyEnd,
    OffsetTooLarge,
    OffsetTooSmall,
    LineNumberTooLarge,
}

impl std::error::Error for ParseError {}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ParseError::UnexpectedAddress => write!(f, "Command takes no line address."),
            ParseError::EarlyEnd => write!(f, "Unexpected early end of command line"),
            ParseError::Unknown(s) => write!(f, "Unknown command '{s}'"),
            ParseError::OffsetTooLarge => write!(f, "Offset too large"),
            ParseError::OffsetTooSmall => write!(f, "Offset too small"),
            ParseError::LineNumberTooLarge => write!(f, "Line number too large"),
        }
    }
}

impl str::FromStr for Cmd {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut cmd_chars = s.chars().peekable();
        parse_cmd(&mut cmd_chars)
    }
}

fn parse_cmd(cmd_chars: &mut iter::Peekable<str::Chars>) -> Result<Cmd, ParseError> {
    let addr_chain: Option<AddrChain> = parse_addr_chain(cmd_chars)?;
    match cmd_chars.peek() {
        Some('q') => addr_chain.map_or(Ok(Cmd::Quit), |_| Err(ParseError::UnexpectedAddress)),
        _ => Err(ParseError::Unknown(cmd_chars.collect())),
    }
}

fn parse_addr_chain(
    cmd_chars: &mut iter::Peekable<str::Chars>,
) -> Result<Option<AddrChain>, ParseError> {
    let left = parse_line_addr(cmd_chars)?;
    let separator = parse_addr_separator(cmd_chars)?;
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
        let right = right.map(|r| Box::new(r));
        Ok(Some(AddrChain {
            left,
            separator,
            right,
        }))
    }
}

fn parse_addr_separator(
    cmd_chars: &mut iter::Peekable<str::Chars>,
) -> Result<Option<AddrSeparator>, ParseError> {
    cmd_chars.peeking_take_while(|c| c.is_blank());
    if let Some(c) = cmd_chars.peek() {
        match c {
            ',' => {
                cmd_chars.next();
                Ok(Some(AddrSeparator::Comma))
            }
            ';' => {
                cmd_chars.next();
                Ok(Some(AddrSeparator::Semicolon))
            }
            _ => Ok(None),
        }
    } else {
        Err(ParseError::EarlyEnd)
    }
}

fn parse_line_addr(
    cmd_chars: &mut iter::Peekable<str::Chars>,
) -> Result<Option<LineAddr>, ParseError> {
    cmd_chars.peeking_take_while(|c| c.is_blank());
    if let Some(c) = cmd_chars.peek() {
        match c {
            '.' => {
                cmd_chars.next();
                let offsets = parse_addr_offsets(cmd_chars)?;
                Ok(Some(LineAddr::Dot(offsets)))
            }
            '$' => {
                cmd_chars.next();
                let offsets = parse_addr_offsets(cmd_chars)?;
                Ok(Some(LineAddr::Dollar(offsets)))
            }
            '/' => {
                cmd_chars.next();
                let re = cmd_chars.by_ref().take_while(|c| *c != '/').collect();
                let offsets = parse_addr_offsets(cmd_chars)?;
                Ok(Some(LineAddr::Regex(re, offsets)))
            }
            '?' => {
                cmd_chars.next();
                let re = cmd_chars.by_ref().take_while(|c| *c != '?').collect();
                let offsets = parse_addr_offsets(cmd_chars)?;
                Ok(Some(LineAddr::RevRegex(re, offsets)))
            }
            '0'..='9' => {
                let num = cmd_chars
                    .peeking_take_while(|c| c.is_ascii_digit())
                    .try_fold(0usize, |acc, c| {
                        c.to_digit(10).and_then(|d| {
                            acc.checked_mul(10).and_then(|n| n.checked_add(d as usize))
                        })
                    })
                    .ok_or(ParseError::LineNumberTooLarge)?;
                let offsets = parse_addr_offsets(cmd_chars)?;
                Ok(Some(LineAddr::Num(num, offsets)))
            }
            '+' | '-' => {
                let offsets = parse_addr_offsets(cmd_chars)?;
                Ok(Some(LineAddr::Dot(offsets)))
            }
            _ => Ok(None),
        }
    } else {
        Err(ParseError::EarlyEnd)
    }
}

fn parse_addr_offsets(
    cmd_chars: &mut iter::Peekable<str::Chars>,
) -> Result<Vec<isize>, ParseError> {
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
                    .ok_or(ParseError::OffsetTooLarge)?;
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
                    .ok_or(ParseError::OffsetTooLarge)?;
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
                    .ok_or(ParseError::OffsetTooSmall)?;
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

    mod parse_cmd {
        use super::*;

        #[test]
        fn unknown_command_gives_error() {
            let input = "o";
            let res = input
                .parse::<Cmd>()
                .err()
                .expect("should always be an error");
            assert!(matches!(res, ParseError::Unknown(_)));
        }

        #[test]
        fn quit() {
            let input = "q";
            let res = input.parse::<Cmd>().expect("should always parse ok");
            assert!(matches!(res, Cmd::Quit));
        }

        #[test]
        fn quit_with_illegal_addr() {
            let input = ".q";
            let res = input.parse::<Cmd>().err().expect("should be an error");
            assert!(matches!(res, ParseError::UnexpectedAddress));
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
            assert_eq!(Ok(Some(AddrSeparator::Comma)), _res);
            assert_eq!("n", input.collect::<String>());
        }

        #[test]
        fn semicolon_addr_separator() {
            let mut input = ";n".chars().peekable();
            let _res = parse_addr_separator(&mut input);
            assert_eq!(Ok(Some(AddrSeparator::Semicolon)), _res);
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
            assert_eq!(ParseError::OffsetTooSmall, _res);
        }
    }
}
