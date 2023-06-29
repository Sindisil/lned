use std::fmt;
use std::iter;

pub trait Peeking: Iterator + Sized {
    fn peeking_take_while<P>(self, pred: P) -> PeekingTakeWhile<Self, P>
    where
        P: Fn(&Self::Item) -> bool;

    fn peeking_skip_while<P>(self, pred: P) -> PeekingSkipWhile<Self, P>
    where
        P: Fn(&Self::Item) -> bool;
}

#[derive(Clone)]
pub struct PeekingTakeWhile<I, P> {
    iter: I,
    pred: P,
}

impl<I, P> fmt::Debug for PeekingTakeWhile<I, P>
where
    I: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PeekingTakeWhile")
            .field("iter", &self.iter)
            .finish()
    }
}

impl<I, P> PeekingTakeWhile<&mut iter::Peekable<I>, P>
where
    I: Iterator,
{
    pub fn new(iter: I, pred: P) -> PeekingTakeWhile<I, P> {
        PeekingTakeWhile { iter, pred }
    }

    fn peek(&mut self) -> Option<&I::Item> {
        self.iter.peek()
    }
}

impl<I, P> Iterator for PeekingTakeWhile<&mut iter::Peekable<I>, P>
where
    I: Iterator,
    P: Fn(&I::Item) -> bool,
{
    type Item = I::Item;

    fn next(&mut self) -> Option<Self::Item> {
        self.iter.next_if(&self.pred)
    }
}

impl<I, P> Iterator for PeekingSkipWhile<&mut iter::Peekable<I>, P>
where
    I: Iterator,
    P: Fn(&I::Item) -> bool,
{
    type Item = I::Item;

    fn next(&mut self) -> Option<Self::Item> {
        while !self.done_skipping {
            self.done_skipping = self.iter.next_if(&self.pred).is_none();
        }
        self.iter.next()
    }
}

pub struct PeekingSkipWhile<I, P> {
    iter: I,
    pred: P,
    done_skipping: bool,
}

impl<I, P> PeekingSkipWhile<&mut iter::Peekable<I>, P>
where
    I: Iterator,
{
    pub fn new(iter: I, pred: P) -> PeekingSkipWhile<I, P> {
        PeekingSkipWhile {
            iter,
            pred,
            done_skipping: false,
        }
    }

    pub fn peek(&mut self) -> Option<&I::Item> {
        self.iter.peek()
    }
}

impl<I, P> fmt::Debug for PeekingSkipWhile<I, P>
where
    I: fmt::Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PeekingSkipWhile")
            .field("iter", &self.iter)
            .field("done_skipping", &self.done_skipping)
            .finish()
    }
}

impl<I> Peeking for &mut iter::Peekable<I>
where
    I: Iterator,
{
    fn peeking_take_while<P>(self, pred: P) -> PeekingTakeWhile<Self, P>
    where
        P: Fn(&Self::Item) -> bool,
    {
        PeekingTakeWhile::new(self, pred)
    }

    fn peeking_skip_while<P>(self, pred: P) -> PeekingSkipWhile<Self, P>
    where
        P: Fn(&Self::Item) -> bool,
    {
        PeekingSkipWhile::new(self, pred)
    }
}
mod tests {
    use super::*;
    use crate::char_utils::CharUtils;

    #[test]
    fn peeking_skip_shile_skips() {
        let mut input = "     some text".chars().peekable();
        let res = input
            .peeking_skip_while(|c| c.is_blank())
            .collect::<String>();
        assert_eq!("some text", res);
    }
}
