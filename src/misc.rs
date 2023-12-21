mod iter_arr;
use std::{
    collections::HashMap,
    ops::{Index, IndexMut},
};

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

pub trait IdType {
    fn from_usize(i: usize) -> Self;
    fn to_usize(self) -> usize;
}

#[derive(Debug)]
pub struct IdGen<'a, T, I> {
    items: Vec<T>,
    map: HashMap<&'a str, I>,
}

impl<'a, T, I> IdGen<'a, T, I>
where
    T: Default,
    I: Copy + IdType,
{
    pub fn id_of(&mut self, id: &'a str) -> I {
        if let Some(&id) = self.map.get(id) {
            return id;
        }

        let new_id = I::from_usize(self.map.len());
        self.items.push(T::default());
        self.map.insert(id, new_id);
        new_id
    }
}

impl<'a, T, I> Default for IdGen<'a, T, I> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, I> IdGen<'_, T, I> {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            map: HashMap::new(),
        }
    }
    pub fn into_items(self) -> Vec<T> {
        self.items
    }
}

impl<'a, T, I> Index<I> for IdGen<'a, T, I>
where
    I: Copy + IdType,
{
    type Output = T;

    fn index(&self, index: I) -> &Self::Output {
        &self.items[index.to_usize()]
    }
}

impl<'a, T, I> IndexMut<I> for IdGen<'a, T, I>
where
    I: Copy + IdType,
{
    fn index_mut(&mut self, index: I) -> &mut Self::Output {
        &mut self.items[index.to_usize()]
    }
}
