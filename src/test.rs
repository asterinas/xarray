extern crate std;
use crate::*;
use std::sync::Arc;

extern crate test;
use test::Bencher;

#[derive(Clone, Copy)]
enum MarkDemo {
    Mark0,
    Mark1,
    Mark2,
}

impl Into<XMark> for MarkDemo {
    fn into(self) -> XMark {
        match self {
            Self::Mark0 => XMark::Mark0,
            Self::Mark1 => XMark::Mark1,
            Self::Mark2 => XMark::Mark2,
        }
    }
}

#[test]
fn test_simple_store() {
    let mut xarray_arc: XArray<Arc<i32>, StdMutex> = XArray::new();
    for i in 0..10000 {
        let value = Arc::new(i * 2);
        xarray_arc.store((i * 3) as u64, value);
    }
    for i in 0..10000 {
        let value = xarray_arc.load((i * 3) as u64).unwrap();
        assert!(*value.as_ref() == i * 2)
    }
}

#[test]
fn test_overwrite_store() {
    let mut xarray_arc: XArray<Arc<i32>, StdMutex> = XArray::new();

    let value = Arc::new(20);
    xarray_arc.store(10, value);
    let v = xarray_arc.load(10).unwrap();
    assert!(*v.as_ref() == 20);

    let value = Arc::new(40);
    xarray_arc.store(10, value);
    let v = xarray_arc.load(10).unwrap();
    assert!(*v.as_ref() == 40);
}

#[test]
fn test_remove() {
    let mut xarray_arc: XArray<Arc<i32>, StdMutex> = XArray::new();
    assert!(xarray_arc.remove(66).is_none());
    for i in 0..10000 {
        let value = Arc::new(i * 2);
        xarray_arc.store(i as u64, value);
    }
    for i in 0..10000 {
        assert!(xarray_arc.remove(i as u64).is_some());
        let value = xarray_arc.load(i as u64);
        assert!(value == None);
        assert!(xarray_arc.remove(i as u64).is_none());
    }
}

#[test]
fn test_mark() {
    let mut xarray_arc: XArray<Arc<i32>, StdMutex, MarkDemo> = XArray::new();
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
    drop(cursor);

    let mut cursor = xarray_arc.cursor(1000);
    let value1 = cursor.load().unwrap();
    let value1_mark0 = cursor.is_marked(MarkDemo::Mark0);
    let value1_mark1 = cursor.is_marked(MarkDemo::Mark1);

    cursor.reset_to(2000);
    let value2 = cursor.load().unwrap();
    let value2_mark0 = cursor.is_marked(MarkDemo::Mark0);
    let value2_mark1 = cursor.is_marked(MarkDemo::Mark1);

    cursor.reset_to(3000);
    let value3 = cursor.load().unwrap();
    let value3_mark1 = cursor.is_marked(MarkDemo::Mark1);

    assert!(*value1.as_ref() == 2000);
    assert!(*value2.as_ref() == 4000);
    assert!(*value3.as_ref() == 6000);
    assert!(value1_mark0 == true);
    assert!(value1_mark1 == true);
    assert!(value2_mark0 == false);
    assert!(value2_mark1 == true);
    assert!(value3_mark1 == false);
    drop(cursor);

    let mut cursor = xarray_arc.cursor_mut(1000);
    cursor.unset_mark(MarkDemo::Mark0).unwrap();
    cursor.unset_mark(MarkDemo::Mark2).unwrap();
    drop(cursor);

    let cursor = xarray_arc.cursor(1000);
    let value1_mark0 = cursor.is_marked(MarkDemo::Mark0);
    let value1_mark2 = cursor.is_marked(MarkDemo::Mark2);
    assert!(value1_mark0 == false);
    assert!(value1_mark2 == false);
    drop(cursor);

    xarray_arc.unset_mark_all(MarkDemo::Mark1);
    let value2_mark1 = xarray_arc.cursor(2000).is_marked(MarkDemo::Mark1);
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
    let mut xarray_arc: XArray<Arc<Wrapper>, StdMutex> = XArray::new();
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
fn test_cow_after_cow() {
    let mut xarray_arc: XArray<Arc<u64>, StdMutex> = XArray::new();
    for i in 1..10000 {
        let value = Arc::new(i * 2);
        xarray_arc.store(i as u64, value);
    }
    // First COW.
    let mut xarray_cow1 = xarray_arc.clone();
    for i in 5000..6000 {
        let value = Arc::new(i * 3);
        xarray_cow1.store(i as u64, value);
    }
    // Second COW.
    let mut xarray_cow2 = xarray_arc.clone();
    for i in 5500..7000 {
        let value = Arc::new(i * 4);
        xarray_cow2.store(i as u64, value);
    }
    // COW after COW.
    let xarray_cow1_cow = xarray_cow1.clone();
    let xarray_cow2_cow = xarray_cow2.clone();

    assert!(*xarray_cow1_cow.load(2341).unwrap().as_ref() == 2341 * 2);
    assert!(*xarray_cow1_cow.load(5100).unwrap().as_ref() == 5100 * 3);
    assert!(*xarray_cow1_cow.load(5677).unwrap().as_ref() == 5677 * 3);
    assert!(*xarray_cow1_cow.load(6315).unwrap().as_ref() == 6315 * 2);

    assert!(*xarray_cow2_cow.load(2341).unwrap().as_ref() == 2341 * 2);
    assert!(*xarray_cow2_cow.load(5100).unwrap().as_ref() == 5100 * 2);
    assert!(*xarray_cow2_cow.load(5677).unwrap().as_ref() == 5677 * 4);
    assert!(*xarray_cow2_cow.load(6315).unwrap().as_ref() == 6315 * 4);
}

#[test]
fn test_cow_mark() {
    let mut xarray_arc: XArray<Arc<i32>, StdMutex, MarkDemo> = XArray::new();
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
    drop(cursor_arc);
    drop(cursor_clone);

    let mark0_1000_arc = xarray_arc.cursor(1000).is_marked(MarkDemo::Mark0);
    let mark0_2000_arc = xarray_arc.cursor(2000).is_marked(MarkDemo::Mark0);
    let mark1_1000_arc = xarray_arc.cursor(1000).is_marked(MarkDemo::Mark1);
    let mark0_3000_arc = xarray_arc.cursor(3000).is_marked(MarkDemo::Mark0);

    let mark0_1000_clone = xarray_clone.cursor(1000).is_marked(MarkDemo::Mark0);
    let mark0_2000_clone = xarray_clone.cursor(2000).is_marked(MarkDemo::Mark0);
    let mark1_1000_clone = xarray_clone.cursor(1000).is_marked(MarkDemo::Mark1);
    let mark0_3000_clone = xarray_clone.cursor(3000).is_marked(MarkDemo::Mark0);

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
    let mut xarray_arc: XArray<Arc<i32>, StdMutex> = XArray::new();
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
    drop(cursor);
    for i in 10000..20000 {
        let value = xarray_arc.load(i as u64).unwrap();
        assert!(*value.as_ref() == i * 2)
    }
}

#[test]
fn test_cow_next() {
    let mut xarray_arc: XArray<Arc<u64>, StdMutex> = XArray::new();
    for i in 1..10000 {
        let value = Arc::new(i * 2);
        xarray_arc.store(i as u64, value);
    }
    let mut xarray_clone = xarray_arc.clone();

    let mut cursor_clone = xarray_clone.cursor_mut(1);
    let mut cursor_arc = xarray_arc.cursor_mut(1);
    // Use next to read xarray_clone;
    while cursor_clone.index() < 10000 {
        let item = cursor_clone.load().unwrap();
        assert!(*item.as_ref() == cursor_clone.index() * 2);
        cursor_clone.next();
    }

    // Use next to write xarray_clone;
    cursor_clone.reset_to(1);
    while cursor_clone.index() < 10000 {
        let value = Arc::new(cursor_clone.index());
        let item = cursor_clone.store(value).unwrap();
        assert!(*item.as_ref() == cursor_clone.index() * 2);
        cursor_clone.next();
    }

    // Use next to read xarray_arc;
    while cursor_arc.index() < 10000 {
        let item = cursor_arc.load().unwrap();
        assert!(*item.as_ref() == cursor_arc.index() * 2);
        cursor_arc.next();
    }

    // Use next to write xarray_arc;
    cursor_arc.reset_to(1);
    while cursor_arc.index() < 10000 {
        let value = Arc::new(cursor_arc.index() * 3);
        let item = cursor_arc.store(value).unwrap();
        assert!(*item.as_ref() == cursor_arc.index() * 2);
        cursor_arc.next();
    }

    // Use next to read xarray_arc and xarray_clone;
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
    let mut xarray_arc: XArray<Arc<i32>, StdMutex> = XArray::new();
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
fn benchmark_next(b: &mut Bencher) {
    b.iter(|| test_next());
}
