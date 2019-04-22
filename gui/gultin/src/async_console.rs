// Copyright (c) 2018 Mitsuharu Seki
//
// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at http://mozilla.org/MPL/2.0/.

use crc::crc64;
use nerust_core::controller::standard_controller::{Buttons, StandardController};
use nerust_core::Console;
use nerust_screen_buffer::ScreenBuffer;
use nerust_screen_traits::LogicalSize;
use nerust_sound_traits::{MixerInput, Sound};
use nerust_timer::Timer;
use std::hash::{Hash, Hasher};
use std::sync::atomic::AtomicPtr;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::{mem, thread};

pub struct AsyncConsole {
    stop_sender: Sender<()>,
    data_sender: Sender<ConsoleData>,
    thread: Option<JoinHandle<()>>,

    logical_size: LogicalSize,
    screen_buffer_ptr: Arc<AtomicPtr<u8>>,
}

impl AsyncConsole {
    pub fn new<S: 'static + Sound + MixerInput + Send>(
        speaker: S,
        mut screen_buffer: ScreenBuffer,
    ) -> Self {
        let (data_sender, data_recv) = channel();
        let (stop_sender, stop_recv) = channel();
        let logical_size = screen_buffer.logical_size();
        let screen_buffer_ptr = Arc::new(AtomicPtr::new(screen_buffer.as_mut_ptr()));

        let mut result = Self {
            data_sender,
            stop_sender,
            thread: None,
            logical_size,
            screen_buffer_ptr: screen_buffer_ptr.clone(),
        };

        result.thread = Some(thread::spawn(move || {
            let mut state =
                AsyncConsoleRunner::new(data_recv, stop_recv, screen_buffer, screen_buffer_ptr);

            state.run(speaker);
        }));

        result
    }

    pub fn logical_size(&self) -> LogicalSize {
        self.logical_size
    }

    pub fn as_ptr(&self) -> *const u8 {
        self.screen_buffer_ptr
            .load(std::sync::atomic::Ordering::SeqCst) as *const u8
    }

    pub fn resume(&self) {
        if self.data_sender.send(ConsoleData::Resume).is_err() {
            warn!("Console resume send failed");
        }
    }

    pub fn pause(&self) {
        if self.data_sender.send(ConsoleData::Pause).is_err() {
            warn!("Console pause send failed");
        }
    }

    pub fn set_pad1(&self, data: Buttons) {
        if self.data_sender.send(ConsoleData::Pad1Data(data)).is_err() {
            warn!("Console pad1 data send failed");
        }
    }

    pub fn load(&self, data: Vec<u8>) {
        if self.data_sender.send(ConsoleData::Load(data)).is_err() {
            warn!("Console load send failed");
        }
    }

    pub fn reset(&self) {
        if self.data_sender.send(ConsoleData::Reset).is_err() {
            warn!("Console reset send failed");
        }
    }
}

impl Drop for AsyncConsole {
    fn drop(&mut self) {
        if self.stop_sender.send(()).is_err() {
            warn!("Console stop send failed");
        }
        mem::replace(&mut self.thread, None).map(JoinHandle::join);
    }
}

enum ConsoleData {
    Load(Vec<u8>),
    Resume,
    Pause,
    Reset,
    Pad1Data(Buttons),
}

struct AsyncConsoleRunner {
    timer: Timer,
    controller: StandardController,
    paused: bool,
    frame_counter: u64,

    stop_receiver: Receiver<()>,
    data_receiver: Receiver<ConsoleData>,
    screen_buffer: ScreenBuffer,
    screen_buffer_ptr: Arc<AtomicPtr<u8>>,
}

impl AsyncConsoleRunner {
    pub fn new(
        data_receiver: Receiver<ConsoleData>,
        stop_receiver: Receiver<()>,
        screen_buffer: ScreenBuffer,
        screen_buffer_ptr: Arc<AtomicPtr<u8>>,
    ) -> Self {
        Self {
            data_receiver,
            stop_receiver,

            timer: Timer::new(),
            controller: StandardController::new(),
            paused: true,
            frame_counter: 0,
            screen_buffer,
            screen_buffer_ptr,
        }
    }

    fn run<S: Sound + MixerInput>(&mut self, mut speaker: S) {
        let mut console: Option<Console> = None;
        while let Err(_) = self.stop_receiver.try_recv() {
            if let Some(console) = console.as_mut() {
                if !self.paused {
                    while !console.step(&mut self.screen_buffer, &mut self.controller, &mut speaker)
                    {
                    }
                    self.frame_counter += 1;
                    self.screen_buffer_ptr.store(
                        self.screen_buffer.as_mut_ptr(),
                        std::sync::atomic::Ordering::SeqCst,
                    );
                }
            }
            self.timer.wait();
            if let Ok(event) = self.data_receiver.try_recv() {
                match event {
                    ConsoleData::Load(data) => {
                        console = Console::new(&mut data.into_iter()).ok();
                    }
                    ConsoleData::Resume => {
                        self.paused = false;
                        speaker.start();
                    }
                    ConsoleData::Pause => {
                        self.paused = true;
                        speaker.pause();
                        let mut hasher = crc64::Digest::new(crc64::ECMA);
                        self.screen_buffer.hash(&mut hasher);
                        info!(
                            "Paused -- FrameCounter : {}, ScreenHash : 0x{:016X}",
                            self.frame_counter,
                            hasher.finish()
                        );
                    }
                    ConsoleData::Reset => {
                        console.as_mut().map(Console::reset).unwrap();
                    }
                    ConsoleData::Pad1Data(data) => {
                        self.controller.set_pad1(data);
                    }
                    // _ => (),
                }
            }
        }
    }
}
