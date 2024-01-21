#[cfg(test)]
use super::*;
#[cfg(test)]
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
fn test_mark() {
    #[derive(Clone, Copy)]
    enum MarkDemo {
        Mark0,
        Mark1,
        Mark2,
    }

    impl ValidMark for MarkDemo {
        fn index_raw(&self) -> usize {
            match self {
                Self::Mark0 => 0,
                Self::Mark1 => 1,
                Self::Mark2 => 2,
            }
        }
    }

    let mut xarray_arc: XArray<Arc<i32>, MarkDemo> = XArray::new();
    for i in 1..10000 {
        let value = Arc::new(i * 2);
        xarray_arc.store(i as u64, value);
    }
    let mut cursor = xarray_arc.cursor_mut(1000);
    cursor.set_mark(MarkDemo::Mark0).unwrap();
    cursor.set_mark(MarkDemo::Mark1).unwrap();
    cursor.reset_to(2000);
    cursor.set_mark(MarkDemo::Mark1).unwrap();
    cursor.reset_to(20000);
    assert!(Err(()) == cursor.set_mark(MarkDemo::Mark1));
    assert!(None == cursor.load());
    let (value1, value1_mark0) = xarray_arc.load_with_mark(1000, MarkDemo::Mark0).unwrap();
    let (_, value1_mark1) = xarray_arc.load_with_mark(1000, MarkDemo::Mark1).unwrap();
    let (value2, value2_mark1) = xarray_arc.load_with_mark(2000, MarkDemo::Mark1).unwrap();
    let (_, value2_mark0) = xarray_arc.load_with_mark(2000, MarkDemo::Mark0).unwrap();
    let (value3, value3_mark1) = xarray_arc.load_with_mark(3000, MarkDemo::Mark1).unwrap();
    assert!(*value1.as_ref() == 2000);
    assert!(*value2.as_ref() == 4000);
    assert!(*value3.as_ref() == 6000);
    assert!(value1_mark0 == true);
    assert!(value1_mark1 == true);
    assert!(value2_mark0 == false);
    assert!(value2_mark1 == true);
    assert!(value3_mark1 == false);

    let mut cursor = xarray_arc.cursor_mut(1000);
    cursor.unset_mark(MarkDemo::Mark0).unwrap();
    cursor.unset_mark(MarkDemo::Mark2).unwrap();
    let (_, value1_mark0) = xarray_arc.load_with_mark(1000, MarkDemo::Mark0).unwrap();
    let (_, value1_mark2) = xarray_arc.load_with_mark(1000, MarkDemo::Mark2).unwrap();
    assert!(value1_mark0 == false);
    assert!(value1_mark2 == false);

    xarray_arc.unset_mark_all(MarkDemo::Mark1);
    let (_, value2_mark1) = xarray_arc.load_with_mark(2000, MarkDemo::Mark1).unwrap();
    assert!(value2_mark1 == false);
}

#[test]
fn test_cow() {
    use std::sync::atomic::AtomicU64;
    use std::sync::atomic::Ordering;

    static INIT_TIMES: AtomicU64 = AtomicU64::new(0);
    static DROP_TIMES: AtomicU64 = AtomicU64::new(0);
    struct Wrapper {
        raw: usize,
    }

    impl Drop for Wrapper {
        fn drop(&mut self) {
            DROP_TIMES.fetch_add(1, Ordering::Relaxed);
        }
    }

    impl Wrapper {
        fn new(raw: usize) -> Self {
            INIT_TIMES.fetch_add(1, Ordering::Relaxed);
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
    assert!(INIT_TIMES.load(Ordering::Relaxed) == DROP_TIMES.load(Ordering::Relaxed));
}

#[test]
fn test_cow_mark() {
    #[derive(Clone, Copy)]
    enum MarkDemo {
        Mark0,
        Mark1,
    }

    impl ValidMark for MarkDemo {
        fn index_raw(&self) -> usize {
            match self {
                Self::Mark0 => 0,
                Self::Mark1 => 1,
            }
        }
    }

    let mut xarray_arc: XArray<Arc<i32>, MarkDemo> = XArray::new();
    for i in 1..10000 {
        let value = Arc::new(i * 2);
        xarray_arc.store(i as u64, value);
    }
    let mut xarray_clone = xarray_arc.clone();
    let mut cursor_arc = xarray_arc.cursor_mut(1000);
    let mut cursor_clone = xarray_clone.cursor_mut(1000);
    cursor_arc.set_mark(MarkDemo::Mark0).unwrap();
    cursor_arc.reset_to(2000);
    cursor_arc.set_mark(MarkDemo::Mark0).unwrap();
    cursor_arc.reset_to(3000);
    cursor_arc.set_mark(MarkDemo::Mark0).unwrap();

    cursor_clone.set_mark(MarkDemo::Mark1).unwrap();

    let (_, mark0_1000_arc) = xarray_arc.load_with_mark(1000, MarkDemo::Mark0).unwrap();
    let (_, mark0_2000_arc) = xarray_arc.load_with_mark(2000, MarkDemo::Mark0).unwrap();
    let (_, mark1_1000_arc) = xarray_arc.load_with_mark(1000, MarkDemo::Mark1).unwrap();
    let (_, mark0_1000_clone) = xarray_clone.load_with_mark(1000, MarkDemo::Mark0).unwrap();
    let (_, mark0_2000_clone) = xarray_clone.load_with_mark(2000, MarkDemo::Mark0).unwrap();
    let (_, mark1_1000_clone) = xarray_clone.load_with_mark(1000, MarkDemo::Mark1).unwrap();
    let (_, mark0_3000_arc) = xarray_arc.load_with_mark(3000, MarkDemo::Mark0).unwrap();
    let (_, mark0_3000_clone) = xarray_clone.load_with_mark(3000, MarkDemo::Mark0).unwrap();

    assert!(mark0_1000_arc == true);
    assert!(mark0_2000_arc == true);
    assert!(mark1_1000_arc == false);
    assert!(mark0_1000_clone == false);
    assert!(mark0_2000_clone == false);
    assert!(mark1_1000_clone == true);
    assert!(mark0_3000_arc == true);
    assert!(mark0_3000_clone == false);
}

#[test]
fn test_next() {
    let mut xarray_arc: XArray<Arc<i32>> = XArray::new();
    for i in 1..10000 {
        let value = Arc::new(i * 2);
        xarray_arc.store(i as u64, value);
    }
    let mut cursor = xarray_arc.cursor_mut(0);
    for i in 1..10000 {
        cursor.next();
        let value = cursor.load().unwrap();
        assert!(*value.as_ref() == i * 2)
    }
    for i in 0..10000 {
        cursor.next();
        let value = Arc::new((10000 + i) * 2);
        cursor.store(value);
    }
    for i in 10000..20000 {
        let value = xarray_arc.load(i as u64).unwrap();
        assert!(*value.as_ref() == i * 2)
    }
}
