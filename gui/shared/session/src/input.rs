#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControllerInput {
    A,
    B,
    Select,
    Start,
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputState {
    Pressed,
    Released,
}
