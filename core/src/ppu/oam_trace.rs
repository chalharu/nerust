use std::env;
use std::ffi::OsStr;
use std::fmt::Write as _;
use std::sync::OnceLock;

const TRACE_ENV_VAR: &str = "NERUST_OAM_TRACE";
static TRACE_ENABLED: OnceLock<bool> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SpriteTraceEntry {
    index: u8,
    oam: [u8; 4],
}

#[derive(Debug, PartialEq, Eq)]
struct OamTraceSnapshot {
    frame: usize,
    scanline: u16,
    sprite_height: u8,
    evaluation_start_oam_address: u8,
    secondary_count: u8,
    status_sprite_overflow: bool,
    in_range: Vec<SpriteTraceEntry>,
    ideal_selected: Vec<SpriteTraceEntry>,
    ideal_dropped: Vec<SpriteTraceEntry>,
    secondary_oam: [[u8; 4]; 8],
}

pub(super) fn emit_scanline_trace(
    frame: usize,
    scanline: u16,
    sprite_height: u8,
    evaluation_start_oam_address: u8,
    status_sprite_overflow: bool,
    sprite_count: u8,
    primary_oam: &[u8; 256],
    secondary_oam: &[u8; 32],
) {
    if !trace_enabled() {
        return;
    }

    let snapshot = collect_scanline_snapshot(
        frame,
        scanline,
        sprite_height,
        evaluation_start_oam_address,
        status_sprite_overflow,
        sprite_count,
        primary_oam,
        secondary_oam,
    );
    eprintln!("{}", format_snapshot(&snapshot));
}

fn trace_enabled() -> bool {
    *TRACE_ENABLED.get_or_init(|| parse_trace_env(env::var_os(TRACE_ENV_VAR).as_deref()))
}

fn parse_trace_env(value: Option<&OsStr>) -> bool {
    matches!(value, Some(value) if value == OsStr::new("1"))
}

fn collect_scanline_snapshot(
    frame: usize,
    scanline: u16,
    sprite_height: u8,
    evaluation_start_oam_address: u8,
    status_sprite_overflow: bool,
    sprite_count: u8,
    primary_oam: &[u8; 256],
    secondary_oam: &[u8; 32],
) -> OamTraceSnapshot {
    let mut in_range = Vec::new();
    for (index, sprite) in primary_oam.chunks_exact(4).enumerate() {
        if sprite_in_range(scanline, sprite_height, sprite[0]) {
            in_range.push(SpriteTraceEntry {
                index: index as u8,
                oam: [sprite[0], sprite[1], sprite[2], sprite[3]],
            });
        }
    }

    let mut ideal_order_in_range = Vec::new();
    let start_sprite_index = usize::from(evaluation_start_oam_address >> 2);
    for offset in 0..64 {
        let index = (start_sprite_index + offset) & 0x3F;
        let sprite = &primary_oam[index * 4..index * 4 + 4];
        if sprite_in_range(scanline, sprite_height, sprite[0]) {
            ideal_order_in_range.push(SpriteTraceEntry {
                index: index as u8,
                oam: [sprite[0], sprite[1], sprite[2], sprite[3]],
            });
        }
    }

    let selected_len = 8.min(ideal_order_in_range.len());
    let ideal_selected = ideal_order_in_range[..selected_len].to_vec();
    let ideal_dropped = ideal_order_in_range[selected_len..].to_vec();

    let mut secondary_slots = [[0; 4]; 8];
    for (slot, bytes) in secondary_oam.chunks_exact(4).enumerate() {
        secondary_slots[slot].copy_from_slice(bytes);
    }

    OamTraceSnapshot {
        frame,
        scanline,
        sprite_height,
        evaluation_start_oam_address,
        secondary_count: sprite_count.min(8),
        status_sprite_overflow,
        in_range,
        ideal_selected,
        ideal_dropped,
        secondary_oam: secondary_slots,
    }
}

fn sprite_in_range(scanline: u16, sprite_height: u8, y: u8) -> bool {
    let y = u16::from(y);
    scanline > y && scanline <= y + u16::from(sprite_height)
}

fn format_snapshot(snapshot: &OamTraceSnapshot) -> String {
    let mut line = String::with_capacity(512);
    let _ = write!(
        line,
        "{{\"trace\":\"oam\",\"frame\":{},\"scanline\":{},\"sprite_height\":{},\"evaluation_start_oam_address\":{},\"secondary_count\":{},\"status_sprite_overflow\":{},\"in_range\":",
        snapshot.frame,
        snapshot.scanline,
        snapshot.sprite_height,
        snapshot.evaluation_start_oam_address,
        snapshot.secondary_count,
        snapshot.status_sprite_overflow
    );
    push_entries(&mut line, &snapshot.in_range);
    line.push_str(",\"ideal_selected\":");
    push_entries(&mut line, &snapshot.ideal_selected);
    line.push_str(",\"ideal_dropped\":");
    push_entries(&mut line, &snapshot.ideal_dropped);
    line.push_str(",\"secondary_oam\":");
    push_secondary_oam(&mut line, &snapshot.secondary_oam);
    line.push('}');
    line
}

fn push_entries(line: &mut String, entries: &[SpriteTraceEntry]) {
    line.push('[');
    for (index, entry) in entries.iter().enumerate() {
        if index > 0 {
            line.push(',');
        }
        let _ = write!(
            line,
            "{{\"index\":{},\"oam\":[{},{},{},{}]}}",
            entry.index, entry.oam[0], entry.oam[1], entry.oam[2], entry.oam[3]
        );
    }
    line.push(']');
}

fn push_secondary_oam(line: &mut String, secondary_oam: &[[u8; 4]; 8]) {
    line.push('[');
    for (slot, bytes) in secondary_oam.iter().enumerate() {
        if slot > 0 {
            line.push(',');
        }
        let _ = write!(
            line,
            "[{},{},{},{}]",
            bytes[0], bytes[1], bytes[2], bytes[3]
        );
    }
    line.push(']');
}

#[cfg(test)]
mod tests {
    use super::{
        OamTraceSnapshot, SpriteTraceEntry, collect_scanline_snapshot, format_snapshot,
        parse_trace_env,
    };
    use std::ffi::OsStr;

    #[test]
    fn parse_trace_env_requires_explicit_one() {
        assert!(!parse_trace_env(None));
        assert!(!parse_trace_env(Some(OsStr::new("0"))));
        assert!(!parse_trace_env(Some(OsStr::new("true"))));
        assert!(parse_trace_env(Some(OsStr::new("1"))));
    }

    #[test]
    fn collect_snapshot_reports_selected_and_dropped_sprites() {
        let mut primary_oam = [0xFF; 256];
        for index in 0..10u8 {
            let base = usize::from(index) * 4;
            primary_oam[base] = 20;
            primary_oam[base + 1] = 0x40 + index;
            primary_oam[base + 2] = index;
            primary_oam[base + 3] = 0x80 + index;
        }

        let mut secondary_oam = [0xFF; 32];
        secondary_oam.copy_from_slice(&primary_oam[..32]);

        let snapshot =
            collect_scanline_snapshot(0, 21, 8, 0x20, true, 8, &primary_oam, &secondary_oam);

        assert_eq!(snapshot.frame, 0);
        assert_eq!(snapshot.scanline, 21);
        assert_eq!(snapshot.sprite_height, 8);
        assert_eq!(snapshot.evaluation_start_oam_address, 0x20);
        assert_eq!(snapshot.secondary_count, 8);
        assert!(snapshot.status_sprite_overflow);
        assert_eq!(snapshot.in_range.len(), 10);
        assert_eq!(snapshot.ideal_selected.len(), 8);
        assert_eq!(snapshot.ideal_dropped.len(), 2);
        assert_eq!(snapshot.ideal_selected[0].index, 8);
        assert_eq!(snapshot.ideal_selected[1].index, 9);
        assert_eq!(snapshot.ideal_selected[7].index, 5);
        assert_eq!(snapshot.ideal_dropped[0].index, 6);
        assert_eq!(snapshot.ideal_dropped[1].index, 7);
        assert_eq!(snapshot.secondary_oam[0], [20, 0x40, 0, 0x80]);
        assert_eq!(snapshot.secondary_oam[7], [20, 0x47, 7, 0x87]);
    }

    #[test]
    fn format_snapshot_is_machine_friendly() {
        let snapshot = OamTraceSnapshot {
            frame: 7,
            scanline: 42,
            sprite_height: 8,
            evaluation_start_oam_address: 4,
            secondary_count: 1,
            status_sprite_overflow: false,
            in_range: vec![SpriteTraceEntry {
                index: 3,
                oam: [41, 12, 2, 88],
            }],
            ideal_selected: vec![SpriteTraceEntry {
                index: 3,
                oam: [41, 12, 2, 88],
            }],
            ideal_dropped: Vec::new(),
            secondary_oam: [
                [41, 12, 2, 88],
                [255, 255, 255, 255],
                [255, 255, 255, 255],
                [255, 255, 255, 255],
                [255, 255, 255, 255],
                [255, 255, 255, 255],
                [255, 255, 255, 255],
                [255, 255, 255, 255],
            ],
        };

        assert_eq!(
            format_snapshot(&snapshot),
            "{\"trace\":\"oam\",\"frame\":7,\"scanline\":42,\"sprite_height\":8,\"evaluation_start_oam_address\":4,\"secondary_count\":1,\"status_sprite_overflow\":false,\"in_range\":[{\"index\":3,\"oam\":[41,12,2,88]}],\"ideal_selected\":[{\"index\":3,\"oam\":[41,12,2,88]}],\"ideal_dropped\":[],\"secondary_oam\":[[41,12,2,88],[255,255,255,255],[255,255,255,255],[255,255,255,255],[255,255,255,255],[255,255,255,255],[255,255,255,255],[255,255,255,255]]}"
        );
    }
}
