#![no_std]
#![allow(incomplete_features)]
#![feature(pointer_is_aligned)]
#![feature(specialization)]
#![feature(associated_type_defaults)]

extern crate alloc;
extern crate smallvec;

use cow::*;
use cursor::*;
use entry::*;
use mark::*;
use node::*;
pub use xarray::*;

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

    pub struct StdMutex;

    impl XLock for StdMutex {
        type Lock<T> = Mutex<T>;
    }
}
