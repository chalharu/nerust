pub mod screen_buffer;
mod screen_buffer_unit;

fn allocate<T: Default + Clone>(size: usize) -> Box<[T]> {
    // let mut buffer = Vec::with_capacity(size);
    // unsafe {
    //     buffer.set_len(size);
    // }
    let buffer = vec![T::default(); size];
    buffer.into_boxed_slice()
}
