bitflags::bitflags! {
    #[derive(
        serde::Serialize,
        serde::Deserialize,
        Debug,
        Clone,
        Copy,
        PartialEq,
        Eq,
    )]
    pub struct Buttons: u8 {
        const A =      0b0000_0001;
        const B =      0b0000_0010;
        const SELECT = 0b0000_0100;
        const START =  0b0000_1000;
        const UP =     0b0001_0000;
        const DOWN =   0b0010_0000;
        const LEFT =   0b0100_0000;
        const RIGHT =  0b1000_0000;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NesInputFrame {
    pub player_one: Buttons,
    pub player_two: Buttons,
    pub microphone: bool,
}

impl Default for NesInputFrame {
    fn default() -> Self {
        Self {
            player_one: Buttons::empty(),
            player_two: Buttons::empty(),
            microphone: false,
        }
    }
}
