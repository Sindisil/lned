pub trait StrUtils {
    fn is_blank(&self) -> bool;
    fn is_ascii_digit(&self) -> bool;
}

impl StrUtils for str {
    fn is_blank(&self) -> bool {
        self == " " || self == "\t"
    }

    fn is_ascii_digit(&self) -> bool {
        !self.is_empty()
            && self.is_ascii()
            && self.chars().next().expect("shouldn't be empty").is_ascii_digit()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tab_is_blank() {
        assert!("\t".is_blank());
    }

    #[test]
    fn space_is_blank() {
        assert!(" ".is_blank());
    }

    #[test]
    fn line_terminators_are_not_blank() {
        assert!(!"\n".is_blank());
        assert!(!"\r\n".is_blank());
    }

    #[test]
    fn is_ascii_digit() {
        let digits = vec!["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];
        for s in digits {
            assert!(s.is_ascii_digit());
        }
        assert!(!" ".is_ascii_digit());
    }
}
