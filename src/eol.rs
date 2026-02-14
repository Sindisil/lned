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
}

impl Display for Eol {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

pub trait EolTerminated {
    #[must_use]
    fn is_eol_terminated(&self) -> bool;

    #[must_use]
    fn get_eol(&self) -> Option<Eol>;
}

impl<T: AsRef<str>> EolTerminated for T {
    fn is_eol_terminated(&self) -> bool {
        let s = self.as_ref();
        s.ends_with("\r\n") || s.ends_with('\n')
    }

    fn get_eol(&self) -> Option<Eol> {
        let s = self.as_ref();
        if s.ends_with(Eol::Crlf.as_str()) {
            return Some(Eol::Crlf);
        }
        if s.ends_with(Eol::Lf.as_str()) {
            return Some(Eol::Lf);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    // Eol and associated trait impl tests
}
