mod support;
use support::*;

fn main() {
    let model = model();
    let _view = panechord_macros::view!("ui.yaml", model, fake);
}
