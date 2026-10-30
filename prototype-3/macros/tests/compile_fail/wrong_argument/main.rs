mod support;
use support::*;

fn main() {
    let model = model();
    let _view = wakui2_macros::view!("ui.yaml", model, fake);
}
