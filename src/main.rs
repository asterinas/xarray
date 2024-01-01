#![feature(pointer_is_aligned)]
use core::prelude::v1;
use std::sync::Arc;

use entry::*;
use mark::*;
use node::*;
use state::*;
use xarray::*;

mod entry;
mod mark;
mod node;
mod state;
mod xarray;

#[derive(Debug)]
struct P {
    x: usize,
}

impl Drop for P {
    fn drop(&mut self) {
        println!("drop");
    }
}

fn main() {
    let mut xarray_arc: XArray<Arc<P>, XMarkDemo> = XArray::new();
    let v1 = Arc::new(P { x: 32 });
    xarray_arc.store(130, v1);
    let v1 = xarray_arc.load(130).unwrap();
    println!("arc:{:?}", v1);

    let mut xarray_usize: XArray<usize, XMarkDemo> = XArray::new();
    xarray_usize.store(100, 10);
    xarray_usize.store(8, 100);
    let v1 = xarray_usize.load(100);
    println!("load usize: {:?}", v1);
    let v2 = xarray_usize.load(8);
    println!("load usize: {:?}", v2);
}
