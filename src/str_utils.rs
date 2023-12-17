pub trait StrUtils {
    fn is_blank(&self) -> bool;
}

impl StrUtils for str {
    fn is_blank(&self) -> bool {
        self == " " || self == "\t"
    }
}

mod tests {}
