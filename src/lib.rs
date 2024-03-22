#![no_std]
#![allow(incomplete_features)]
#![feature(specialization)]
#![feature(strict_provenance)]
#![feature(test)]

extern crate alloc;

pub use cursor::{Cursor, CursorMut};
pub use entry::{ArcRef, BoxRef, ItemEntry};
pub use mark::XMark;
pub use range::Range;
pub use xarray::XArray;

mod borrow;
mod cursor;
mod entry;
mod mark;
mod node;
mod range;
mod xarray;

#[cfg(all(test, feature = "std"))]
mod test;
