// Re-export `Controller`, `ControllerHub`, `ControllerCollection`,
// `OpenBusReadResult`, `Port`, and `SimplePort` from `nerust_input_traits`.
pub use nerust_input_traits::{
    Controller, ControllerCollection, ControllerHub, OpenBusReadResult, Port, SimplePort,
};

/// NES port constants indexed by CPU address ($4016 → index 0, $4017 → index 1).
pub const NES_PORTS: [SimplePort; 2] =
    [SimplePort::new(0, "player1"), SimplePort::new(1, "player2")];
