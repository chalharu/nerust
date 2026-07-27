use std::{
    fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use nerust_core_traits::CoreOptions;
use nerust_core_traits::factory::load::SystemLoadOptions;
use nerust_input_traits::AttachmentId;

mod core_mock;
mod factory_mock;
mod input_mock;

pub(crate) use crate::test_support::{DummyOtherSystemId, DummySystemId};
pub(crate) use core_mock::*;
pub(crate) use factory_mock::*;
pub(crate) use input_mock::*;

pub(crate) static CORE_CREATION_COUNT: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

pub(crate) const TEST_SLOT_P1: AttachmentId = AttachmentId::new("test.slot.p1");

#[derive(
    Default, Debug, Clone, PartialEq, Eq, clap::Args, serde::Serialize, serde::Deserialize,
)]
pub(crate) struct NoopSystemLoadOptions;
impl SystemLoadOptions for NoopSystemLoadOptions {}

#[derive(Default, Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct NoopCoreOptions;
impl CoreOptions for NoopCoreOptions {}

pub(crate) fn test_rom() -> Vec<u8> {
    let mut data = vec![0x4E, 0x45, 0x53, 0x1A, 2u8, 1, 0, 0];
    data.resize(16 + 0x8000 + 0x2000, 0);
    data
}

pub(crate) fn test_rom_with_mapper4() -> Vec<u8> {
    let mut data = vec![0x4E, 0x45, 0x53, 0x1A, 2u8, 1, 0x40, 0];
    data.resize(16 + 0x8000 + 0x2000, 0);
    data
}

pub(crate) fn unique_temp_dir(label: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("current time should be after unix epoch")
        .as_nanos();
    let path = std::env::temp_dir().join(format!("nerust-{label}-{}-{nonce}", std::process::id()));
    fs::create_dir_all(&path).expect("temp dir should create");
    path
}
