use nerust_input_traits::SimplePort;

/// NES port constants indexed by CPU address ($4016 → index 0, $4017 → index 1).
pub const NES_PORTS: [SimplePort; 2] = [
    SimplePort::new(0, "nes.attachment.player1"),
    SimplePort::new(1, "nes.attachment.player2"),
];
