use nerust_core::controller::Controller;

pub trait ControllerRuntime: Controller + Send {
    fn reset_runtime(&mut self);
    fn apply_input_state(&mut self, bytes: &[u8]) -> Result<(), String>;
    fn validate_controller_state(&self, bytes: &[u8]) -> Result<(), String>;
    fn apply_controller_state(&mut self, bytes: &[u8]) -> Result<(), String>;
    fn current_controller_state(&self) -> Result<Vec<u8>, String>;
    fn current_input_state(&self) -> Result<Vec<u8>, String>;
}
