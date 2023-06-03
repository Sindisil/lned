use std::fmt;
use std::iter;
use std::str;

#[derive(Debug)]
pub enum Cmd {
    Quit,
}

#[derive(Debug)]
pub enum AddrChain {
    Address(LineAddr),
    Chain(Option<LineAddr>, AddrSeparator, Option<Box<AddrChain>>),
}

#[derive(Debug)]
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
}

impl std::error::Error for ParseError {}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ParseError::UnexpectedAddress => write!(f, "Command takes no line address."),
            ParseError::Unknown(s) => write!(f, "Unknown command '{s}'"),
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
    let address = parse_addr_chain(cmd_chars)?;
    match cmd_chars.peek() {
        Some('q') => address.map_or(Ok(Cmd::Quit), |_| {
            Err(ParseError::Unknown(cmd_chars.collect()))
        }),
        _ => Err(ParseError::Unknown(cmd_chars.collect())),
    }
}

fn parse_addr_chain(
    cmd_chars: &mut iter::Peekable<str::Chars>,
) -> Result<Option<AddrChain>, ParseError> {
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_unknown_command_gives_error() {
        let input = "o";
        let res = input
            .parse::<Cmd>()
            .err()
            .expect("should always be an error");
        assert!(matches!(res, ParseError::Unknown(_)));
    }

    #[test]
    fn parse_quit() {
        let input = "q";
        let res = input.parse::<Cmd>().expect("should always parse ok");
        assert!(matches!(res, Cmd::Quit));
    }
}
