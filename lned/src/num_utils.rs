pub trait NumUtils {
    fn decimal_digits(&self) -> usize;
}

impl NumUtils for usize {
    fn decimal_digits(&self) -> usize {
        let mut d = 1;
        let mut n = *self / 10;
        while n > 0 {
            n /= 10;
            d += 1;
        }
        d
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use similar_asserts::assert_eq;

    #[test]
    fn decimal_digits() {
        assert_eq!(1usize, 0usize.decimal_digits());
        assert_eq!(1usize, 5usize.decimal_digits());
        assert_eq!(3usize, 999usize.decimal_digits());
        assert_eq!(2usize, 13usize.decimal_digits());
        assert_eq!(5usize, 12345usize.decimal_digits());
    }
}
