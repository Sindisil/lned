use std::cmp::Ordering;
use std::fmt::{self, Display, Formatter};
use std::ops::{AddAssign, SubAssign};
use std::str::FromStr;

use crate::error::ParseEolError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Eol {
    Lf,
    Crlf,
}

impl Eol {
    #[must_use]
    pub fn native() -> Eol {
        if std::env::consts::FAMILY == "windows" { Eol::Crlf } else { Eol::Lf }
    }

    #[inline]
    #[must_use]
    pub fn str_value(self) -> &'static str {
        match self {
            Eol::Lf => "\n",
            Eol::Crlf => "\r\n",
        }
    }

    #[must_use]
    pub fn display_str(self) -> &'static str {
        match self {
            Eol::Lf => "LF",
            Eol::Crlf => "CRLF",
        }
    }

    #[must_use]
    pub fn from_line<T: AsRef<str>>(s: T) -> Option<Eol> {
        let s = s.as_ref();
        if s.ends_with(Eol::Crlf.str_value()) {
            return Some(Eol::Crlf);
        }
        if s.ends_with(Eol::Lf.str_value()) {
            return Some(Eol::Lf);
        }
        None
    }

    #[must_use]
    pub fn strip(s: &str) -> &str {
        let eol_str = Eol::from_line(s).map_or("", |eol| eol.str_value());
        s.trim_end_matches(eol_str)
    }
}

impl From<Eol> for &str {
    fn from(value: Eol) -> Self {
        value.str_value()
    }
}

impl From<&Eol> for &str {
    fn from(value: &Eol) -> Self {
        value.str_value()
    }
}

impl Display for Eol {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.display_str())
    }
}

impl FromStr for Eol {
    type Err = ParseEolError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.to_lowercase();
        if s == "crlf" {
            Ok(Eol::Crlf)
        } else if s == "lf" {
            Ok(Eol::Lf)
        } else {
            Err(ParseEolError)
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Eols {
    pub default_eol: Eol,
    pub lfs: usize,
    pub crlfs: usize,
}

impl Eols {
    pub fn new(default_eol: Eol) -> Eols {
        Eols { default_eol, ..Default::default() }
    }

    pub fn prevailing(&self) -> Eol {
        match self.lfs.cmp(&self.crlfs) {
            Ordering::Greater => Eol::Lf,
            Ordering::Equal => self.default_eol,
            Ordering::Less => Eol::Crlf,
        }
    }

    pub fn is_mixed(&self) -> bool {
        self.lfs & self.crlfs != 0
    }

    pub fn is_empty(&self) -> bool {
        self.crlfs + self.lfs == 0
    }

    pub fn eol_count(&self) -> usize {
        self.crlfs + self.lfs
    }

    /// Create an Eols object from text lines.
    ///
    /// [`Eols`].default will be the first EOL found, or `Eol::Lf` if
    /// none of the lines were terminated.
    ///
    /// Unterminated lines are skipped.
    pub fn from_lines(lines: impl IntoIterator<Item: AsRef<str>>) -> Self {
        let mut lines =
            lines.into_iter().skip_while(|l| Eol::from_line(l).is_none());
        let Some(first_eol) = lines.next().and_then(Eol::from_line) else {
            // No lines had EOLs, return default
            return Eols::default();
        };

        let mut eols = Eols {
            default_eol: first_eol,
            crlfs: usize::from(first_eol == Eol::Crlf),
            lfs: usize::from(first_eol == Eol::Lf),
        };

        for line in lines {
            if let Some(line_eol) = Eol::from_line(line) {
                eols.crlfs += usize::from(line_eol == Eol::Crlf);
                eols.lfs += usize::from(line_eol == Eol::Lf);
            }
        }

        eols
    }
}

impl Default for Eols {
    fn default() -> Eols {
        Eols { default_eol: Eol::Lf, lfs: 0, crlfs: 0 }
    }
}

impl Display for Eols {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let s = match self.prevailing() {
            Eol::Lf => {
                if self.crlfs == 0 {
                    "LF"
                } else {
                    "mostly LF"
                }
            }
            Eol::Crlf => {
                if self.lfs == 0 {
                    "CRLF"
                } else {
                    "mostly CRLF"
                }
            }
        };
        write!(f, "{s}")
    }
}

impl AddAssign<Eol> for Eols {
    fn add_assign(&mut self, rhs: Eol) {
        match rhs {
            Eol::Lf => self.lfs += 1,
            Eol::Crlf => self.crlfs += 1,
        }
    }
}

impl AddAssign<Eols> for Eols {
    fn add_assign(&mut self, rhs: Eols) {
        if self.is_empty() {
            self.default_eol = rhs.default_eol;
        }
        self.crlfs += rhs.crlfs;
        self.lfs += rhs.lfs;
    }
}

impl SubAssign<Eol> for Eols {
    fn sub_assign(&mut self, rhs: Eol) {
        match rhs {
            Eol::Lf => self.lfs -= 1,
            Eol::Crlf => self.crlfs -= 1,
        }
    }
}
impl SubAssign<Eols> for Eols {
    fn sub_assign(&mut self, rhs: Eols) {
        self.crlfs -= rhs.crlfs;
        self.lfs -= rhs.lfs;
    }
}

#[cfg(test)]
pub trait IsEol {
    fn is_eol(&self) -> bool;
}

impl IsEol for &str {
    fn is_eol(&self) -> bool {
        Eol::from_line(self).is_some()
    }
}

mod tests {
    use super::*;

    #[test]
    fn eols_when_all_crlf() {
        let lines =
            vec!["L1\r\n".to_owned(), "L2\r\n".to_owned(), "L3\r\n".to_owned()];
        let expected = Eols { default_eol: Eol::Crlf, crlfs: 3, lfs: 0 };
        assert_eq!(Eols::from_lines(&lines), expected);
    }

    #[test]
    fn eols_when_all_lf() {
        let lines =
            vec!["L1\n".to_owned(), "L2\n".to_owned(), "L3\n".to_owned()];
        let expected = Eols { default_eol: Eol::Lf, lfs: 3, crlfs: 0 };
        assert_eq!(Eols::from_lines(&lines), expected);
    }

    #[test]
    fn eols_when_most_crlf() {
        let lines =
            vec!["L1\r\n".to_owned(), "L2\n".to_owned(), "L3\r\n".to_owned()];
        let expected = Eols { default_eol: Eol::Crlf, crlfs: 2, lfs: 1 };
        let actual = Eols::from_lines(&lines);
        assert_eq!(actual, expected);
    }

    #[test]
    fn eols_when_most_lf() {
        let lines =
            vec!["L1\n".to_owned(), "L2\n".to_owned(), "L3\r\n".to_owned()];
        let expected = Eols { default_eol: Eol::Lf, lfs: 2, crlfs: 1 };
        let actual = Eols::from_lines(&lines);
        assert_eq!(actual, expected);
    }

    #[test]
    fn eols_when_equal_lf_crlf() {
        let lines = vec![
            "L1\n".to_owned(),
            "L2\r\n".to_owned(),
            "L3\r\n".to_owned(),
            "L4\n".to_owned(),
        ];
        let expected = Eols { default_eol: Eol::Lf, crlfs: 2, lfs: 2 };
        let actual = Eols::from_lines(&lines);
        assert!(actual.is_mixed());
        assert_eq!(actual, expected);
    }

    // Eol tests

    #[test]
    fn eol_from_str() {
        assert_eq!("CRLF".parse::<Eol>().unwrap(), Eol::Crlf);
        assert_eq!("LF".parse::<Eol>().unwrap(), Eol::Lf);
        assert_eq!("not an eol".parse::<Eol>(), Err(ParseEolError),);
    }

    #[test]
    fn eol_display_str() {
        assert_eq!(&Eol::Lf.to_string(), "LF");
        assert_eq!(&Eol::Crlf.to_string(), "CRLF");
    }
}
