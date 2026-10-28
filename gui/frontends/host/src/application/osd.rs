//! Application-owned notification, modal flight, presenter and raster state.

#[cfg(test)]
mod tests;

use std::{sync::mpsc::Receiver, time::Instant};

use nerust_render_traits::SurfaceSize;
use wakui::pause::PauseOwner;

use super::ApplicationHost;
use crate::{
    frame::FrameCtx,
    loop_driver::WakeNeed,
    osd::{ModalAdvance, ModalToken, OsdLayerSync, OsdNotifications, OsdTurn, OsdUpdate},
};

#[derive(Default)]
pub(super) struct OsdRuntime {
    notifications: OsdNotifications,
    layer: OsdLayerSync,
}

impl OsdRuntime {
    pub(super) fn close(&mut self) {
        self.notifications.close();
        self.layer.invalidate();
    }
}

impl ApplicationHost {
    /// Capture/pump/commit under the host borrow; native presentation happens afterwards.
    pub fn pump_osd(&mut self, ctx: &FrameCtx, now: Instant) -> OsdTurn {
        if self.close_exited() {
            return OsdTurn::default();
        }
        let report = self.osd.notifications.pump(&mut self.session, ctx, now);
        let pending = self.osd.notifications.modal_pending();
        if self.owns_pause(PauseOwner::Modal) != pending {
            self.set_pause_owner(PauseOwner::Modal, pending);
        }
        // Pump first: a completed worker must chain newly queued batches without
        // briefly releasing Modal and resuming a core behind the next dialog.
        let advance = self.osd.notifications.poll_modal_ack();
        let modal = match advance {
            Some(ModalAdvance::Present(presentation)) => Some(presentation),
            Some(advance) => {
                self.commit_modal_advance(&advance);
                None
            }
            None => self.osd.notifications.take_modal(),
        };
        let wake = WakeNeed {
            osd_deadline: self.osd_wakeup(now),
            flight_pending: self.osd_flight_pending(),
            ..Default::default()
        };
        if report.changed || wake.osd_deadline.is_some() {
            self.note_redraw();
        }
        OsdTurn {
            modal,
            wake,
            changed: report.changed,
        }
    }

    pub fn attach_osd_modal_receiver(&mut self, token: ModalToken, receiver: Receiver<()>) -> bool {
        !self.close_exited()
            && self
                .osd
                .notifications
                .attach_modal_receiver(token, receiver)
    }

    pub fn acknowledge_osd_modal(&mut self, token: ModalToken) -> ModalAdvance {
        if self.close_exited() {
            return ModalAdvance::Ignored;
        }
        let advance = self.osd.notifications.acknowledge_modal(token);
        self.commit_modal_advance(&advance);
        advance
    }

    /// Returns only owned output; no renderer or native callback is borrowed here.
    pub fn prepare_osd(
        &mut self,
        size: SurfaceSize,
        client_height: u32,
        ctx: &FrameCtx,
        now: Instant,
    ) -> OsdUpdate {
        if self.close_exited() {
            return OsdUpdate::default();
        }
        self.osd.layer.prepare(
            self.osd.notifications.manager(),
            size,
            client_height,
            ctx,
            now,
        )
    }

    pub fn invalidate_osd(&mut self) {
        if !self.close_exited() {
            self.osd.layer.invalidate();
        }
    }

    pub fn osd_wakeup(&self, now: Instant) -> Option<Instant> {
        self.osd.notifications.wakeup_deadline(now)
    }

    pub fn osd_flight_pending(&self) -> bool {
        self.osd.notifications.flight_pending()
    }

    pub fn osd_modal_pending(&self) -> bool {
        self.osd.notifications.modal_pending()
    }

    pub fn osd_pending(&self) -> usize {
        self.osd.notifications.pending()
    }

    pub fn osd_layer_live(&self) -> bool {
        self.osd.layer.layer_live()
    }

    pub fn osd_notifications(&self) -> &OsdNotifications {
        &self.osd.notifications
    }

    pub fn osd_layer(&self) -> &OsdLayerSync {
        &self.osd.layer
    }
}
