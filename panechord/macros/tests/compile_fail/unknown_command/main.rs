mod support;
use support::*;

fn main() {
    let model = model();
    let _ = panechord_macros::view!("ui.yaml", model, fake);
}