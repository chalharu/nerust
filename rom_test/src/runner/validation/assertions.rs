#[derive(Clone, Copy)]
pub(in crate::runner::validation) struct CartridgeRamAssertion {
    pub(in crate::runner::validation) frame: u64,
    pub(in crate::runner::validation) address: usize,
    pub(in crate::runner::validation) expected_value: u8,
    pub(in crate::runner::validation) expect_open_bus: bool,
}
