use super::{envelope::Envelope, length_counter::LengthCounter, timer::Timer};

/// Duty cycle lookup table shared by both pulse channels.
const DUTY_TABLE: [[u8; 8]; 4] = [
    [0, 0, 0, 0, 0, 0, 0, 1], // 12.5%
    [1, 0, 0, 0, 0, 0, 0, 1], // 25%
    [1, 0, 0, 0, 0, 1, 1, 1], // 50%
    [0, 1, 1, 1, 1, 1, 1, 0], // 75%
];

pub fn step_pulse(timer: &mut Timer, duty_pos: &mut u8) {
    if timer.step() {
        *duty_pos = (*duty_pos + 1) & 7;
    }
}

pub fn pulse_output(
    envelope: &Envelope,
    duty: u8,
    duty_pos: u8,
    dac_enabled: bool,
    active: bool,
) -> u8 {
    if !dac_enabled || !active || DUTY_TABLE[duty as usize][duty_pos as usize] == 0 {
        0
    } else {
        envelope.output()
    }
}

pub fn update_dac(value: u8, dac_enabled: &mut bool, active: &mut bool) {
    *dac_enabled = value & 0xF8 != 0;
    if !*dac_enabled {
        *active = false;
    }
}

pub fn prepare_trigger(length: &mut LengthCounter, dac_enabled: bool, active: &mut bool) {
    if length.counter() == 0 {
        length.reload_at_zero();
        length.set_enabled(false);
    }
    if dac_enabled && !*active {
        *active = true;
    }
}

pub fn write_pulse_duty_length(value: u8, duty: &mut u8, length: &mut LengthCounter) {
    *duty = (value >> 6) & 3;
    length.load(value & 0x3F);
}

pub fn write_envelope(
    value: u8,
    envelope: &mut Envelope,
    dac_enabled: &mut bool,
    active: &mut bool,
) {
    envelope.reload_volume(value);
    update_dac(value, dac_enabled, active);
}

pub fn write_frequency_low(value: u8, frequency: &mut u16, timer: &mut Timer) {
    *frequency = (*frequency & 0x700) | u16::from(value);
    timer.set_period(2048u16.wrapping_sub(*frequency));
}

pub fn write_frequency_high(value: u8, frequency: &mut u16, timer: &mut Timer) {
    *frequency = (*frequency & 0xFF) | ((u16::from(value) & 0x07) << 8);
    timer.set_period(2048u16.wrapping_sub(*frequency));
}

/// Apply NRx4 length enable and the extra length clock hardware quirk.
pub fn apply_length_control(
    value: u8,
    next_div_lsb: bool,
    length: &mut LengthCounter,
    active: &mut bool,
) {
    let length_enable = value & 0x40 != 0;
    let triggered = value & 0x80 != 0;
    if length_enable && !length.enabled() && next_div_lsb && length.counter() > 0 {
        length.set_enabled(true);
        if length.clock() {
            if triggered {
                length.set_counter(length.max() - 1);
            } else {
                *active = false;
            }
        }
    }
    length.set_enabled(length_enable);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabling_dac_deactivates_channel() {
        let mut dac_enabled = true;
        let mut active = true;

        update_dac(0, &mut dac_enabled, &mut active);

        assert!(!dac_enabled);
        assert!(!active);
    }

    #[test]
    fn length_enable_quirk_deactivates_expired_channel() {
        let mut length = LengthCounter::new(64);
        length.load(63);
        let mut active = true;

        apply_length_control(0x40, true, &mut length, &mut active);

        assert_eq!(length.counter(), 0);
        assert!(!active);
    }

    #[test]
    fn triggered_length_enable_quirk_reloads_counter() {
        let mut length = LengthCounter::new(64);
        length.load(63);
        let mut active = true;

        apply_length_control(0xC0, true, &mut length, &mut active);

        assert_eq!(length.counter(), 63);
        assert!(active);
    }
}
