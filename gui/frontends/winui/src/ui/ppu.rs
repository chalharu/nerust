//! PPU viewer: live pattern-table images plus observation panels.
//!
//! A 500ms-throttled Refresh drain with `need_images` feeds both the
//! pictures and the formatted panels, mirroring the tao twin's sync.
//! Hovering an image shows the tile readout (8x8 tiles, 2x scale).

use std::time::Instant;

use nerust_frontend_host::debugger::{DebuggerEvent, DebuggerModel};
use nerust_gui_shell::debug::{DebugAction, DebugDrainInput, DrainFlight, DrainPoll};
use windows_core::Result;

use crate::bindings::Microsoft::UI::Xaml::Controls::{Image, Orientation, StackPanel, TextBlock};
use crate::bindings::Microsoft::UI::Xaml::Media::Imaging::SoftwareBitmapSource;
use crate::bindings::Microsoft::UI::Xaml::Window;
use crate::bindings::Windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use crate::bindings::Windows::Storage::Streams::DataWriter;

use super::h;

pub struct PpuWin {
    pub window: Window,
    model: DebuggerModel,
    flight: Option<DrainFlight>,
    last_poll: Instant,
    image_box: StackPanel,
    hover: TextBlock,
    panels: TextBlock,
    sources: Vec<SoftwareBitmapSource>,
}

impl PpuWin {
    pub fn open(host: &mut nerust_win::WinHost) -> Option<Self> {
        let flow = (|| -> Result<Self> {
            let window = Window::new()?;
            window.SetTitle(&h("PPU viewer"))?;
            let hover = TextBlock::new()?;
            let image_box = StackPanel::new()?;
            image_box.SetOrientation(Orientation::Vertical)?;
            let panels = TextBlock::new()?;
            panels.SetTextWrapping(crate::bindings::Microsoft::UI::Xaml::TextWrapping::Wrap)?;
            let stack = StackPanel::new()?;
            stack.SetOrientation(Orientation::Vertical)?;
            stack.Children()?.Append(&hover)?;
            stack.Children()?.Append(&image_box)?;
            stack.Children()?.Append(&panels)?;
            window.SetContent(&stack)?;
            window.Activate()?;
            Ok(Self {
                window,
                model: DebuggerModel::from_session(host.session()),
                flight: None,
                last_poll: Instant::now()
                    .checked_sub(std::time::Duration::from_secs(1))
                    .unwrap_or_else(Instant::now),
                image_box,
                hover,
                panels,
                sources: Vec::new(),
            })
        })();
        match flow {
            Ok(win) => Some(win),
            Err(error) => {
                log::error!("winui ppu open failed: {error}");
                None
            }
        }
    }

    /// Poll the drain flight; called from the main tick.
    pub fn poll(&mut self, host: &mut nerust_win::WinHost) {
        if let Some(flight) = self.flight.as_mut() {
            match flight.poll() {
                DrainPoll::Pending => {}
                DrainPoll::Done(output) => {
                    self.model.apply_drain(output.display, output.reselected);
                    self.rebuild();
                    self.flight = None;
                }
            }
            return;
        }
        if self.last_poll.elapsed().as_millis() < 500 {
            return;
        }
        self.last_poll = Instant::now();
        let _ = self.model.dispatch(DebuggerEvent::RefreshPressed);
        let input = DebugDrainInput {
            space: self.model.selected_space(),
            mem_addr: self.model.anchor(),
            dis_addr: self.model.nav.dis_addr(),
            watch: None,
            freeze: None,
            reg_target: None,
            prev_dump_rows: &[],
            need_images: true,
        };
        let prev = self.model.display.dump_rows.clone();
        let input = DebugDrainInput {
            prev_dump_rows: &prev,
            ..input
        };
        let _ = &prev;
        self.flight = Some(
            host.session()
                .debug_drain_begin(&[DebugAction::Refresh], &input),
        );
    }

    fn rebuild(&mut self) {
        let display = &self.model.display;
        if self
            .image_box
            .Children()
            .and_then(|items| items.Clear())
            .is_ok()
        {
            self.sources.clear();
            for image in &display.images {
                let picture = (|| -> Result<()> {
                    let writer = DataWriter::new()?;
                    writer.WriteBytes(&image.rgba)?;
                    let buffer = writer.DetachBuffer()?;
                    let bitmap = SoftwareBitmap::CreateCopyFromBuffer(
                        &buffer,
                        BitmapPixelFormat::Bgra8,
                        image.width as i32,
                        image.height as i32,
                    )?;
                    let source = SoftwareBitmapSource::new()?;
                    source.SetBitmapAsync(&bitmap)?;
                    let view = Image::new()?;
                    view.SetSource(&source)?;
                    let label = TextBlock::new()?;
                    label.SetText(&h(&image.label))?;
                    let (iw, ih) = (image.width, image.height);
                    let hover = self.hover.clone();
                    let info = format!("{} ({}x{})", image.label, iw, ih);
                    view.Tapped(
                        &crate::bindings::Microsoft::UI::Xaml::Input::TappedEventHandler::new(
                            move |_, _| {
                                let _ = hover.SetText(&h(&info));
                                Ok(())
                            },
                        ),
                    )?;
                    self.image_box.Children()?.Append(&label)?;
                    self.image_box.Children()?.Append(&view)?;
                    self.sources.push(source);
                    Ok(())
                })();
                if let Err(error) = picture {
                    log::warn!("winui ppu picture failed: {error}");
                }
            }
        }
        let text = nerust_gui_viewmodel::debugger::format_panels(&display.panels);
        let _ = self.panels.SetText(&h(&text));
        let _ = self.window.SetTitle(&h("PPU viewer"));
    }
}
