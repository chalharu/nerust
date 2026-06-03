pub mod resampler;

use std::f32;

pub trait Filter {
    fn step(&mut self, data: f32) -> f32;
    fn chain<F: Filter>(self, filter: F) -> ChaindFilter<Self, F>
    where
        Self: Sized,
    {
        ChaindFilter::create(self, filter)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct IirFilter {
    b0: f32,
    b1: f32,
    a1: f32,
    prev_data: f32,
    prev_result: f32,
}

// 双一次変換を利用する
impl IirFilter {
    pub fn get_highpass_filter(sample_rate: f32, cutoff_freq: f32) -> Self {
        let t = 1.0 / sample_rate;
        let omega_c = 2.0 * f32::consts::PI * cutoff_freq;
        let c = (omega_c * t / 2.0).tan();

        let b0 = 1.0 / (1.0 + c);
        let b1 = -b0;
        let a1 = (1.0 - c) / (1.0 + c);

        Self {
            b0,
            b1,
            a1,
            prev_result: 0.0,
            prev_data: 0.0,
        }
    }
    pub fn get_lowpass_filter(sample_rate: f32, cutoff_freq: f32) -> Self {
        let t = 1.0 / sample_rate;
        let omega_c = 2.0 * f32::consts::PI * cutoff_freq;
        let c = (omega_c * t / 2.0).tan();

        let b0 = c / (1.0 + c);
        let b1 = b0;
        let a1 = (1.0 - c) / (1.0 + c);

        Self {
            b0,
            b1,
            a1,
            prev_result: 0.0,
            prev_data: 0.0,
        }
    }
}

impl Filter for IirFilter {
    fn step(&mut self, data: f32) -> f32 {
        self.prev_result = self.b0 * data + self.b1 * self.prev_data + self.a1 * self.prev_result;
        self.prev_data = data;
        self.prev_result
    }
}

#[derive(Debug)]
pub struct ChaindFilter<F1: Filter, F2: Filter> {
    filter1: F1,
    filter2: F2,
}

impl<F1: Filter, F2: Filter> ChaindFilter<F1, F2> {
    fn create(filter1: F1, filter2: F2) -> Self {
        Self { filter1, filter2 }
    }
}

impl<F1: Filter, F2: Filter> Filter for ChaindFilter<F1, F2> {
    fn step(&mut self, data: f32) -> f32 {
        self.filter2.step(self.filter1.step(data))
    }
}

pub type NesFilter = ChaindFilter<ChaindFilter<IirFilter, IirFilter>, IirFilter>;
pub type SnesFilter = ChaindFilter<IirFilter, IirFilter>;

impl NesFilter {
    pub fn new(sample_rate: f32) -> Self {
        IirFilter::get_lowpass_filter(sample_rate, 14000.0)
            .chain(IirFilter::get_highpass_filter(sample_rate, 90.0))
            .chain(IirFilter::get_highpass_filter(sample_rate, 442.0))
    }
}

impl SnesFilter {
    pub fn new(sample_rate: f32) -> Self {
        let lowpass_cutoff = 16000.0_f32.min(sample_rate * 0.45).max(1.0);
        let highpass_cutoff = 20.0_f32.min(sample_rate * 0.10).max(1.0);
        IirFilter::get_lowpass_filter(sample_rate, lowpass_cutoff)
            .chain(IirFilter::get_highpass_filter(sample_rate, highpass_cutoff))
    }
}
