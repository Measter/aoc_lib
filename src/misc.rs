use std::{convert::TryInto, marker::PhantomData, ptr::NonNull};

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

    fn size_hint(&self) -> (usize, Option<usize>) {
        let len = self.0.len().saturating_sub(N - 1);
        (len, Some(len))
    }
}

impl<'a, T, const N: usize> ExactSizeIterator for ArrWindows<'a, T, N> {}

impl<'a, T, const N: usize> ArrWindows<'a, T, N> {
    pub fn new(ts: &'a [T]) -> Self {
        assert!(N > 0);
        Self(ts)
    }

    pub fn remaining(&self) -> &'a [T] {
        self.0
    }
}

pub struct ArrChunks<'a, T, const N: usize> {
    slice: &'a [T],
}

impl<'a, T, const N: usize> ArrChunks<'a, T, N> {
    pub fn new(slice: &'a [T]) -> Self {
        assert!(N > 0);
        Self { slice }
    }
}

impl<'a, T, const N: usize> Iterator for ArrChunks<'a, T, N> {
    type Item = &'a [T; N];
    fn next(&mut self) -> Option<Self::Item> {
        let next = self.slice.get(..N)?;
        self.slice = self.slice.get(N..)?;
        next.try_into().ok()
    }
}

pub struct ArrChunksMut<'a, T, const N: usize> {
    slice: NonNull<[T]>,
    _marker: PhantomData<&'a mut T>,
}

impl<'a, T, const N: usize> Iterator for ArrChunksMut<'a, T, N> {
    type Item = &'a mut [T; N];

    fn next(&mut self) -> Option<Self::Item> {
        // SAFETY: We only construct self.slice from an existing reference, which cannot be null.
        // We additionally only pass out non-overlapping subslices, so we can't end up with
        // multiple unique references to the same data.
        unsafe {
            let slice = self.slice.as_mut();
            if slice.len() < N {
                return None;
            }

            let (start, end) = slice.split_at_mut(N);
            self.slice = NonNull::new(end).unwrap();
            start.try_into().ok()
        }
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        // SAFETY: See comment in `next` implementation.
        unsafe {
            let slice = self.slice.as_ref();

            if slice.len() < N {
                (0, Some(0))
            } else {
                let len = slice.len() / N;
                (len, Some(len))
            }
        }
    }
}

impl<'a, T, const N: usize> ExactSizeIterator for ArrChunksMut<'a, T, N> {}

impl<'a, T, const N: usize> ArrChunksMut<'a, T, N> {
    pub fn new(ts: &'a mut [T]) -> Self {
        assert!(N > 0);
        Self {
            slice: NonNull::new(ts).unwrap(),
            _marker: PhantomData,
        }
    }
}
