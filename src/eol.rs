use std::fmt::{self, Display, Formatter};

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
    pub fn as_str(self) -> &'static str {
        match self {
            Eol::Lf => "\n",
            Eol::Crlf => "\r\n",
        }
    }

    #[must_use]
    pub fn get_eol<T: AsRef<str>>(s: T) -> Option<Eol> {
        let s = s.as_ref();
        if s.ends_with(Eol::Crlf.as_str()) {
            return Some(Eol::Crlf);
        }
        if s.ends_with(Eol::Lf.as_str()) {
            return Some(Eol::Lf);
        }
        None
    }
}

impl Display for Eol {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[cfg(test)]
mod tests {
    // Eol tests
}
