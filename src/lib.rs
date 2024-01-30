#![no_std]
#![allow(incomplete_features)]
#![feature(pointer_is_aligned)]
#![feature(specialization)]
#![feature(associated_type_defaults)]

extern crate alloc;
extern crate smallvec;


pub use cursor::*;
pub use entry::*;
pub use mark::*;
pub use xarray::*;

use cow::*;
use node::*;

mod cow;
mod cursor;
mod entry;
mod mark;
mod node;
mod xarray;

#[cfg(all(test, feature = "std"))]
mod test;

#[cfg(feature = "std")]
pub use std_specific::*;

#[cfg(feature = "std")]
mod std_specific {
    extern crate std;

    use crate::*;
    use std::sync::{Mutex, MutexGuard};

    impl<T> ValidLock<T> for Mutex<T> {
        type Target<'a> = MutexGuard<'a, T>
        where T: 'a;

        fn new(inner: T) -> Self {
            Mutex::new(inner)
        }

        fn lock(&self) -> Self::Target<'_> {
            self.lock().unwrap()
        }
    }

    abstract_lock_to!(Mutex, StdMutex);
}
