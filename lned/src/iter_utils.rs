use core::iter::Peekable;

pub trait Peeking: Iterator + Sized {
    fn peeking_take_while<P>(self, pred: P) -> PeekingTakeWhile<Self, P>
    where
        P: Fn(&Self::Item) -> bool;
}

#[derive(Debug, Clone)]
pub struct PeekingTakeWhile<I, P> {
    iter: I,
    pred: P,
}

impl<I, P> PeekingTakeWhile<&mut Peekable<I>, P>
where
    I: Iterator,
{
    pub fn new(iter: I, pred: P) -> PeekingTakeWhile<I, P> {
        PeekingTakeWhile { iter, pred }
    }
}

impl<I, P> Iterator for PeekingTakeWhile<&mut Peekable<I>, P>
where
    I: Iterator,
    P: Fn(&I::Item) -> bool,
{
    type Item = I::Item;

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next_if(&self.pred)
    }
}

impl<I> Peeking for &mut Peekable<I>
where
    I: Iterator,
{
    fn peeking_take_while<P>(self, pred: P) -> PeekingTakeWhile<Self, P>
    where
        P: Fn(&Self::Item) -> bool,
    {
        PeekingTakeWhile::new(self, pred)
    }
}

#[cfg(test)]

mod tests {
    use super::*;

    #[test]
    fn peeking_take_while() {
        let s = "12345abcde";
        let mut piter = s.chars().peekable();
        let numbers = piter
            .peeking_take_while(char::is_ascii_digit)
            .collect::<String>();
        assert_eq!("12345", numbers);
        assert_eq!('a', *piter.peek().unwrap());
        let letters = piter.collect::<String>();
        assert_eq!("abcde", letters);
    }
}
