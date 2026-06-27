#[cfg(feature = "gtk")]
fn main() {
    nerust_gtk::run();
}

#[cfg(feature = "tao")]
fn main() {
    nerust_tao::run();
}

#[cfg(not(any(feature = "gtk", feature = "tao")))]
fn main() {
    panic!("No frontend selected. Enable feature 'gtk' (default) or 'tao'.");
}
