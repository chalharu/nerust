//! Framework app boundary: the single policy owner as a trait.
//!
//! View definitions implement [`FrontendApp`] over the session; native
//! frontends hold it behind [`crate::app::launch::Application`]
//! and never name app, shell, or ViewModel types. Every method is object-safe
//! so the composition root can inject the concrete app as a trait object.
//!
//! The facade composes the narrow [`crate::app::roles`] ports (one trait
//! per concern); each role is blanket-implemented for every `FrontendApp`,
//! so implementors write a single `impl FrontendApp` while consumers bound
//! on the port they need.

use crate::{
    app::roles::{
        AuxPorts, ChromePorts, ContentPorts, DialogPorts, FocusPorts, LifecyclePorts, PollPorts,
        ViewportPorts,
    },
    chrome::window::WindowEffects,
    input::event::EventHandler,
};

pub use crate::app::launch::{Application, ApplicationBuilder, LaunchArguments};

/// Opaque close outcome: the native handle leaves as type-erased storage
/// so frontends downcast on their side of the boundary.
pub struct WidgetCloseOutcome {
    pub native: Option<Box<dyn std::any::Any>>,
    pub effects: WindowEffects,
}

/// The single policy owner, behind a trait object. Native adapters drive
/// one [`Application::into_app`] through these narrow ports; all ViewModel contact
/// stays inside the implementation. `K` is the view-side aux kind
/// (view-defined kinds); the framework never names it.
///
/// File and content ports are generic (open/unload content, file dialogs).
/// Suspension arbitration lives view-side; the framework only transports
/// the underlying notifications (focus events, modal effects, commands).
/// Content verbs and aux kinds live view-side behind separate domain
/// ports; flavors never name them.
///
/// This trait declares no methods itself: every port comes from a
/// [`crate::app::roles`] trait. Read the role whose concern you need;
/// implementors still write one `impl FrontendApp` block.
pub trait FrontendApp<K: Copy + Eq + std::hash::Hash>:
    EventHandler<K>
    + LifecyclePorts
    + ContentPorts
    + ChromePorts
    + FocusPorts
    + AuxPorts<K>
    + PollPorts
    + DialogPorts
    + ViewportPorts
{
}
