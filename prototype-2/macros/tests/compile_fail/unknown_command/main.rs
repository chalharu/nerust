mod support;
use support::*;

fn main() {
    let model = model();
    let _ = wakui2_macros::view!("ui.yaml", model, fake);
}