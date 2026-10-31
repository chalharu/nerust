use crate::backend;
use panechord_runtime::Binding;

pub struct NumberText;
impl NumberText {
    pub fn forward(value: f64) -> String {
        value.to_string()
    }
    pub fn backward(value: String) -> Result<f64, String> {
        value
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite() && (0.0..=1.0).contains(v))
            .ok_or_else(|| "enter a finite number between 0 and 1".into())
    }
}
pub async fn validate_name(value: String) -> Result<(), String> {
    futures_timer::Delay::new(std::time::Duration::from_millis(30)).await;
    if value.trim().is_empty() {
        Err("A name is required".into())
    } else {
        Ok(())
    }
}

// The GTK primitive is constructed by application code, not a compiler opcode.
#[cfg(feature = "backend-gtk")]
pub fn meter(view: &backend::View, parent: &backend::Parent, id: &str, value: Binding<f64>) {
    let meter = backend::gtk::LevelBar::for_interval(0.0, 1.0);
    view.mount_native(parent, id, &meter);
    view.bind_native(id, value, &meter, |meter, value| meter.set_value(value));
}
#[cfg(all(feature = "backend-tao", not(feature = "backend-gtk")))]
pub fn meter(view: &backend::View, parent: &backend::Parent, id: &str, value: Binding<f64>) {
    view.meter(parent, id, value);
}
