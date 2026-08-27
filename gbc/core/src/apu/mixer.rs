/// APU stereo mixer.
///
/// Combines the outputs of the four channels according to NR50 and NR51.
/// NR50 controls master volume for left/right outputs.
/// NR51 controls which channels are connected to each output.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct Mixer {
    /// NR50: Master volume (FF24)
    /// bits 6-4: left volume (0-7, treated as 1-8)
    /// bits 2-0: right volume (0-7, treated as 1-8)
    nr50: u8,
    /// NR51: Sound panning (FF25)
    /// bits 7-4: CH4-CH1 left enable
    /// bits 3-0: CH4-CH1 right enable
    nr51: u8,
}

impl Mixer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set NR50 register value.
    pub fn write_nr50(&mut self, value: u8) {
        self.nr50 = value;
    }

    /// Set NR51 register value.
    pub fn write_nr51(&mut self, value: u8) {
        self.nr51 = value;
    }

    pub fn nr50(&self) -> u8 {
        self.nr50
    }

    pub fn nr51(&self) -> u8 {
        self.nr51
    }

    /// Mix the four channel outputs into stereo.
    ///
    /// Each channel outputs a digital value 0-15.
    /// Returns (left, right) as float values normalized to 0.0-1.0.
    pub fn mix(&self, ch1: u8, ch2: u8, ch3: u8, ch4: u8) -> (f32, f32) {
        // NR50: volume 0-7 is treated as 1-8
        let left_vol = (((self.nr50 >> 4) & 7) as f32) + 1.0;
        let right_vol = ((self.nr50 & 7) as f32) + 1.0;

        let mut left = 0.0f32;
        let mut right = 0.0f32;

        // NR51: left channels (bits 7-4), right channels (bits 3-0)
        if self.nr51 & 0x10 != 0 {
            left += ch1 as f32;
        }
        if self.nr51 & 0x20 != 0 {
            left += ch2 as f32;
        }
        if self.nr51 & 0x40 != 0 {
            left += ch3 as f32;
        }
        if self.nr51 & 0x80 != 0 {
            left += ch4 as f32;
        }
        if self.nr51 & 0x01 != 0 {
            right += ch1 as f32;
        }
        if self.nr51 & 0x02 != 0 {
            right += ch2 as f32;
        }
        if self.nr51 & 0x04 != 0 {
            right += ch3 as f32;
        }
        if self.nr51 & 0x08 != 0 {
            right += ch4 as f32;
        }

        // Normalize: each channel 0-15, 4 channels max 60
        // Volume 1-8, so max total = 60 * 8 = 480
        left *= left_vol / 480.0;
        right *= right_vol / 480.0;

        (left, right)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixer_silence_when_all_disabled() {
        let mixer = Mixer {
            nr50: 0x77, // max volume
            nr51: 0x00, // no channels
        };
        let (l, r) = mixer.mix(15, 15, 15, 15);
        assert_eq!(l, 0.0);
        assert_eq!(r, 0.0);
    }

    #[test]
    fn mixer_full_volume_all_channels() {
        let mixer = Mixer {
            nr50: 0x77, // max volume (7+1=8)
            nr51: 0xFF, // all channels to both outputs
        };
        let (l, r) = mixer.mix(15, 15, 15, 15);
        // 60 * 8 / 480 = 1.0
        assert!((l - 1.0).abs() < f32::EPSILON);
        assert!((r - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn mixer_left_only() {
        let mixer = Mixer {
            nr50: 0x77,
            nr51: 0xF0, // all channels left only
        };
        let (l, r) = mixer.mix(15, 15, 15, 15);
        assert!((l - 1.0).abs() < f32::EPSILON);
        assert_eq!(r, 0.0);
    }

    #[test]
    fn mixer_right_only() {
        let mixer = Mixer {
            nr50: 0x77,
            nr51: 0x0F, // all channels right only
        };
        let (l, r) = mixer.mix(15, 15, 15, 15);
        assert_eq!(l, 0.0);
        assert!((r - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn mixer_applies_left_and_right_volume_independently() {
        let mixer = Mixer {
            nr50: 0x73, // left 8/8, right 4/8
            nr51: 0xFF,
        };
        let (l, r) = mixer.mix(15, 15, 15, 15);
        assert!((l - 1.0).abs() < f32::EPSILON);
        assert!((r - 0.5).abs() < f32::EPSILON);
    }
}
