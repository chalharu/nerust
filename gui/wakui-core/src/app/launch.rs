//! Composition bundle: the app plus its launch arguments.
//!
//! The composition root builds an [`Application`] (already-constructed
//! policy owner plus file operands) and hands it to a backend's `run`.
//! Launch-argument interpretation lives here; policy lives in the
//! [`crate::app::roles`] ports and the [`crate::app::boundary::FrontendApp`]
//! facade.

use crate::app::boundary::FrontendApp;

/// Built bundle handed from the composition root to a frontend. The app is
/// already constructed (content handle, factory, loader); frontend entry points
/// receive only this struct. `K` is the view-side aux kind, `A` the
/// launch-arguments type.
///
/// Build it with [`Application::builder`]: launch arguments travel inside
/// the bundle, so no backend reads the process argv on its own. The
/// framework never interprets them; each backend just applies its platform
/// file-open convention through [`LaunchArguments`] (tao opens the first
/// file at startup, gtk routes them through the toolkit open events). The
/// bundle is consumed once by the backend's `run`.
///
/// `A` defaults to `()` (no arguments); apps with richer CLIs define their
/// own arguments struct. The builder starts untyped (`()`) and each
/// `.arguments()` call swaps to a differently-typed builder (typestate, no
/// `Default` bound anywhere); the [`LaunchArguments`] bound is checked at
/// the backend's `run`, so the wrong bundle never compiles.
pub struct Application<K: Copy + Eq + std::hash::Hash, A = ()> {
    app: Box<dyn FrontendApp<K>>,
    arguments: A,
}

/// Launch arguments carried in the bundle. Backends read the file operands
/// through this trait; the framework never interprets them. Flags were
/// already consumed upstream, so only file operands travel here.
pub trait LaunchArguments {
    /// File operands to open at startup.
    fn files(&self) -> Vec<String>;
}

impl LaunchArguments for Vec<String> {
    fn files(&self) -> Vec<String> {
        self.clone()
    }
}

impl LaunchArguments for () {
    fn files(&self) -> Vec<String> {
        Vec::new()
    }
}

/// Builder for [`Application`]: the single place launch arguments enter the
/// bundle. Chainable; `build()` hands the bundle to a backend's `run`.
///
/// Typestate: starts as `ApplicationBuilder<K, ()>`; each `.arguments()`
/// call swaps to a differently-typed builder. No `Default` bound is needed
/// at any step.
pub struct ApplicationBuilder<K: Copy + Eq + std::hash::Hash, A = ()> {
    app: Box<dyn FrontendApp<K>>,
    arguments: A,
}

impl<K: Copy + Eq + std::hash::Hash> Application<K, ()> {
    /// Start building over an already-constructed app, with no arguments.
    pub fn builder(app: Box<dyn FrontendApp<K>>) -> ApplicationBuilder<K, ()> {
        ApplicationBuilder::new(app)
    }

    /// Bundle an app with no launch arguments.
    pub fn new(app: Box<dyn FrontendApp<K>>) -> Self {
        Self { app, arguments: () }
    }
}

impl<K: Copy + Eq + std::hash::Hash, A> Application<K, A> {
    /// Borrow the policy owner back (e.g. for pre-run probes in tests).
    pub fn app_mut(&mut self) -> &mut dyn FrontendApp<K> {
        &mut *self.app
    }

    /// Release the policy owner; the backend calls this once at startup.
    pub fn into_app(self) -> Box<dyn FrontendApp<K>> {
        self.app
    }

    /// Opaque launch arguments. The framework never interprets them;
    /// backends read them through [`LaunchArguments`].
    pub fn arguments(&self) -> &A {
        &self.arguments
    }
}

impl<K: Copy + Eq + std::hash::Hash> ApplicationBuilder<K, ()> {
    /// Start building over an already-constructed app, with no arguments.
    pub fn new(app: Box<dyn FrontendApp<K>>) -> Self {
        Self { app, arguments: () }
    }

    /// Append one launch argument, swapping to `Vec<String>` arguments.
    pub fn argument(self, argument: impl Into<String>) -> ApplicationBuilder<K, Vec<String>> {
        ApplicationBuilder {
            app: self.app,
            arguments: vec![argument.into()],
        }
    }
}

impl<K: Copy + Eq + std::hash::Hash, A> ApplicationBuilder<K, A> {
    /// Set launch arguments, swapping to a differently-typed builder. The
    /// type initializes here (or replaces a previous value).
    pub fn arguments<B>(self, arguments: B) -> ApplicationBuilder<K, B> {
        ApplicationBuilder {
            app: self.app,
            arguments,
        }
    }

    /// Hand the bundle to a backend's `run`.
    pub fn build(self) -> Application<K, A> {
        Application {
            app: self.app,
            arguments: self.arguments,
        }
    }
}

impl<K: Copy + Eq + std::hash::Hash> ApplicationBuilder<K, Vec<String>> {
    /// Append one launch argument (file operand).
    pub fn argument(mut self, argument: impl Into<String>) -> Self {
        self.arguments.push(argument.into());
        self
    }
}
