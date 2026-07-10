/// Shared strobe write logic for a single port.
pub fn write(strobe: &mut bool, cached: &u8, result: &mut u8, value: u8) {
    let new_strobe = value & 1 == 1;
    if *strobe && !new_strobe {
        *result = *cached;
    }
    *strobe = new_strobe;
}
