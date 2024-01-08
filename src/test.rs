use super::*;
use std::sync::Arc;

#[test]
fn test_store() {
    let mut xarray_arc: XArray<Arc<i32>> = XArray::new();
    for i in 1..10000 {
        let value = Arc::new(i * 2);
        xarray_arc.store((i * 3) as u64, value);
    }
    for i in 1..10000 {
        let value = xarray_arc.load((i * 3) as u64).unwrap();
        assert!(*value.as_ref() == i * 2)
    }
}

#[test]
fn test_remove() {
    let mut xarray_arc: XArray<Arc<i32>> = XArray::new();
    for i in 0..10000 {
        let value = Arc::new(i * 2);
        xarray_arc.store(i as u64, value);
    }
    for i in 0..10000 {
        xarray_arc.remove(i as u64);
        let value = xarray_arc.load(i as u64);
        assert!(value == None)
    }
}

#[test]
fn test_cow() {
    static mut INIT_COUNT: usize = 0;
    static mut DROP_COUNT: usize = 0;
    struct Wrapper {
        raw: usize,
    }

    impl Drop for Wrapper {
        fn drop(&mut self) {
            unsafe {
                DROP_COUNT += 1;
            }
        }
    }

    impl Wrapper {
        fn new(raw: usize) -> Self {
            unsafe {
                INIT_COUNT += 1;
            }
            Self { raw }
        }
    }
    let mut xarray_arc: XArray<Arc<Wrapper>> = XArray::new();
    for i in 1..10000 {
        let value = Arc::new(Wrapper::new(i * 2));
        xarray_arc.store(i as u64, value);
    }
    let mut xarray_clone = xarray_arc.clone();

    for i in 1..10000 {
        if i % 2 == 0 {
            let value = Arc::new(Wrapper::new(i * 6));
            xarray_arc.store(i as u64, value);
        } else {
            let value = Arc::new(Wrapper::new(i * 8));
            xarray_clone.store(i as u64, value);
        }
    }

    for i in 1..10000 {
        let value_origin = xarray_arc.load(i).unwrap();
        let value_clone = xarray_clone.load(i).unwrap();
        if i % 2 == 0 {
            assert!(value_origin.raw as u64 == i * 6);
            assert!(value_clone.raw as u64 == i * 2);
        } else {
            assert!(value_origin.raw as u64 == i * 2);
            assert!(value_clone.raw as u64 == i * 8);
        }
    }
    drop(xarray_arc);
    drop(xarray_clone);
    unsafe {
        assert!(INIT_COUNT == DROP_COUNT);
    }
}
