use crate::metadata::{STATE_ARCHIVE_SCHEMA_VERSION, StateArchiveMetadata};
use crate::state_slot_path;
use crate::time::unix_millis;
use nerust_contract::{CoreOptions, PersistenceTarget, RomIdentity};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::time::SystemTime;

const STATE_ARCHIVE_FIXTURE_HEX: &str = "504b0304140000000800000021004ce2ae73e30000006a010000100000006d657461646174612e6d73677061636b55904d4ec3400c851d7eaec09a3b80b88ee54ca664443c133c9ea8ddb1e720958802597181b245bd013beec124226abafc9e3fd94ffe819bf7686acb849d95e8822ff6b1098aaeba1e2375b642524cde6d91e311a0783dfed630d41451ebc4a527d71c7aa6b6b582ba6bedd5474c25ae02185a7944098c46ccc37d31985a4e7831a890f3d99df172643677e8e4193b12475e61b49c1ad2204bc1fdc66d35897d9b966c823065899d48563854169f9cafe0739d981435cb4dd22fe8a7ee25a95ad91dfae57a633df44bd10cdf2fd02f4527bcfd9fd28c30bf0657c924ccfe99b04efe00504b030414000000080000002100def67eb017000000150000000900000073746174652e62696e4bcbac28292d4ad5cd4d4ccec8cc4bd52d2e492c490500504b030414000000080000002100f860c12e0b000000090000000d0000007468756d626e61696c2e706e67eb0cf073e7e592e2fa0f00504b01021403140000000800000021004ce2ae73e30000006a010000100000000000000000000000a481000000006d657461646174612e6d73677061636b504b0102140314000000080000002100def67eb01700000015000000090000000000000000000000a4811101000073746174652e62696e504b0102140314000000080000002100f860c12e0b000000090000000d0000000000000000000000a4814f0100007468756d626e61696c2e706e67504b05060000000003000300b0000000850100000000";

fn fixture_bytes(hex: &str) -> Vec<u8> {
    assert!(
        !hex.trim().is_empty(),
        "fixture hex must be populated before running persistence tests"
    );
    let hex = hex.trim();
    assert_eq!(hex.len() % 2, 0, "fixture hex length must be even");
    hex.as_bytes()
        .chunks_exact(2)
        .map(|chunk| {
            let text = std::str::from_utf8(chunk).expect("fixture hex should be valid utf-8");
            u8::from_str_radix(text, 16).expect("fixture hex should decode")
        })
        .collect()
}

pub(super) fn prepare_test_dir(name: &str) -> PathBuf {
    let dir = test_dir(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

pub(super) fn write_fixture_archive(name: &str) -> PathBuf {
    let dir = prepare_test_dir(name);
    let path = state_slot_path(&dir, 99);
    fs::write(&path, fixture_bytes(STATE_ARCHIVE_FIXTURE_HEX)).unwrap();
    path
}

pub(super) fn test_target() -> PersistenceTarget {
    PersistenceTarget {
        rom_identity: test_rom_identity(),
        options: CoreOptions::default(),
    }
}

pub(super) fn test_rom_identity() -> RomIdentity {
    RomIdentity {
        format: nerust_contract::RomFormat::INes,
        mapper_type: 4,
        sub_mapper_type: 0,
        mirror_mode: nerust_contract::MirrorMode::Horizontal,
        has_battery: true,
        trainer_len: 0,
        prg_rom_len: 0x8000,
        chr_rom_len: 0x2000,
        prg_ram_len: 0,
        save_prg_ram_len: 0x2000,
        chr_ram_len: 0,
        save_chr_ram_len: 0,
        prg_rom_crc64: 1,
        chr_rom_crc64: 2,
        trainer_crc64: 3,
    }
}

pub(super) fn test_metadata(slot_id: u64, has_thumbnail: bool) -> StateArchiveMetadata {
    StateArchiveMetadata {
        schema_version: STATE_ARCHIVE_SCHEMA_VERSION,
        slot_id,
        saved_at_unix_ms: unix_millis(SystemTime::now()).unwrap(),
        has_thumbnail,
        mapper_type: 4,
        sub_mapper_type: 0,
        prg_rom_crc64: 1,
        chr_rom_crc64: 2,
        trainer_crc64: 3,
        mmc3_irq_variant: 0,
        emulator_version: "test".into(),
        rom_format: 0,
        mirror_mode_kind: 0,
        mirror_mode_custom_lut: Vec::new(),
        has_battery: false,
        trainer_len: 0,
        prg_rom_len: 0,
        chr_rom_len: 0,
        prg_ram_len: 0,
        save_prg_ram_len: 0,
        chr_ram_len: 0,
        save_chr_ram_len: 0,
    }
}

fn test_dir(name: &str) -> PathBuf {
    env::current_dir()
        .unwrap()
        .join("target")
        .join("persistence-tests")
        .join(name)
}
