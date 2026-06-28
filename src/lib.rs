mod inner;

pub fn run() {
    #[cfg(not(any(feature = "gtk", feature = "tao", clippy)))]
    compile_error!("No frontend selected. Enable feature 'gtk' or 'tao'.");
    #[cfg(any(feature = "gtk", feature = "tao"))]
    inner::run();
}
