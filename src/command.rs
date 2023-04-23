use std::fmt;
use std::str;

#[derive(Debug, PartialEq)]
pub enum Cmd {
    //Quit,
}

#[derive(Debug, PartialEq)]
pub enum ParseError {
    Unknown(String),
}

impl std::error::Error for ParseError {}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            ParseError::Unknown(s) => write!(f, "Unknown command '{s}'"),
        }
    }
}

impl str::FromStr for Cmd {
    type Err = ParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Err(ParseError::Unknown(s.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_unknown_command_gives_error() {
        let input = "o";
        let res = input.parse::<Cmd>();
        assert_eq!(res, Err(ParseError::Unknown(input.to_string())));
    }
}
