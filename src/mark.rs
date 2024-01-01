use std::cmp::min;

use crate::*;

#[derive(Clone, Copy)]
pub struct Mark {
    pub inner: usize,
}

impl Mark {
    pub fn set(&mut self, offset: usize) {
        self.inner |= 1 << offset;
    }

    pub fn unset(&mut self, offset: usize) {
        self.inner &= !(1 << offset);
    }

    pub fn is_marked(&self, offset: usize) -> bool {
        (self.inner | 1 << offset) == 1
    }
}

pub trait XMark {
    fn index_raw(&self) -> usize;
    fn index(&self) -> usize {
        let index = self.index_raw();
        debug_assert!(index < 3);
        index
    }
}

pub enum XMarkDemo {
    Dirty,
    COW,
    LOCKED,
}

impl XMark for XMarkDemo {
    fn index_raw(&self) -> usize {
        match self {
            XMarkDemo::Dirty => 0,
            XMarkDemo::COW => 1,
            XMarkDemo::LOCKED => 2,
        }
    }
}
