mod support;
use support::*;

fn main() {
    let model = model();
    let view = panechord_macros::view!("ui.yaml", model, fake);
    println!("{}", view.output());
}