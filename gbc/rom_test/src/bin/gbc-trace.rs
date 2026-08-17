//! Temporary debug tracer for investigating ROM test failures.
//! Usage: gbc-trace <rom-path> <model> <cycles> [trace-filter]
//!
//! Prints a per-M-cycle trace of PC + register state, plus PPU events
//! (mode changes, LY transitions, STAT/IF writes).

use std::path::PathBuf;

use nerust_gbc_core::{
    cartridge::Cartridge,
    cartridge_header::CartridgeHeader,
    cpu_core::{GbcModel, Lr35902Cpu},
    memory::GbcMemoryBus,
};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let rom_path = PathBuf::from(&args[1]);
    let model = args.get(2).map(String::as_str).unwrap_or("dmg");
    let cycles: usize = args.get(3).and_then(|s| s.parse().ok()).unwrap_or(400000);
    let filter = args.get(4).cloned().unwrap_or_default();

    let rom_bytes = std::fs::read(&rom_path).unwrap();
    let header = CartridgeHeader::parse(&rom_bytes).unwrap();
    let mbc = nerust_gbc_core::cartridge_mbc::create_mbc(&header, rom_bytes, None);

    let mut bus = GbcMemoryBus::new([0; 0x100], false);
    bus.set_cartridge(Cartridge::new(mbc));
    let hw_is_cgb = matches!(model, "cgb_c" | "cgb_d" | "agb");
    bus.set_cgb_mode(hw_is_cgb);
    bus.set_cgb_revision_d(matches!(model, "cgb_d" | "agb"));
    bus.set_cgb_game(hw_is_cgb && header.cgb_flag & 0x80 != 0);
    match model {
        "dmg0" => bus.set_boot_counter(0x182F),
        "dmg" => bus.set_boot_counter(0xABCB),
        _ => bus.set_boot_counter(0x2677),
    }
    bus.set_post_boot_io(hw_is_cgb);
    let mut cpu = match model {
        "dmg0" => Lr35902Cpu::with_model(GbcModel::Dmg0),
        "dmg" => Lr35902Cpu::with_model(GbcModel::Dmg),
        "cgb_c" | "cgb_d" => Lr35902Cpu::with_model(GbcModel::Cgb),
        _ => Lr35902Cpu::with_model(GbcModel::Agb),
    };
    cpu.registers_mut().set_pc(0x0100);

    let mut last_ly = 0xFFu8;
    let mut last_stat = 0xFFu8;
    let mut last_if = 0xFFu8;
    for mcycle in 0..cycles {
        let ds_before = bus.is_double_speed();
        for _ in 0..4 {
            bus.step_tcycle(&mut cpu);
        }
        if bus.is_double_speed() != ds_before {
            eprintln!("M{} SPEED SWITCH -> double={}", mcycle, bus.is_double_speed());
        }
        if std::env::var("TRACE_ADDR").is_ok() {
            let pc = cpu.registers().pc();
            if (0xC000..=0xC200).contains(&pc) || (0x4080..=0x40b0).contains(&pc) {
                eprintln!(
                    "M{} pc={:04x} ds={} tima={:02x} if={:02x} ime={}",
                    mcycle, pc, bus.is_double_speed() as u8, bus.read(0xFF05), bus.read(0xFF0F), bus.ime_enabled()
                );
            }
        }
        if std::env::var("TRACE_RANGE").is_ok() {
            let pc = cpu.registers().pc();
            if (0x150..=0x400).contains(&pc) || pc == 0x48 {
                eprintln!(
                    "M{} pc={:04x} ly={} clock_stat={:02x} if={:02x}",
                    mcycle, pc, bus.read(0xFF44), bus.read(0xFF41), bus.read(0xFF0F)
                );
            }
        }
        let ly = bus.read(0xFF44);
        let stat = bus.read(0xFF41);
        let if_ = bus.read(0xFF0F);
        let pc = cpu.registers().pc();
        if filter.contains("events") {
            if ly != last_ly || (stat & 0x47) != (last_stat & 0x47) || if_ != last_if {
                eprintln!(
                    "M{:>7} ly={:>3} stat={:02x} if={:02x} pc={:04x} b={:02x}",
                    mcycle,
                    ly,
                    stat,
                    if_,
                    pc,
                    cpu.registers().b()
                );
            }
        }
        if filter.contains("pc") {
            eprintln!(
                "M{:>7} pc={:04x} a={:02x} b={:02x} c={:02x} d={:02x} e={:02x} hl={:04x} sp={:04x} ly={} stat={:02x} if={:02x} ie={:02x} ime={}",
                mcycle,
                pc,
                cpu.registers().a(),
                cpu.registers().b(),
                cpu.registers().c(),
                cpu.registers().d(),
                cpu.registers().e(),
                cpu.registers().hl(),
                cpu.registers().sp(),
                ly,
                stat,
                if_,
                bus.read_ie(),
                bus.ime_enabled()
            );
        }
        last_ly = ly;
        last_stat = stat;
        last_if = if_;
    }
    let out = bus.serial_output();
    eprintln!("SERIAL: {}", String::from_utf8_lossy(out));
    eprintln!("SERIAL HEX: {}", out.iter().map(|b| format!("{:02X}", b)).collect::<String>());

    // Dump mooneye lcdon_timing-style pass results (HRAM buffers) when asked.
    if let Ok(base) = std::env::var("DUMP_HRAM") {
        let base = u16::from_str_radix(base.trim_start_matches("0x"), 16).unwrap();
        let mut bytes = Vec::new();
        for i in 0..24u16 {
            bytes.push(bus.read(0xFF80 + base + i));
        }
        eprintln!(
            "HRAM[{:04x}]: {}",
            base,
            bytes.iter().map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ")
        );
    }
    if std::env::var("DUMP_ALL_HRAM").is_ok() {
        for row in 0..8u16 {
            let base = 0xFF80 + row * 16;
            let bytes: Vec<String> = (0..16).map(|i| format!("{:02x}", bus.read(base + i))).collect();
            eprintln!("{:04x}: {}", base, bytes.join(" "));
        }
    }

    // Dump BG tilemap $9800 as text (mooneye harness writes results there;
    // the font is ASCII tiles loaded at VRAM tile 1 = char 0x20).
    let mut text = String::new();
    for row in 0..18u16 {
        for col in 0..20u16 {
            let tile = bus.read(0x9800 + row * 32 + col);
            let ch = if !(0x20..0x7F).contains(&tile) { '.' } else { tile as char };
            text.push(ch);
        }
        text.push('\n');
    }
    eprintln!("SCREEN:\n{}", text);
}
