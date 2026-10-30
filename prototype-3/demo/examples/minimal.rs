//! User-facing minimum: a model, its behavior, and a view macro. No contract crate.
#[cfg(feature = "backend-gtk")]
use wakui2_gtk as backend;
use wakui2_runtime::Store;
#[cfg(all(feature = "backend-tao", not(feature = "backend-gtk")))]
use wakui2_tao as backend;

#[derive(Clone)]
struct Greeting {
    name: String,
}
#[wakui2_macros::commands]
impl Greeting {
    #[command]
    fn greet(&mut self) {
        self.name = format!("Hello, {}!", self.name);
    }
}
fn main() -> Result<(), String> {
    backend::run(|executor| {
        let model = Store::new(
            Greeting {
                name: "Wakui".into(),
            },
            executor,
        );
        let owner = model
            .try_into_owner()
            .unwrap_or_else(|_| panic!("shared model"));
        wakui2_macros::owned_view!("ui/minimal.yaml", owner, backend)
    })
}
