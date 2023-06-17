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

#[derive(Clone)]
struct PeekingTakeWhile<I, P> {
    iter: I,
    pred: P,
}

impl<I, P> fmt::Debug for PeekingTakeWhile<I, P>
where
    I: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PeekingTakeWhile")
            .field("iter", &self.iter)
            .finish()
    }
}

impl<I, P> PeekingTakeWhile<&mut iter::Peekable<I>, P>
where
    I: Iterator,
{
    pub fn new(iter: I, pred: P) -> PeekingTakeWhile<I, P> {
        PeekingTakeWhile { iter, pred }
    }

    fn peek(&mut self) -> Option<&<I as Iterator>::Item> {
        self.iter.peek()
    }
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

use std::fmt::Debug;
impl<I, P> Iterator for PeekingSkipWhile<&mut iter::Peekable<I>, P>
where
    I: Iterator,
    I::Item: Debug,
    P: Fn(&I::Item) -> bool,
{
    type Item = I::Item;

    fn next(&mut self) -> Option<Self::Item> {
        while !self.done_skipping {
            self.done_skipping = self.iter.next_if(&self.pred).is_none();
        }
        self.iter.next()
    }
}

struct PeekingSkipWhile<I, P> {
    iter: I,
    pred: P,
    done_skipping: bool,
}

impl<I, P> PeekingSkipWhile<I, P> {
    pub fn new(iter: I, pred: P) -> PeekingSkipWhile<I, P> {
        PeekingSkipWhile {
            iter,
            pred,
            done_skipping: false,
        }
    }
}

impl<I, P> fmt::Debug for PeekingSkipWhile<I, P>
where
    I: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PeekingSkipWhile")
            .field("iter", &self.iter)
            .field("done_skipping", &self.done_skipping)
            .finish()
    }
}

trait Peeking: Iterator + Sized {
    fn peeking_take_while<P>(self, pred: P) -> PeekingTakeWhile<Self, P>
    where
        P: Fn(&Self::Item) -> bool;

    fn peeking_skip_while<P>(self, pred: P) -> PeekingSkipWhile<Self, P>
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
        PeekingTakeWhile::new(self, pred)
    }

    fn peeking_skip_while<P>(self, pred: P) -> PeekingSkipWhile<Self, P>
    where
        P: Fn(&Self::Item) -> bool,
    {
        PeekingSkipWhile::new(self, pred)
    }
}

#[derive(Debug, PartialEq)]
pub enum Cmd {
    Quit,
    Print(Option<AddrChain>),
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

#[derive(Debug, PartialEq)]
pub enum ParseError {
    Unknown(String),
    UnexpectedAddress,
    OffsetTooLarge,
    OffsetTooSmall,
    LineNumberTooLarge,
}

impl std::error::Error for ParseError {}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ParseError::UnexpectedAddress => write!(f, "Command takes no line address."),
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
        None => Ok(Cmd::Print(addr_chain.or(Some(AddrChain {
            left: Some(LineAddr::Dot(vec![1])),
            ..Default::default()
        })))),
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
        let right = right.map(Box::new);
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
    while let Some(c) = cmd_chars.peek() {
        if c.is_blank() {
            cmd_chars.next();
        } else {
            break;
        }
    }
    match cmd_chars.peek() {
        Some(',') => {
            cmd_chars.next();
            Ok(Some(AddrSeparator::Comma))
        }
        Some(';') => {
            cmd_chars.next();
            Ok(Some(AddrSeparator::Semicolon))
        }
        _ => Ok(None),
    }
}

fn parse_line_addr(
    cmd_chars: &mut iter::Peekable<str::Chars>,
) -> Result<Option<LineAddr>, ParseError> {
    while let Some(c) = cmd_chars.peek() {
        if c.is_blank() {
            cmd_chars.next();
        } else {
            break;
        }
    }
    match cmd_chars.peek() {
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
                .ok_or(ParseError::LineNumberTooLarge)?;
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
        fn blank_cmd_line() {
            let input = "";
            let res = input.parse::<Cmd>().expect("successful parse");
            let expected = Cmd::Print(Some(AddrChain {
                left: Some(LineAddr::Dot(vec![1])),
                ..Default::default()
            }));
            assert_eq!(expected, res);
        }

        #[test]
        fn offset_only_cmd() {
            let input = "-";
            let res = input.parse::<Cmd>().expect("successful parse");
            let expected = Cmd::Print(Some(AddrChain {
                left: Some(LineAddr::Dot(vec![-1])),
                ..Default::default()
            }));
            assert_eq!(expected, res);
        }

        #[test]
        fn quit() {
            let input = "q";
            let _res = input.parse::<Cmd>().expect("should always parse ok");
            assert!(matches!(Cmd::Quit, _res));
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

        #[test]
        fn parse_line_error_propegates_errors() {
            let mut input = ".+9999999999999999999999999999n".chars().peekable();
            let _res = parse_line_addr(&mut input)
                .err()
                .expect("should be an error");
            assert_eq!(ParseError::OffsetTooLarge, _res);
        }

        #[test]
        fn peeking_skip_shile_skips() {
            let mut input = "     some text".chars().peekable();
            let res = input
                .peeking_skip_while(|c| c.is_blank())
                .collect::<String>();
            assert_eq!("some text", res);
        }
    }
}
