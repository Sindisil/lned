pub trait CharUtils {
    fn is_blank(&self) -> bool;
}

impl CharUtils for char {
    fn is_blank(&self) -> bool {
        *self == ' ' || *self == '\t'
    }
}

mod tests {}
