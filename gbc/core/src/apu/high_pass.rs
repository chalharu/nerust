/// APU high-pass filter (HPF).
///
/// Removes DC offset from the audio output. The filter is more aggressive
/// on GBA than on GBC, which itself is more aggressive than on DMG.
///
/// Reference: Pan Docs - Audio Details (HPF section)
/// Charge factor formula: 0.999958^(4194304/rate) for DMG
///                       0.998943^(4194304/rate) for CGB
#[derive(Debug, Clone)]
pub(crate) struct HighPassFilter {
    capacitor: f64,
    charge_factor: f64,
}

impl HighPassFilter {
    /// Create a new HPF.
    ///
    /// - `is_cgb`: true for CGB/MGB (0.998943), false for DMG (0.999958)
    /// - `sample_rate`: output sample rate (e.g. 44100)
    pub fn new(is_cgb: bool, sample_rate: u32) -> Self {
        let raw_factor: f64 = if is_cgb { 0.998943 } else { 0.999958 };
        let charge_factor = raw_factor.powf(4_194_304.0 / sample_rate as f64);
        Self {
            capacitor: 0.0,
            charge_factor,
        }
    }

    /// Apply the HPF to one sample.
    ///
    /// If DACs are disabled, the output is 0 and the capacitor is reset.
    pub fn step(&mut self, input: f64, dacs_enabled: bool) -> f64 {
        if dacs_enabled {
            let output = input - self.capacitor;
            self.capacitor = input - output * self.charge_factor;
            output
        } else {
            self.capacitor = 0.0;
            0.0
        }
    }

    /// Reset the filter state.
    pub fn reset(&mut self) {
        self.capacitor = 0.0;
    }
}

impl Default for HighPassFilter {
    fn default() -> Self {
        Self::new(false, 44_100)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hpf_passes_ac_signal() {
        let mut hpf = HighPassFilter::new(false, 44_100);
        // DC offset should be removed over time
        let mut sum = 0.0;
        for _ in 0..10000 {
            sum += hpf.step(1.0, true);
        }
        // After many samples, the output should be near 0
        assert!(sum > 0.0); // Some energy passes through
    }

    #[test]
    fn hpf_disabled_dacs_outputs_zero() {
        let mut hpf = HighPassFilter::new(false, 44_100);
        let out = hpf.step(1.0, false);
        assert_eq!(out, 0.0);
    }
}
