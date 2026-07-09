use nerust_nes_core::OpenBusReadResult;

/// Shared shift-register read logic for NES-style controllers.
/// `cached` is [p1_byte, p2_byte] (no mic).
/// Returns the shifted bit plus mic on D2 when address==0 `mic_d2` is true.
pub fn read(
    cached: &[u8; 2],
    index: &mut [u8; 2],
    strobe: bool,
    address: usize,
    mic_d2: bool,
) -> OpenBusReadResult {
    match address {
        0 => {
            let bit = if index[0] < 8 {
                let b = (cached[0] >> index[0]) & 1;
                if !strobe {
                    index[0] += 1;
                }
                b
            } else {
                1
            };
            let mic = if mic_d2 { 4 } else { 0 };
            OpenBusReadResult::new(bit | mic, 7)
        }
        _ => {
            let bit = if index[1] < 8 {
                let b = (cached[1] >> index[1]) & 1;
                if !strobe {
                    index[1] += 1;
                }
                b
            } else {
                1
            };
            OpenBusReadResult::new(bit, 0x1F)
        }
    }
}

/// Shared strobe write logic.
pub fn write(strobe: &mut bool, index: &mut [u8; 2], value: u8) {
    *strobe = value & 1 == 1;
    if *strobe {
        *index = [0, 0];
    }
}
