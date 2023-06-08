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
pub enum Cmd {
    Quit,
}

#[derive(Debug)]
pub enum AddrChain {
    Address(LineAddr),
    Chain(Option<LineAddr>, AddrSeparator, Option<Box<AddrChain>>),
}

#[derive(Debug, PartialEq)]
pub enum LineAddr {
    Dot(Vec<isize>),
    Dollar(Vec<isize>),
    Num(usize, Vec<isize>),
    Regex(String, Vec<isize>),
    RevRegex(String, Vec<isize>),
}

#[derive(Debug)]
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
    BadLineNumber(String),
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
            ParseError::BadLineNumber(s) => write!(f, "Bad line number: '{s}'"),
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
    let mut addr_chain: Option<AddrChain> = None;
    parse_addr_chain(cmd_chars, &mut addr_chain)?;
    match cmd_chars.peek() {
        Some('q') => addr_chain.map_or(Ok(Cmd::Quit), |_| {
            Err(ParseError::Unknown(cmd_chars.collect()))
        }),
        _ => Err(ParseError::Unknown(cmd_chars.collect())),
    }
}

fn parse_addr_chain(
    cmd_chars: &mut iter::Peekable<str::Chars>,
    chain: &mut Option<AddrChain>,
) -> Result<Option<AddrChain>, ParseError> {
    // Try to parse first address
    let left_addr = parse_line_addr(cmd_chars)?;
    // Try to parse separator.
    // If no separator, add parsed left addr (if any), and return chain
    // Recursively call parse_addr_chain()
    Ok(None)
}

fn parse_line_addr(
    cmd_chars: &mut iter::Peekable<str::Chars>,
) -> Result<Option<LineAddr>, ParseError> {
    let _ = cmd_chars.by_ref().skip_while(|c| c.is_whitespace());
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
                let num: String = cmd_chars
                    .by_ref()
                    .take_while(|c| matches!('0'..='9', c))
                    .collect();
                let num = num.parse().map_err(|_| ParseError::BadLineNumber(num))?;
                let offsets = parse_addr_offsets(cmd_chars)?;
                Ok(Some(LineAddr::Num(num, offsets)))
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
    let mut offset_chars = String::new();
    while let Some(c) = cmd_chars
        .by_ref()
        .next_if(|c| c.is_blank() || c.is_ascii_digit() || *c == '+' || *c == '-')
    {
        offset_chars.push(c);
    }
    let mut offset_chars = offset_chars.chars().peekable();
    let mut offsets = Vec::new();
    while let Some(c) = offset_chars.peek() {
        match c {
            '0'..='9' => {
                let offset = offset_chars
                    .by_ref()
                    .take_while(|c| c.is_ascii_digit())
                    .try_fold(0isize, |acc, c| {
                        c.to_digit(10).and_then(|d| {
                            acc.checked_mul(10).and_then(|n| n.checked_add(d as isize))
                        })
                    })
                    .ok_or(ParseError::OffsetTooLarge)?;
                offsets.push(offset);
            }
            '+' => {
                let offset = offset_chars
                    .by_ref()
                    .skip(1)
                    .take_while(|c| c.is_ascii_digit())
                    .try_fold(0isize, |acc, c| {
                        c.to_digit(10).and_then(|d| {
                            acc.checked_mul(10).and_then(|n| n.checked_add(d as isize))
                        })
                    })
                    .ok_or(ParseError::OffsetTooLarge)?;
                offsets.push(cmp::max(1, offset));
            }
            '-' => {
                let offset = offset_chars
                    .by_ref()
                    .skip(1)
                    .take_while(|c| c.is_ascii_digit())
                    .try_fold(0isize, |acc, c| {
                        c.to_digit(10).and_then(|d| {
                            acc.checked_mul(10).and_then(|n| n.checked_sub(d as isize))
                        })
                    })
                    .ok_or(ParseError::OffsetTooSmall)?;
                offsets.push(cmp::min(-1, offset));
            }
            _ => {
                offset_chars.next();
            }
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
            let mut input = " +4++ 5-6   -7 +8---n".chars().peekable();
            let _res = parse_addr_offsets(&mut input).unwrap();
            assert_eq!(vec![4, 1, 1, 5, -6, -7, 8, -1, -1, -1,], _res);
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
    }
}
