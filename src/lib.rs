#![no_std]
#![allow(incomplete_features)]
#![feature(pointer_is_aligned)]
#![feature(specialization)]
#![feature(associated_type_defaults)]
#![feature(never_type)]
#![feature(test)]

extern crate alloc;

pub use cursor::{Cursor, CursorMut};
pub use entry::ItemEntry;
pub use lock::{ValidLock, XLock};
pub use mark::XMark;
pub use range::Range;
pub use xarray::XArray;

#[cfg(feature = "std")]
pub use lock::std_specific::*;

mod cow;
mod cursor;
mod entry;
mod lock;
mod mark;
mod node;
mod range;
mod xarray;

#[cfg(all(test, feature = "std"))]
mod test;
