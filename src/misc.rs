mod iter_arr;
pub use iter_arr::*;

pub trait ResultZip<T, U, E> {
    fn zip(self, rhs: Result<U, E>) -> Result<(T, U), E>;
}

impl<T, U, E> ResultZip<T, U, E> for Result<T, E> {
    fn zip(self, rhs: Result<U, E>) -> Result<(T, U), E> {
        Ok((self?, rhs?))
    }
}

pub struct Top<T, const N: usize>(pub [T; N]);
impl<T: Ord, const N: usize> Top<T, N> {
    #[inline]
    pub fn push(&mut self, mut value: T) {
        for v in &mut self.0 {
            if &mut value > v {
                std::mem::swap(v, &mut value);
            }
        }
    }
}
