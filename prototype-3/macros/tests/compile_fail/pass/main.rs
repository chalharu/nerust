mod support;
use support::*;

fn main() {
    let model = model();
    let view = wakui2_macros::view!("ui.yaml", model, fake);
    println!("{}", view.output());
}