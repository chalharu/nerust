//! User-facing minimum: a model, its behavior, and a view macro. No contract crate.
#[cfg(feature = "backend-gtk")]
use panechord_gtk as backend;
use panechord_runtime::Store;
#[cfg(all(feature = "backend-tao", not(feature = "backend-gtk")))]
use panechord_tao as backend;

#[derive(Clone)]
struct Greeting {
    name: String,
}
#[panechord_macros::commands]
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
                name: "Panechord".into(),
            },
            executor,
        );
        let owner = model
            .try_into_owner()
            .unwrap_or_else(|_| panic!("shared model"));
        panechord_macros::owned_view!("ui/minimal.yaml", owner, backend)
    })
}
