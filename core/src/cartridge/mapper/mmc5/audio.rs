use super::Mmc5;
use crate::apu::envelope::{Envelope, EnvelopeDao, HaveEnvelopeDao};
use crate::apu::length_counter::{
    HaveLengthCounter, HaveLengthCounterDao, LengthCounter, LengthCounterDao,
};
use crate::apu::timer::{HaveTimerDao, TimerDao};
use crate::cpu::interrupt::Interrupt;
use crate::persistence::PersistenceError;
use prost::Message;

const DUTY_TABLE: [[bool; 8]; 4] = [
    [false, true, false, false, false, false, false, false],
    [false, true, true, false, false, false, false, false],
    [false, true, true, true, true, false, false, false],
    [true, false, false, true, true, true, true, true],
];

const AUDIO_CLOCK_RATE: u64 = 1_789_773;

#[derive(Clone, PartialEq, Message)]
pub(super) struct Mmc5PulseMessage {
    #[prost(uint32, tag = "1")]
    pub(super) duty_mode: u32,
    #[prost(uint32, tag = "2")]
    pub(super) duty_value: u32,
    #[prost(uint32, tag = "3")]
    pub(super) period: u32,
    #[prost(message, optional, tag = "4")]
    pub(super) envelope: Option<crate::persistence::EnvelopeDaoMessage>,
    #[prost(message, optional, tag = "5")]
    pub(super) length_counter: Option<crate::persistence::LengthCounterDaoMessage>,
    #[prost(message, optional, tag = "6")]
    pub(super) timer: Option<crate::persistence::TimerDaoMessage>,
}

#[derive(serde_derive::Serialize, serde_derive::Deserialize, Debug, Clone, Copy)]
pub(super) struct Mmc5Pulse {
    duty_mode: u8,
    duty_value: u8,
    period: u16,
    envelope: EnvelopeDao,
    length_counter: LengthCounterDao,
    timer: TimerDao,
}

impl HaveLengthCounterDao for Mmc5Pulse {
    fn length_counter_dao(&self) -> &LengthCounterDao {
        &self.length_counter
    }

    fn length_counter_dao_mut(&mut self) -> &mut LengthCounterDao {
        &mut self.length_counter
    }
}

impl HaveEnvelopeDao for Mmc5Pulse {
    fn envelope_dao(&self) -> &EnvelopeDao {
        &self.envelope
    }

    fn envelope_dao_mut(&mut self) -> &mut EnvelopeDao {
        &mut self.envelope
    }
}

impl HaveLengthCounter for Mmc5Pulse {
    type LengthCounter = Self;

    fn length_counter(&self) -> &Self::LengthCounter {
        self
    }

    fn length_counter_mut(&mut self) -> &mut Self::LengthCounter {
        self
    }
}

impl HaveTimerDao for Mmc5Pulse {
    fn timer_dao(&self) -> &TimerDao {
        &self.timer
    }

    fn timer_dao_mut(&mut self) -> &mut TimerDao {
        &mut self.timer
    }
}

impl Mmc5Pulse {
    pub(super) fn new() -> Self {
        Self {
            duty_mode: 0,
            duty_value: 0,
            period: 0,
            envelope: EnvelopeDao::new(),
            length_counter: LengthCounterDao::new(),
            timer: TimerDao::new(),
        }
    }

    pub(super) fn write_control(&mut self, value: u8) {
        self.length_counter.set_halt((value & 0x20) != 0);
        self.envelope.set_enabled((value & 0x10) == 0);
        self.envelope.set_period(value & 0x0F);
        self.duty_mode = (value >> 6) & 0x03;
    }

    pub(super) fn write_timer_low(&mut self, value: u8) {
        self.set_period((self.period & 0xFF00) | u16::from(value));
    }

    pub(super) fn write_timer_high(&mut self, value: u8) {
        self.length_counter.set_load(value >> 3);
        self.set_period((self.period & 0x00FF) | (u16::from(value & 0x07) << 8));
        self.duty_value = 0;
        self.envelope.restart();
    }

    pub(super) fn set_enabled(&mut self, enabled: bool) {
        LengthCounter::set_enabled(self, enabled);
    }

    pub(super) fn step_timer(&mut self) {
        if self.timer.step_timer() {
            self.duty_value = self.duty_value.wrapping_sub(1) & 0x07;
        }
    }

    pub(super) fn step_frame(&mut self) {
        self.step_envelope();
        self.step_length();
    }

    pub(super) fn output(&self) -> u8 {
        if !DUTY_TABLE[usize::from(self.duty_mode)][usize::from(self.duty_value)] {
            0
        } else {
            Envelope::get_volume(self)
        }
    }

    pub(super) fn status(&self) -> bool {
        self.get_status()
    }

    fn set_period(&mut self, period: u16) {
        self.period = period;
        self.timer.set_period((period << 1) + 1);
    }

    pub(super) fn export_state_proto(&self) -> Mmc5PulseMessage {
        Mmc5PulseMessage {
            duty_mode: u32::from(self.duty_mode),
            duty_value: u32::from(self.duty_value),
            period: u32::from(self.period),
            envelope: Some(self.envelope.export_state_proto()),
            length_counter: Some(self.length_counter.export_state_proto()),
            timer: Some(self.timer.export_state_proto()),
        }
    }

    pub(super) fn import_state_proto(
        &mut self,
        payload: &Mmc5PulseMessage,
    ) -> Result<(), PersistenceError> {
        self.duty_mode = u8::try_from(payload.duty_mode)
            .map_err(|_| PersistenceError::Validation("MMC5 pulse duty_mode overflow".into()))?;
        self.duty_value = u8::try_from(payload.duty_value)
            .map_err(|_| PersistenceError::Validation("MMC5 pulse duty_value overflow".into()))?;
        self.period = u16::try_from(payload.period)
            .map_err(|_| PersistenceError::Validation("MMC5 pulse period overflow".into()))?;
        self.envelope.import_state_proto(
            payload.envelope.as_ref().ok_or_else(|| {
                PersistenceError::Validation("missing MMC5 pulse envelope".into())
            })?,
        )?;
        self.length_counter
            .import_state_proto(payload.length_counter.as_ref().ok_or_else(|| {
                PersistenceError::Validation("missing MMC5 pulse length counter".into())
            })?)?;
        self.timer.import_state_proto(
            payload
                .timer
                .as_ref()
                .ok_or_else(|| PersistenceError::Validation("missing MMC5 pulse timer".into()))?,
        )?;
        Ok(())
    }
}

impl Mmc5 {
    pub(super) fn clock_audio(&mut self, interrupt: &mut Interrupt) {
        self.pulse_1.step_length_counter();
        self.pulse_2.step_length_counter();
        self.pulse_1.step_timer();
        self.pulse_2.step_timer();
        self.audio_frame_accumulator += 240;
        if self.audio_frame_accumulator >= AUDIO_CLOCK_RATE {
            self.audio_frame_accumulator -= AUDIO_CLOCK_RATE;
            self.pulse_1.step_frame();
            self.pulse_2.step_frame();
        }
        self.update_external_irq(interrupt);
    }

    pub(super) fn write_pcm_sample(&mut self, value: u8, interrupt: &mut Interrupt) {
        if value == 0 {
            self.pcm_irq_pending = true;
        } else {
            self.pcm_output = value;
            self.pcm_irq_pending = false;
        }
        self.update_external_irq(interrupt);
    }

    pub(super) fn read_audio_status(&self) -> u8 {
        (if self.pulse_1.status() { 0x01 } else { 0 })
            | if self.pulse_2.status() { 0x02 } else { 0 }
    }

    pub(super) fn audio_output(&self) -> f32 {
        self.pulse_table[usize::from(self.pulse_1.output()) + usize::from(self.pulse_2.output())]
            + self.pcm_table[usize::from(self.pcm_output)]
    }
}
