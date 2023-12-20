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

#[derive(Debug)]
pub struct IdGen<'a, T, Id, IdToUsize, UsizeToId> {
    items: Vec<T>,
    utoid: IdToUsize,
    idtou: UsizeToId,
    map: HashMap<&'a str, Id>,
}

impl<'a, T, Id, UsizeToId, IdToUsize> IdGen<'a, T, Id, UsizeToId, IdToUsize>
where
    T: Default,
    Id: Copy,
    UsizeToId: Fn(usize) -> Id,
    IdToUsize: Fn(Id) -> usize,
{
    pub fn new(utoid: UsizeToId, idtou: IdToUsize) -> Self {
        Self {
            items: Vec::new(),
            utoid,
            idtou,
            map: HashMap::new(),
        }
    }

    pub fn id_of(&mut self, id: &'a str) -> Id {
        if let Some(&id) = self.map.get(id) {
            return id;
        }

        let new_id = (self.utoid)(self.map.len());
        self.items.push(T::default());
        self.map.insert(id, new_id);
        new_id
    }
}

impl<T, Id, UsizeToId, IdToUsize> IdGen<'_, T, Id, UsizeToId, IdToUsize> {
    pub fn into_items(self) -> Vec<T> {
        self.items
    }
}

impl<'a, T, Id, UsizeToId, IdToUsize> Index<Id> for IdGen<'a, T, Id, UsizeToId, IdToUsize>
where
    Id: Copy,
    IdToUsize: Fn(Id) -> usize,
{
    type Output = T;

    fn index(&self, index: Id) -> &Self::Output {
        let idx = (self.idtou)(index);
        &self.items[idx]
    }
}

impl<'a, T, Id, UsizeToId, IdToUsize> IndexMut<Id> for IdGen<'a, T, Id, UsizeToId, IdToUsize>
where
    Id: Copy,
    IdToUsize: Fn(Id) -> usize,
{
    fn index_mut(&mut self, index: Id) -> &mut Self::Output {
        let idx = (self.idtou)(index);
        &mut self.items[idx]
    }
}
