extern crate std;
use crate::*;
use std::sync::Arc;

extern crate test;
use test::Bencher;

fn init_continuous_with_arc<M: Into<XMark>>(xarray: &mut XArray<Arc<i32>, M>, item_num: i32) {
    for i in 0..item_num {
        let value = Arc::new(i);
        xarray.store(i as u64, value);
    }
}

fn init_sparse_with_arc<M: Into<XMark>>(xarray: &mut XArray<Arc<i32>, M>, item_num: i32) {
    for i in 0..2 * item_num {
        if i % 2 == 0 {
            let value = Arc::new(i);
            xarray.store(i as u64, value);
        }
    }
}

#[test]
fn test_store_continuous() {
    let mut xarray_arc: XArray<Arc<i32>> = XArray::new();
    init_continuous_with_arc(&mut xarray_arc, 10000);
    for i in 0..10000 {
        let value = xarray_arc.load(i as u64).unwrap();
        assert_eq!(*value.as_ref(), i);
    }
}

#[test]
fn test_store_sparse() {
    let mut xarray_arc: XArray<Arc<i32>> = XArray::new();
    init_sparse_with_arc(&mut xarray_arc, 10000);
    for i in 0..10000 {
        if i % 2 == 0 {
            let value = xarray_arc.load(i as u64).unwrap();
            assert_eq!(*value.as_ref(), i);
        }
    }
}

#[test]
fn test_store_overwrite() {
    let mut xarray_arc: XArray<Arc<i32>> = XArray::new();
    init_continuous_with_arc(&mut xarray_arc, 10000);
    // Overwrite 20 at index 10.
    let value = Arc::new(20);
    xarray_arc.store(10, value);
    let v = xarray_arc.load(10).unwrap();
    assert_eq!(*v.as_ref(), 20);
    // Overwrite 40 at index 10.
    let value = Arc::new(40);
    xarray_arc.store(10, value);
    let v = xarray_arc.load(10).unwrap();
    assert_eq!(*v.as_ref(), 40);
}

#[test]
fn test_remove() {
    let mut xarray_arc: XArray<Arc<i32>> = XArray::new();
    assert!(xarray_arc.remove(100).is_none());
    init_continuous_with_arc(&mut xarray_arc, 10000);

    for i in 0..10000 {
        assert_eq!(*xarray_arc.remove(i as u64).unwrap().as_ref(), i);
        let value = xarray_arc.load(i as u64);
        assert_eq!(value, None);
        assert!(xarray_arc.remove(i as u64).is_none());
    }
}

#[test]
fn test_cursor_load() {
    let mut xarray_arc: XArray<Arc<i32>> = XArray::new();
    init_continuous_with_arc(&mut xarray_arc, 10000);

    let mut cursor = xarray_arc.cursor(0);

    for i in 0..10000 {
        let value = cursor.load().unwrap();
        assert_eq!(*value.as_ref(), i);
        cursor.next();
    }

    cursor.reset_to(20000);
    assert!(cursor.load().is_none());
}

#[test]
fn test_cursor_store_continuous() {
    let mut xarray_arc: XArray<Arc<i32>> = XArray::new();
    let mut cursor = xarray_arc.cursor_mut(0);

    for i in 0..10000 {
        let value = Arc::new(i);
        cursor.store(value);
        cursor.next();
    }
    drop(cursor);

    for i in 0..10000 {
        let value = xarray_arc.load(i as u64).unwrap();
        assert_eq!(*value.as_ref(), i);
    }
}

#[test]
fn test_cursor_store_sparse() {
    let mut xarray_arc: XArray<Arc<i32>> = XArray::new();
    let mut cursor = xarray_arc.cursor_mut(0);

    for i in 0..10000 {
        if i % 2 == 0 {
            let value = Arc::new(i);
            cursor.store(value);
        }
        cursor.next();
    }
    drop(cursor);

    for i in 0..10000 {
        if i % 2 == 0 {
            let value = xarray_arc.load(i as u64).unwrap();
            assert_eq!(*value.as_ref(), i);
        }
    }
}

#[test]
fn test_set_mark() {
    let mut xarray_arc: XArray<Arc<i32>, XMark> = XArray::new();
    init_continuous_with_arc(&mut xarray_arc, 10000);

    let mut cursor = xarray_arc.cursor_mut(1000);
    cursor.set_mark(XMark::Mark0).unwrap();
    cursor.set_mark(XMark::Mark1).unwrap();
    cursor.reset_to(2000);
    cursor.set_mark(XMark::Mark1).unwrap();

    cursor.reset_to(1000);
    let value1_mark0 = cursor.is_marked(XMark::Mark0);
    let value1_mark1 = cursor.is_marked(XMark::Mark1);

    cursor.reset_to(2000);
    let value2_mark0 = cursor.is_marked(XMark::Mark0);
    let value2_mark1 = cursor.is_marked(XMark::Mark1);

    cursor.reset_to(3000);
    let value3_mark1 = cursor.is_marked(XMark::Mark1);

    assert!(value1_mark0 == true);
    assert!(value1_mark1 == true);
    assert!(value2_mark0 == false);
    assert!(value2_mark1 == true);
    assert!(value3_mark1 == false);
}

#[test]
fn test_unset_mark() {
    let mut xarray_arc: XArray<Arc<i32>, XMark> = XArray::new();
    init_continuous_with_arc(&mut xarray_arc, 10000);

    let mut cursor = xarray_arc.cursor_mut(1000);
    cursor.set_mark(XMark::Mark0).unwrap();
    cursor.set_mark(XMark::Mark1).unwrap();

    cursor.unset_mark(XMark::Mark0).unwrap();
    cursor.unset_mark(XMark::Mark2).unwrap();

    let value1_mark0 = cursor.is_marked(XMark::Mark0);
    let value1_mark2 = cursor.is_marked(XMark::Mark2);
    assert!(value1_mark0 == false);
    assert!(value1_mark2 == false);
}

#[test]
fn test_mark_overflow() {
    let mut xarray_arc: XArray<Arc<i32>, XMark> = XArray::new();
    init_continuous_with_arc(&mut xarray_arc, 10000);

    let mut cursor = xarray_arc.cursor_mut(20000);
    assert!(Err(()) == cursor.set_mark(XMark::Mark1));
    assert!(false == cursor.is_marked(XMark::Mark1));
}

#[test]
fn test_unset_mark_all() {
    let mut xarray_arc: XArray<Arc<i32>, XMark> = XArray::new();
    init_continuous_with_arc(&mut xarray_arc, 10000);
    xarray_arc.cursor_mut(2000).set_mark(XMark::Mark1);
    xarray_arc.cursor_mut(2000).set_mark(XMark::Mark2);
    xarray_arc.cursor_mut(200).set_mark(XMark::Mark1);
    xarray_arc.unset_mark_all(XMark::Mark1);

    assert_eq!(xarray_arc.cursor(2000).is_marked(XMark::Mark1), false);
    assert_eq!(xarray_arc.cursor(2000).is_marked(XMark::Mark2), true);
    assert_eq!(xarray_arc.cursor(200).is_marked(XMark::Mark1), false);
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
    // Init xarray_arc.
    let mut xarray_arc: XArray<Arc<Wrapper>> = XArray::new();
    for i in 1..10000 {
        let value = Arc::new(Wrapper::new(i * 2));
        xarray_arc.store(i as u64, value);
    }
    // Clone the xarray_arc.
    let mut xarray_clone = xarray_arc.clone();

    // Store different items in xarray_arc and xarray_clone respectively.
    for i in 1..10000 {
        if i % 2 == 0 {
            let value = Arc::new(Wrapper::new(i * 6));
            xarray_arc.store(i as u64, value);
        } else {
            let value = Arc::new(Wrapper::new(i * 8));
            xarray_clone.store(i as u64, value);
        }
    }
    // Determine whether they do not affect each other
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
    // Check drop times.
    assert!(INIT_TIMES.load(Ordering::Relaxed) == DROP_TIMES.load(Ordering::Relaxed));
}

#[test]
fn test_cow_after_cow() {
    let mut xarray_arc: XArray<Arc<u64>> = XArray::new();
    for i in 1..10000 {
        let value = Arc::new(i * 2);
        xarray_arc.store(i as u64, value);
    }
    // First COW.
    let mut xarray_cow1 = xarray_arc.clone();
    for i in 5000..7000 {
        let value = Arc::new(i * 3);
        xarray_cow1.store(i as u64, value);
    }
    // Second COW.
    let mut xarray_cow2 = xarray_arc.clone();
    for i in 6000..8000 {
        let value = Arc::new(i * 4);
        xarray_cow2.store(i as u64, value);
    }
    // COW after COW.
    let xarray_cow1_cow = xarray_cow1.clone();
    let xarray_cow2_cow = xarray_cow2.clone();

    assert!(*xarray_cow1_cow.load(2000).unwrap().as_ref() == 2000 * 2);
    assert!(*xarray_cow1_cow.load(5100).unwrap().as_ref() == 5100 * 3);
    assert!(*xarray_cow1_cow.load(6100).unwrap().as_ref() == 6100 * 3);
    assert!(*xarray_cow1_cow.load(7100).unwrap().as_ref() == 7100 * 2);

    assert!(*xarray_cow2_cow.load(2000).unwrap().as_ref() == 2000 * 2);
    assert!(*xarray_cow2_cow.load(5100).unwrap().as_ref() == 5100 * 2);
    assert!(*xarray_cow2_cow.load(6100).unwrap().as_ref() == 6100 * 4);
    assert!(*xarray_cow2_cow.load(7100).unwrap().as_ref() == 7100 * 4);
}

#[test]
fn test_cow_mark() {
    let mut xarray_arc: XArray<Arc<i32>, XMark> = XArray::new();
    for i in 1..10000 {
        let value = Arc::new(i * 2);
        xarray_arc.store(i as u64, value);
    }
    let mut xarray_clone = xarray_arc.clone();
    let mut cursor_arc = xarray_arc.cursor_mut(1000);
    let mut cursor_clone = xarray_clone.cursor_mut(1000);
    cursor_arc.set_mark(XMark::Mark0).unwrap();
    cursor_arc.reset_to(2000);
    cursor_arc.set_mark(XMark::Mark0).unwrap();
    cursor_arc.reset_to(3000);
    cursor_arc.set_mark(XMark::Mark0).unwrap();

    cursor_clone.set_mark(XMark::Mark1).unwrap();
    drop(cursor_arc);
    drop(cursor_clone);

    let mark0_1000_arc = xarray_arc.cursor(1000).is_marked(XMark::Mark0);
    let mark0_2000_arc = xarray_arc.cursor(2000).is_marked(XMark::Mark0);
    let mark1_1000_arc = xarray_arc.cursor(1000).is_marked(XMark::Mark1);
    let mark0_3000_arc = xarray_arc.cursor(3000).is_marked(XMark::Mark0);

    let mark0_1000_clone = xarray_clone.cursor(1000).is_marked(XMark::Mark0);
    let mark0_2000_clone = xarray_clone.cursor(2000).is_marked(XMark::Mark0);
    let mark1_1000_clone = xarray_clone.cursor(1000).is_marked(XMark::Mark1);
    let mark0_3000_clone = xarray_clone.cursor(3000).is_marked(XMark::Mark0);

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
fn test_cow_cursor() {
    let mut xarray_arc: XArray<Arc<u64>> = XArray::new();
    for i in 1..10000 {
        let value = Arc::new(i * 2);
        xarray_arc.store(i as u64, value);
    }
    let mut xarray_clone = xarray_arc.clone();

    let mut cursor_clone = xarray_clone.cursor_mut(1);
    let mut cursor_arc = xarray_arc.cursor_mut(1);
    // Use cursor to read xarray_clone;
    while cursor_clone.index() < 10000 {
        let item = cursor_clone.load().unwrap();
        assert!(*item.as_ref() == cursor_clone.index() * 2);
        cursor_clone.next();
    }

    // Use cursor to write xarray_clone;
    cursor_clone.reset_to(1);
    while cursor_clone.index() < 10000 {
        let value = Arc::new(cursor_clone.index());
        let item = cursor_clone.store(value).unwrap();
        assert!(*item.as_ref() == cursor_clone.index() * 2);
        cursor_clone.next();
    }

    // Use cursor to read xarray_arc;
    while cursor_arc.index() < 10000 {
        let item = cursor_arc.load().unwrap();
        assert!(*item.as_ref() == cursor_arc.index() * 2);
        cursor_arc.next();
    }

    // Use cursor to write xarray_arc;
    cursor_arc.reset_to(1);
    while cursor_arc.index() < 10000 {
        let value = Arc::new(cursor_arc.index() * 3);
        let item = cursor_arc.store(value).unwrap();
        assert!(*item.as_ref() == cursor_arc.index() * 2);
        cursor_arc.next();
    }

    // Use cursor to read xarray_arc and xarray_clone;
    cursor_arc.reset_to(1);
    cursor_clone.reset_to(1);
    while cursor_arc.index() < 10000 {
        let item_arc = cursor_arc.load().unwrap();
        let item_clone = cursor_clone.load().unwrap();
        assert!(*item_arc.as_ref() == cursor_arc.index() * 3);
        assert!(*item_clone.as_ref() == cursor_clone.index());
        cursor_arc.next();
        cursor_clone.next();
    }
}

#[test]
fn test_range() {
    let mut xarray_arc: XArray<Arc<i32>> = XArray::new();
    for i in 0..10000 {
        let value = Arc::new(i * 2);
        xarray_arc.store((i * 2) as u64, value);
    }

    let mut count = 0;
    for (index, item) in xarray_arc.range(1000..2000) {
        assert!(*item.as_ref() as u64 == index);
        count += 1;
    }
    assert!(count == 500);
}

#[bench]
fn benchmark_cursor_load(b: &mut Bencher) {
    b.iter(|| test_cursor_load());
}

#[bench]
fn benchmark_cursor_store_continuous(b: &mut Bencher) {
    b.iter(|| test_cursor_store_continuous());
}

#[bench]
fn benchmark_cursor_store_sparse(b: &mut Bencher) {
    b.iter(|| test_cursor_store_sparse());
}
