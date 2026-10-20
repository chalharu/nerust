//! Headless window opener: fails closed while recording requests.
//! Production code never touches this module.

use std::sync::{Arc, Mutex};

/// Recorded window-creation request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OpenerCall {
    Main,
    Settings,
    Debug,
    Ppu,
    History,
}

/// `WindowOpener` that fails every creation closed and records calls.
/// Aux handles are unbuildable headless (they embed `TaoWindow`),
/// so only the failure/skip paths are reachable — the honest maximum.
#[derive(Debug, Default)]
pub(crate) struct FakeOpener {
    calls: Mutex<Vec<OpenerCall>>,
    main_detail: Mutex<String>,
}

impl FakeOpener {
    pub(crate) fn failing(detail: &str) -> Self {
        let opener = Self::default();
        *crate::aux::bridge::lock_recovered(&opener.main_detail) = detail.to_string();
        opener
    }

    pub(crate) fn calls(&self) -> Vec<OpenerCall> {
        crate::aux::bridge::lock_recovered(&self.calls).clone()
    }
}

impl crate::ports::WindowOpener for FakeOpener {
    fn create_main(
        &self,
        _builder: tao::window::WindowBuilder,
    ) -> Result<Arc<tao::window::Window>, String> {
        crate::aux::bridge::lock_recovered(&self.calls).push(OpenerCall::Main);
        Err(crate::aux::bridge::lock_recovered(&self.main_detail).clone())
    }

    fn create_aux(
        &self,
        kind: crate::aux::focus::AuxKind,
        _params: crate::ports::AuxOpenParams,
    ) -> Option<Box<dyn crate::aux::registry::AuxController>> {
        use crate::aux::focus::AuxKind;
        let call = match kind {
            AuxKind::Settings => OpenerCall::Settings,
            AuxKind::Debug => OpenerCall::Debug,
            AuxKind::Ppu => OpenerCall::Ppu,
            AuxKind::History => OpenerCall::History,
        };
        crate::aux::bridge::lock_recovered(&self.calls).push(call);
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aux::focus::AuxKind;
    use crate::ports::{AuxOpenParams, WindowOpener};
    use nerust_settings_core::model::language::AppLanguage;

    #[test]
    fn fake_opener_records_each_aux_kind() {
        let opener = FakeOpener::failing("no display");
        assert!(
            opener
                .create_aux(
                    AuxKind::History,
                    AuxOpenParams::History {
                        language: AppLanguage::English
                    }
                )
                .is_none()
        );
        assert!(
            opener
                .create_aux(
                    AuxKind::Debug,
                    AuxOpenParams::Debug {
                        display: Default::default(),
                        spaces: Vec::new(),
                        mem_addr: 0,
                        language: AppLanguage::English,
                        position: None,
                    }
                )
                .is_none()
        );
        assert_eq!(opener.calls(), [OpenerCall::History, OpenerCall::Debug]);
    }

    #[test]
    fn fake_main_records_and_fails_closed() {
        use crate::ports::WindowOpener as _;
        let opener = FakeOpener::failing("no display");
        let builder = tao::window::WindowBuilder::new().with_title("test");
        assert!(opener.create_main(builder).is_err());
        assert_eq!(opener.calls(), [OpenerCall::Main]);
    }
}
