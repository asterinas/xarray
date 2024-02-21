use core::ops::{Deref, DerefMut};

/// ValidLock is a trait that needs to be implemented for locks used internally within XArray.
/// It abstracts the functionalities of the necessary locks inside.
pub trait ValidLock<T>: Sized {
    type Target<'a>: Deref<Target = T> + DerefMut<Target = T>
    where
        Self: 'a;

    fn new(inner: T) -> Self;

    fn lock(&self) -> Self::Target<'_>;
}

/// XLock represents a HKT (Higher-Kinded Type) abstraction of ValidLock used within XArray,
/// leveraging Rust's GAT (Generic Associated Types) to empower an HKT.
///
/// This trait is typically auto-implemented via the abstract_lock_to! macro. For example, for a lock type Mutex<T>,
/// using `abstract_lock_to!(Mutex, XMutex);` yields the corresponding higher-kinded type XMutex,
/// which is automatically implemented with the XLock trait inside the macro. This allows XMutex to serve any type T,
/// obtaining the corresponding Mutex<T> by using `XMutex::Lock<T>`.
pub trait XLock {
    type Lock<T>: ValidLock<T>;

    fn new<T>(inner: T) -> Self::Lock<T> {
        Self::Lock::<T>::new(inner)
    }
}

#[macro_export]
/// Abstract a lock type that implements `ValidLock` to its HKT (Higher-Kinded Type) struct.
/// The first parameter is the source type name and the second parameter is the HKT type name.
/// This HKT type will implement `XLock` trait automatically and can be used as a generic parameter
/// for `XArray`.
macro_rules! abstract_lock_to {
    ($lock_type:ident, $name:ident) => {
        pub struct $name;

        impl XLock for $name {
            type Lock<T> = $lock_type<T>;
        }
    };
}

#[cfg(feature = "std")]
pub mod std_specific {
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
