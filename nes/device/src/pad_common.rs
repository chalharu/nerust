/// Shared strobe write logic.
pub fn write<const N: usize>(strobe: &mut bool, cached: &[u8; N], result: &mut [u8; N], value: u8) {
    let new_strobe = value & 1 == 1;
    // 1 -> 0 に変化したとき
    if *strobe && !new_strobe {
        *result = *cached;
    }
    *strobe = new_strobe;
}
