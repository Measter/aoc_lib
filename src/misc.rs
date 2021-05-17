use std::convert::TryInto;

// I'd rather stay with stable, so I guess I'll implement this myself.

#[derive(Debug, Clone)]
pub struct ArrWindows<'a, T, const N: usize>(&'a [T]);
impl<'a, T, const N: usize> Iterator for ArrWindows<'a, T, N> {
    type Item = &'a [T; N];

    fn next(&mut self) -> Option<Self::Item> {
        let next = self.0.get(..N)?;
        self.0 = self.0.get(1..)?;
        next.try_into().ok()
    }
}

impl<'a, T, const N: usize> ArrWindows<'a, T, N> {
    pub fn new(ts: &'a [T]) -> Self {
        Self(ts)
    }

    pub fn remaining(&self) -> &'a [T] {
        self.0
    }
}
