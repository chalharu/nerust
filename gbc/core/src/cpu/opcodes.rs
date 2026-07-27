use crate::cpu::registers::CpuRegisters;
use crate::memory::GbcMemoryBus;

pub fn execute(opcode: u8, reg: &mut CpuRegisters, bus: &mut GbcMemoryBus) -> u32 {
    match opcode {
        // ── 0x00-0x0F ──────────────────────────────────────────
        0x00 => 4, // NOP
        0x01 => {
            // LD BC, d16
            let lo = bus.read(reg.pc) as u16;
            reg.pc = reg.pc.wrapping_add(1);
            let hi = bus.read(reg.pc) as u16;
            reg.pc = reg.pc.wrapping_add(1);
            reg.set_bc((hi << 8) | lo);
            12
        }
        0x02 => {
            // LD (BC), A
            bus.write(reg.bc(), reg.a);
            8
        }
        0x03 => {
            // INC BC
            reg.set_bc(reg.bc().wrapping_add(1));
            8
        }
        0x04 => {
            // INC B
            reg.set_h((reg.b & 0x0F) == 0x0F);
            reg.b = reg.b.wrapping_add(1);
            reg.set_z(reg.b == 0);
            reg.set_n(false);
            4
        }
        0x05 => {
            // DEC B
            reg.set_h((reg.b & 0x0F) == 0);
            reg.b = reg.b.wrapping_sub(1);
            reg.set_z(reg.b == 0);
            reg.set_n(true);
            4
        }
        0x06 => {
            // LD B, d8
            reg.b = bus.read(reg.pc);
            reg.pc = reg.pc.wrapping_add(1);
            8
        }
        0x07 => {
            // RLCA
            let carry = reg.a & 0x80 != 0;
            reg.a = (reg.a << 1) | carry as u8;
            reg.set_z(false);
            reg.set_n(false);
            reg.set_h(false);
            reg.set_c(carry);
            4
        }
        0x08 => {
            // LD (a16), SP
            let lo = bus.read(reg.pc) as u16;
            reg.pc = reg.pc.wrapping_add(1);
            let hi = bus.read(reg.pc) as u16;
            reg.pc = reg.pc.wrapping_add(1);
            let addr = (hi << 8) | lo;
            bus.write(addr, reg.sp as u8);
            bus.write(addr.wrapping_add(1), (reg.sp >> 8) as u8);
            20
        }
        0x09 => {
            // ADD HL, BC
            let hl = reg.hl();
            let bc = reg.bc();
            reg.set_h((hl & 0x0FFF) + (bc & 0x0FFF) > 0x0FFF);
            reg.set_c(hl as u32 + bc as u32 > 0xFFFF);
            reg.set_n(false);
            reg.set_hl(hl.wrapping_add(bc));
            8
        }
        0x0A => {
            // LD A, (BC)
            reg.a = bus.read(reg.bc());
            8
        }
        0x0B => {
            // DEC BC
            reg.set_bc(reg.bc().wrapping_sub(1));
            8
        }
        0x0C => {
            // INC C
            reg.c = inc8(reg.c, reg);
            4
        }
        0x0D => {
            // DEC C
            reg.c = dec8(reg.c, reg);
            4
        }
        0x0E => {
            // LD C, d8
            reg.c = bus.read(reg.pc);
            reg.pc = reg.pc.wrapping_add(1);
            8
        }
        0x0F => {
            // RRCA
            let carry = reg.a & 0x01 != 0;
            reg.a = (reg.a >> 1) | if carry { 0x80 } else { 0 };
            reg.set_z(false);
            reg.set_n(false);
            reg.set_h(false);
            reg.set_c(carry);
            4
        }
        // ── 0x10-0x1F ──────────────────────────────────────────
        0x10 => {
            // STOP
            // STOP is 2 bytes (opcode + 0x00). PC already advanced by step().
            // Actually, step() advances PC by 1 before calling execute.
            // The second byte (0x00) is at the PC we already advanced to.
            // But there's a quirk: DIV reset and speed switch happen on stop.
            // We skip the second byte.
            reg.pc = reg.pc.wrapping_add(1); // skip nop byte
            bus.stop();
            4
        }
        0x11 => {
            // LD DE, d16
            let lo = bus.read(reg.pc) as u16;
            reg.pc = reg.pc.wrapping_add(1);
            let hi = bus.read(reg.pc) as u16;
            reg.pc = reg.pc.wrapping_add(1);
            reg.set_de((hi << 8) | lo);
            12
        }
        0x12 => {
            // LD (DE), A
            bus.write(reg.de(), reg.a);
            8
        }
        0x13 => {
            // INC DE
            reg.set_de(reg.de().wrapping_add(1));
            8
        }
        0x14 => {
            // INC D
            reg.d = inc8(reg.d, reg);
            4
        }
        0x15 => {
            // DEC D
            reg.d = dec8(reg.d, reg);
            4
        }
        0x16 => {
            // LD D, d8
            reg.d = bus.read(reg.pc);
            reg.pc = reg.pc.wrapping_add(1);
            8
        }
        0x17 => {
            // RLA
            let carry = reg.a & 0x80 != 0;
            reg.a = (reg.a << 1) | reg.c_flag() as u8;
            reg.set_z(false);
            reg.set_n(false);
            reg.set_h(false);
            reg.set_c(carry);
            4
        }
        0x18 => {
            // JR e
            let offset = bus.read(reg.pc) as i8;
            reg.pc = reg.pc.wrapping_add(1);
            reg.pc = reg.pc.wrapping_add_signed(offset as i16);
            12
        }
        0x19 => {
            // ADD HL, DE
            let hl = reg.hl();
            let de = reg.de();
            reg.set_h((hl & 0x0FFF) + (de & 0x0FFF) > 0x0FFF);
            reg.set_c(hl as u32 + de as u32 > 0xFFFF);
            reg.set_n(false);
            reg.set_hl(hl.wrapping_add(de));
            8
        }
        0x1A => {
            // LD A, (DE)
            reg.a = bus.read(reg.de());
            8
        }
        0x1B => {
            // DEC DE
            reg.set_de(reg.de().wrapping_sub(1));
            8
        }
        0x1C => {
            // INC E
            reg.e = inc8(reg.e, reg);
            4
        }
        0x1D => {
            // DEC E
            reg.e = dec8(reg.e, reg);
            4
        }
        0x1E => {
            // LD E, d8
            reg.e = bus.read(reg.pc);
            reg.pc = reg.pc.wrapping_add(1);
            8
        }
        0x1F => {
            // RRA
            let carry = reg.a & 0x01 != 0;
            reg.a = (reg.a >> 1) | if reg.c_flag() { 0x80 } else { 0 };
            reg.set_z(false);
            reg.set_n(false);
            reg.set_h(false);
            reg.set_c(carry);
            4
        }
        // ── 0x20-0x2F ──────────────────────────────────────────
        0x20 => {
            // JR NZ, e
            let offset = bus.read(reg.pc) as i8;
            reg.pc = reg.pc.wrapping_add(1);
            if !reg.z_flag() {
                reg.pc = reg.pc.wrapping_add_signed(offset as i16);
                return 12;
            }
            8
        }
        0x21 => {
            // LD HL, d16
            let lo = bus.read(reg.pc) as u16;
            reg.pc = reg.pc.wrapping_add(1);
            let hi = bus.read(reg.pc) as u16;
            reg.pc = reg.pc.wrapping_add(1);
            reg.set_hl((hi << 8) | lo);
            12
        }
        0x22 => {
            // LD (HL+), A
            let addr = reg.hl();
            bus.write(addr, reg.a);
            reg.set_hl(addr.wrapping_add(1));
            8
        }
        0x23 => {
            // INC HL
            reg.set_hl(reg.hl().wrapping_add(1));
            8
        }
        0x24 => {
            // INC H
            reg.h = inc8(reg.h, reg);
            4
        }
        0x25 => {
            // DEC H
            reg.h = dec8(reg.h, reg);
            4
        }
        0x26 => {
            // LD H, d8
            reg.h = bus.read(reg.pc);
            reg.pc = reg.pc.wrapping_add(1);
            8
        }
        0x27 => {
            // DAA
            let mut adjust = 0u8;
            let mut carry = reg.c_flag();
            if reg.n_flag() {
                if reg.h_flag() {
                    adjust |= 0x06;
                }
                if carry {
                    adjust |= 0x60;
                }
                reg.a = reg.a.wrapping_sub(adjust);
            } else {
                if reg.h_flag() || (reg.a & 0x0F) > 0x09 {
                    adjust |= 0x06;
                }
                if carry || reg.a > 0x99 {
                    adjust |= 0x60;
                    carry = true;
                }
                reg.a = reg.a.wrapping_add(adjust);
            }
            reg.set_z(reg.a == 0);
            reg.set_h(false);
            reg.set_c(carry);
            4
        }
        0x28 => {
            // JR Z, e
            let offset = bus.read(reg.pc) as i8;
            reg.pc = reg.pc.wrapping_add(1);
            if reg.z_flag() {
                reg.pc = reg.pc.wrapping_add_signed(offset as i16);
                return 12;
            }
            8
        }
        0x29 => {
            // ADD HL, HL
            let hl = reg.hl();
            reg.set_h((hl & 0x0FFF) + (hl & 0x0FFF) > 0x0FFF);
            reg.set_c((hl as u32) * 2 > 0xFFFF);
            reg.set_n(false);
            reg.set_hl(hl.wrapping_add(hl));
            8
        }
        0x2A => {
            // LD A, (HL+)
            reg.a = bus.read(reg.hl());
            reg.set_hl(reg.hl().wrapping_add(1));
            8
        }
        0x2B => {
            // DEC HL
            reg.set_hl(reg.hl().wrapping_sub(1));
            8
        }
        0x2C => {
            // INC L
            reg.l = inc8(reg.l, reg);
            4
        }
        0x2D => {
            // DEC L
            reg.l = dec8(reg.l, reg);
            4
        }
        0x2E => {
            // LD L, d8
            reg.l = bus.read(reg.pc);
            reg.pc = reg.pc.wrapping_add(1);
            8
        }
        0x2F => {
            // CPL
            reg.a = !reg.a;
            reg.set_n(true);
            reg.set_h(true);
            4
        }
        0x30 => {
            // JR NC, e
            let offset = bus.read(reg.pc) as i8;
            reg.pc = reg.pc.wrapping_add(1);
            if !reg.c_flag() {
                reg.pc = reg.pc.wrapping_add_signed(offset as i16);
                return 12;
            }
            8
        }
        0x31 => {
            // LD SP, d16
            let lo = bus.read(reg.pc) as u16;
            reg.pc = reg.pc.wrapping_add(1);
            let hi = bus.read(reg.pc) as u16;
            reg.pc = reg.pc.wrapping_add(1);
            reg.sp = (hi << 8) | lo;
            12
        }
        0x32 => {
            // LD (HL-), A
            let addr = reg.hl();
            bus.write(addr, reg.a);
            reg.set_hl(addr.wrapping_sub(1));
            8
        }
        0x33 => {
            // INC SP
            reg.sp = reg.sp.wrapping_add(1);
            8
        }
        0x34 => {
            // INC (HL)
            let addr = reg.hl();
            let mut v = bus.read(addr);
            v = inc8(v, reg);
            bus.write(addr, v);
            12
        }
        0x35 => {
            // DEC (HL)
            let addr = reg.hl();
            let mut v = bus.read(addr);
            v = dec8(v, reg);
            bus.write(addr, v);
            12
        }
        0x36 => {
            // LD (HL), d8
            bus.write(reg.hl(), bus.read(reg.pc));
            reg.pc = reg.pc.wrapping_add(1);
            12
        }
        0x37 => {
            // SCF
            reg.set_n(false);
            reg.set_h(false);
            reg.set_c(true);
            4
        }
        0x38 => {
            // JR C, e
            let offset = bus.read(reg.pc) as i8;
            reg.pc = reg.pc.wrapping_add(1);
            if reg.c_flag() {
                reg.pc = reg.pc.wrapping_add_signed(offset as i16);
                return 12;
            }
            8
        }
        0x39 => {
            // ADD HL, SP
            let hl = reg.hl();
            let sp = reg.sp;
            reg.set_h((hl & 0x0FFF) + (sp & 0x0FFF) > 0x0FFF);
            reg.set_c(hl as u32 + sp as u32 > 0xFFFF);
            reg.set_n(false);
            reg.set_hl(hl.wrapping_add(sp));
            8
        }
        0x3A => {
            // LD A, (HL-)
            reg.a = bus.read(reg.hl());
            reg.set_hl(reg.hl().wrapping_sub(1));
            8
        }
        0x3B => {
            // DEC SP
            reg.sp = reg.sp.wrapping_sub(1);
            8
        }
        0x3C => {
            // INC A
            reg.a = inc8(reg.a, reg);
            4
        }
        0x3D => {
            // DEC A
            reg.a = dec8(reg.a, reg);
            4
        }
        0x3E => {
            // LD A, d8
            reg.a = bus.read(reg.pc);
            reg.pc = reg.pc.wrapping_add(1);
            8
        }
        0x3F => {
            // CCF
            reg.set_n(false);
            reg.set_h(false);
            reg.set_c(!reg.c_flag());
            4
        }
        // ── 0x40-0x7F: LD r, r ─────────────────────────────────
        0x40 => 4, // LD B, B (NOP effect)
        0x41 => {
            reg.b = reg.c;
            4
        }
        0x42 => {
            reg.b = reg.d;
            4
        }
        0x43 => {
            reg.b = reg.e;
            4
        }
        0x44 => {
            reg.b = reg.h;
            4
        }
        0x45 => {
            reg.b = reg.l;
            4
        }
        0x46 => {
            reg.b = bus.read(reg.hl());
            8
        }
        0x47 => {
            reg.b = reg.a;
            4
        }
        0x48 => {
            reg.c = reg.b;
            4
        }
        0x49 => 4, // LD C, C
        0x4A => {
            reg.c = reg.d;
            4
        }
        0x4B => {
            // CB prefix
            let cb = bus.read(reg.pc);
            reg.pc = reg.pc.wrapping_add(1);
            execute_cb(cb, reg, bus)
        }
        0x4C => {
            reg.c = reg.e;
            4
        }
        0x4D => {
            reg.c = reg.h;
            4
        }
        0x4E => {
            reg.c = reg.l;
            4
        }
        0x4F => {
            reg.c = bus.read(reg.hl());
            8
        }
        0x50 => {
            reg.d = reg.b;
            4
        }
        0x51 => {
            reg.d = reg.c;
            4
        }
        0x52 => 4, // LD D, D
        0x53 => {
            reg.d = reg.e;
            4
        }
        0x54 => {
            reg.d = reg.h;
            4
        }
        0x55 => {
            reg.d = reg.l;
            4
        }
        0x56 => {
            reg.d = bus.read(reg.hl());
            8
        }
        0x57 => {
            reg.d = reg.a;
            4
        }
        0x58 => {
            reg.e = reg.b;
            4
        }
        0x59 => {
            reg.e = reg.c;
            4
        }
        0x5A => {
            reg.e = reg.d;
            4
        }
        0x5B => 4, // LD E, E
        0x5C => {
            reg.e = reg.h;
            4
        }
        0x5D => {
            reg.e = reg.l;
            4
        }
        0x5E => {
            reg.e = bus.read(reg.hl());
            8
        }
        0x5F => {
            reg.e = reg.a;
            4
        }
        0x60 => {
            reg.h = reg.b;
            4
        }
        0x61 => {
            reg.h = reg.c;
            4
        }
        0x62 => {
            reg.h = reg.d;
            4
        }
        0x63 => {
            reg.h = reg.e;
            4
        }
        0x64 => 4, // LD H, H
        0x65 => {
            reg.h = reg.l;
            4
        }
        0x66 => {
            reg.h = bus.read(reg.hl());
            8
        }
        0x67 => {
            reg.h = reg.a;
            4
        }
        0x68 => {
            reg.l = reg.b;
            4
        }
        0x69 => {
            reg.l = reg.c;
            4
        }
        0x6A => {
            reg.l = reg.d;
            4
        }
        0x6B => {
            reg.l = reg.e;
            4
        }
        0x6C => {
            reg.l = reg.h;
            4
        }
        0x6D => 4, // LD L, L
        0x6E => {
            reg.l = bus.read(reg.hl());
            8
        }
        0x6F => {
            reg.l = reg.a;
            4
        }
        0x70 => {
            bus.write(reg.hl(), reg.b);
            8
        }
        0x71 => {
            bus.write(reg.hl(), reg.c);
            8
        }
        0x72 => {
            bus.write(reg.hl(), reg.d);
            8
        }
        0x73 => {
            bus.write(reg.hl(), reg.e);
            8
        }
        0x74 => {
            bus.write(reg.hl(), reg.h);
            8
        }
        0x75 => {
            bus.write(reg.hl(), reg.l);
            8
        }
        0x76 => {
            // HALT
            // HALT behavior: if IME=0 and pending interrupt, HALT bug.
            // We delegate to InterruptController which handles the bug detection.
            bus.halt_cpu();
            // If halt bug triggers, pc stays at current position.
            // Otherwise, CPU stops until interrupt.
            4
        }
        // Note: bus.halt_cpu() doesn't exist yet; we'll add a facade.
        0x77 => {
            bus.write(reg.hl(), reg.a);
            8
        }
        0x78 => {
            reg.a = reg.b;
            4
        }
        0x79 => {
            reg.a = reg.c;
            4
        }
        0x7A => {
            reg.a = reg.d;
            4
        }
        0x7B => {
            reg.a = reg.e;
            4
        }
        0x7C => {
            reg.a = reg.h;
            4
        }
        0x7D => {
            reg.a = reg.l;
            4
        }
        0x7E => {
            reg.a = bus.read(reg.hl());
            8
        }
        0x7F => 4, // LD A, A
        // ── 0x80-0xBF: ALU ops ─────────────────────────────────
        0x80 => {
            add(reg, reg.b);
            4
        }
        0x81 => {
            add(reg, reg.c);
            4
        }
        0x82 => {
            add(reg, reg.d);
            4
        }
        0x83 => {
            add(reg, reg.e);
            4
        }
        0x84 => {
            add(reg, reg.h);
            4
        }
        0x85 => {
            add(reg, reg.l);
            4
        }
        0x86 => {
            add(reg, bus.read(reg.hl()));
            8
        }
        0x87 => {
            add(reg, reg.a);
            4
        }
        0x88 => {
            adc(reg, reg.b);
            4
        }
        0x89 => {
            adc(reg, reg.c);
            4
        }
        0x8A => {
            adc(reg, reg.d);
            4
        }
        0x8B => {
            adc(reg, reg.e);
            4
        }
        0x8C => {
            adc(reg, reg.h);
            4
        }
        0x8D => {
            adc(reg, reg.l);
            4
        }
        0x8E => {
            adc(reg, bus.read(reg.hl()));
            8
        }
        0x8F => {
            adc(reg, reg.a);
            4
        }
        0x90 => {
            sub(reg, reg.b);
            4
        }
        0x91 => {
            sub(reg, reg.c);
            4
        }
        0x92 => {
            sub(reg, reg.d);
            4
        }
        0x93 => {
            sub(reg, reg.e);
            4
        }
        0x94 => {
            sub(reg, reg.h);
            4
        }
        0x95 => {
            sub(reg, reg.l);
            4
        }
        0x96 => {
            sub(reg, bus.read(reg.hl()));
            8
        }
        0x97 => {
            sub(reg, reg.a);
            4
        }
        0x98 => {
            sbc(reg, reg.b);
            4
        }
        0x99 => {
            sbc(reg, reg.c);
            4
        }
        0x9A => {
            sbc(reg, reg.d);
            4
        }
        0x9B => {
            sbc(reg, reg.e);
            4
        }
        0x9C => {
            sbc(reg, reg.h);
            4
        }
        0x9D => {
            sbc(reg, reg.l);
            4
        }
        0x9E => {
            sbc(reg, bus.read(reg.hl()));
            8
        }
        0x9F => {
            sbc(reg, reg.a);
            4
        }
        // AND
        0xA0 => {
            and(reg, reg.b);
            4
        }
        0xA1 => {
            and(reg, reg.c);
            4
        }
        0xA2 => {
            and(reg, reg.d);
            4
        }
        0xA3 => {
            and(reg, reg.e);
            4
        }
        0xA4 => {
            and(reg, reg.h);
            4
        }
        0xA5 => {
            and(reg, reg.l);
            4
        }
        0xA6 => {
            and(reg, bus.read(reg.hl()));
            8
        }
        0xA7 => {
            and(reg, reg.a);
            4
        }
        // XOR
        0xA8 => {
            xor(reg, reg.b);
            4
        }
        0xA9 => {
            xor(reg, reg.c);
            4
        }
        0xAA => {
            xor(reg, reg.d);
            4
        }
        0xAB => {
            xor(reg, reg.e);
            4
        }
        0xAC => {
            xor(reg, reg.h);
            4
        }
        0xAD => {
            xor(reg, reg.l);
            4
        }
        0xAE => {
            xor(reg, bus.read(reg.hl()));
            8
        }
        0xAF => {
            xor(reg, reg.a);
            4
        }
        // OR
        0xB0 => {
            or(reg, reg.b);
            4
        }
        0xB1 => {
            or(reg, reg.c);
            4
        }
        0xB2 => {
            or(reg, reg.d);
            4
        }
        0xB3 => {
            or(reg, reg.e);
            4
        }
        0xB4 => {
            or(reg, reg.h);
            4
        }
        0xB5 => {
            or(reg, reg.l);
            4
        }
        0xB6 => {
            or(reg, bus.read(reg.hl()));
            8
        }
        0xB7 => {
            or(reg, reg.a);
            4
        }
        // CP
        0xB8 => {
            cp(reg, reg.b);
            4
        }
        0xB9 => {
            cp(reg, reg.c);
            4
        }
        0xBA => {
            cp(reg, reg.d);
            4
        }
        0xBB => {
            cp(reg, reg.e);
            4
        }
        0xBC => {
            cp(reg, reg.h);
            4
        }
        0xBD => {
            cp(reg, reg.l);
            4
        }
        0xBE => {
            cp(reg, bus.read(reg.hl()));
            8
        }
        0xBF => {
            cp(reg, reg.a);
            4
        }
        // ── 0xC0-0xCF ──────────────────────────────────────────
        0xC0 => {
            // RET NZ
            if !reg.z_flag() {
                ret(reg, bus);
                return 20;
            }
            8
        }
        0xC1 => {
            // POP BC
            {
                let v = pop(reg, bus);
                reg.set_bc(v);
            };
            12
        }
        0xC2 => {
            // JP NZ, a16
            let lo = bus.read(reg.pc) as u16;
            reg.pc = reg.pc.wrapping_add(1);
            let hi = bus.read(reg.pc) as u16;
            reg.pc = reg.pc.wrapping_add(1);
            if !reg.z_flag() {
                reg.pc = (hi << 8) | lo;
                return 16;
            }
            12
        }
        0xC3 => {
            // JP a16
            let lo = bus.read(reg.pc) as u16;
            reg.pc = reg.pc.wrapping_add(1);
            let hi = bus.read(reg.pc) as u16;
            reg.pc = (hi << 8) | lo;
            16
        }
        0xC4 => {
            // CALL NZ, a16
            let lo = bus.read(reg.pc) as u16;
            reg.pc = reg.pc.wrapping_add(1);
            let hi = bus.read(reg.pc) as u16;
            reg.pc = reg.pc.wrapping_add(1);
            if !reg.z_flag() {
                call(reg, bus, (hi << 8) | lo);
                return 24;
            }
            12
        }
        0xC5 => {
            // PUSH BC
            {
                let v = reg.bc();
                push(reg, bus, v);
            };
            16
        }
        0xC6 => {
            // ADD A, d8
            let v = bus.read(reg.pc);
            reg.pc = reg.pc.wrapping_add(1);
            add(reg, v);
            8
        }
        0xC7 => {
            // RST 00h
            rst(reg, bus, 0x00);
            16
        }
        0xC8 => {
            // RET Z
            if reg.z_flag() {
                ret(reg, bus);
                return 20;
            }
            8
        }
        0xC9 => {
            // RET
            ret(reg, bus);
            16
        }
        0xCA => {
            // JP Z, a16
            let lo = bus.read(reg.pc) as u16;
            reg.pc = reg.pc.wrapping_add(1);
            let hi = bus.read(reg.pc) as u16;
            reg.pc = reg.pc.wrapping_add(1);
            if reg.z_flag() {
                reg.pc = (hi << 8) | lo;
                return 16;
            }
            12
        }
        0xCB => unreachable!("CB prefix handled at 0x4B"),
        0xCC => {
            // CALL Z, a16
            let lo = bus.read(reg.pc) as u16;
            reg.pc = reg.pc.wrapping_add(1);
            let hi = bus.read(reg.pc) as u16;
            reg.pc = reg.pc.wrapping_add(1);
            if reg.z_flag() {
                call(reg, bus, (hi << 8) | lo);
                return 24;
            }
            12
        }
        0xCD => {
            // CALL a16
            let lo = bus.read(reg.pc) as u16;
            reg.pc = reg.pc.wrapping_add(1);
            let hi = bus.read(reg.pc) as u16;
            reg.pc = reg.pc.wrapping_add(1);
            call(reg, bus, (hi << 8) | lo);
            24
        }
        0xCE => {
            // ADC A, d8
            let v = bus.read(reg.pc);
            reg.pc = reg.pc.wrapping_add(1);
            adc(reg, v);
            8
        }
        0xCF => {
            // RST 08h
            rst(reg, bus, 0x08);
            16
        }
        // ── 0xD0-0xDF ──────────────────────────────────────────
        0xD0 => {
            // RET NC
            if !reg.c_flag() {
                ret(reg, bus);
                return 20;
            }
            8
        }
        0xD1 => {
            // POP DE
            {
                let v = pop(reg, bus);
                reg.set_de(v);
            };
            12
        }
        0xD2 => {
            // JP NC, a16
            let lo = bus.read(reg.pc) as u16;
            reg.pc = reg.pc.wrapping_add(1);
            let hi = bus.read(reg.pc) as u16;
            reg.pc = reg.pc.wrapping_add(1);
            if !reg.c_flag() {
                reg.pc = (hi << 8) | lo;
                return 16;
            }
            12
        }
        // Invalid opcodes — hardware would lock up.
        // For emulation: advance PC (already done by step()), no side effects, 1 M-cycle.
        0xD3 | 0xDB | 0xDD | 0xE3 | 0xE4 | 0xEB | 0xEC | 0xED | 0xF4 | 0xFC | 0xFD => 4,
        0xD4 => {
            // CALL NC, a16
            let lo = bus.read(reg.pc) as u16;
            reg.pc = reg.pc.wrapping_add(1);
            let hi = bus.read(reg.pc) as u16;
            reg.pc = reg.pc.wrapping_add(1);
            if !reg.c_flag() {
                call(reg, bus, (hi << 8) | lo);
                return 24;
            }
            12
        }
        0xD5 => {
            // PUSH DE
            {
                let v = reg.de();
                push(reg, bus, v);
            };
            16
        }
        0xD6 => {
            // SUB d8
            let v = bus.read(reg.pc);
            reg.pc = reg.pc.wrapping_add(1);
            sub(reg, v);
            8
        }
        0xD7 => {
            // RST 10h
            rst(reg, bus, 0x10);
            16
        }
        0xD8 => {
            // RET C
            if reg.c_flag() {
                ret(reg, bus);
                return 20;
            }
            8
        }
        0xD9 => {
            // RETI
            ret(reg, bus);
            bus.set_ime(true);
            16
        }
        0xDA => {
            // JP C, a16
            let lo = bus.read(reg.pc) as u16;
            reg.pc = reg.pc.wrapping_add(1);
            let hi = bus.read(reg.pc) as u16;
            reg.pc = reg.pc.wrapping_add(1);
            if reg.c_flag() {
                reg.pc = (hi << 8) | lo;
                return 16;
            }
            12
        }
        0xDC => {
            // CALL C, a16
            let lo = bus.read(reg.pc) as u16;
            reg.pc = reg.pc.wrapping_add(1);
            let hi = bus.read(reg.pc) as u16;
            reg.pc = reg.pc.wrapping_add(1);
            if reg.c_flag() {
                call(reg, bus, (hi << 8) | lo);
                return 24;
            }
            12
        }
        0xDE => {
            // SBC A, d8
            let v = bus.read(reg.pc);
            reg.pc = reg.pc.wrapping_add(1);
            sbc(reg, v);
            8
        }
        0xDF => {
            // RST 18h
            rst(reg, bus, 0x18);
            16
        }
        // ── 0xE0-0xEF ──────────────────────────────────────────
        0xE0 => {
            // LDH (a8), A
            let addr = 0xFF00 | bus.read(reg.pc) as u16;
            reg.pc = reg.pc.wrapping_add(1);
            bus.write(addr, reg.a);
            12
        }
        0xE1 => {
            // POP HL
            {
                let v = pop(reg, bus);
                reg.set_hl(v);
            };
            12
        }
        0xE2 => {
            // LD (C), A  (LDH (FF00+C), A)
            bus.write(0xFF00 | reg.c as u16, reg.a);
            8
        }
        0xE5 => {
            // PUSH HL
            {
                let v = reg.hl();
                push(reg, bus, v);
            };
            16
        }
        0xE6 => {
            // AND d8
            let v = bus.read(reg.pc);
            reg.pc = reg.pc.wrapping_add(1);
            and(reg, v);
            8
        }
        0xE7 => {
            // RST 20h
            rst(reg, bus, 0x20);
            16
        }
        0xE8 => {
            // ADD SP, e
            let offset = bus.read(reg.pc) as i8;
            reg.pc = reg.pc.wrapping_add(1);
            let result = reg.sp.wrapping_add_signed(offset as i16);
            reg.set_h((reg.sp & 0x000F) + (offset as u8 as u16 & 0x000F) > 0x000F);
            reg.set_c((reg.sp & 0x00FF) + (offset as u8 as u16 & 0x00FF) > 0x00FF);
            reg.set_z(false);
            reg.set_n(false);
            reg.sp = result;
            16
        }
        0xE9 => {
            // JP (HL)
            reg.pc = reg.hl();
            4
        }
        0xEA => {
            // LD (a16), A
            let lo = bus.read(reg.pc) as u16;
            reg.pc = reg.pc.wrapping_add(1);
            let hi = bus.read(reg.pc) as u16;
            reg.pc = reg.pc.wrapping_add(1);
            bus.write((hi << 8) | lo, reg.a);
            16
        }
        0xEE => {
            // XOR d8
            let v = bus.read(reg.pc);
            reg.pc = reg.pc.wrapping_add(1);
            xor(reg, v);
            8
        }
        0xEF => {
            // RST 28h
            rst(reg, bus, 0x28);
            16
        }
        // ── 0xF0-0xFF ──────────────────────────────────────────
        0xF0 => {
            // LDH A, (a8)
            let addr = 0xFF00 | bus.read(reg.pc) as u16;
            reg.pc = reg.pc.wrapping_add(1);
            reg.a = bus.read(addr);
            12
        }
        0xF1 => {
            // POP AF
            {
                let v = pop(reg, bus);
                reg.set_af(v);
            };
            12
        }
        0xF2 => {
            // LD A, (C)  (LDH A, (FF00+C))
            reg.a = bus.read(0xFF00 | reg.c as u16);
            8
        }
        0xF3 => {
            // DI
            bus.set_ime(false);
            4
        }
        0xF5 => {
            // PUSH AF
            {
                let v = reg.af();
                push(reg, bus, v);
            };
            16
        }
        0xF6 => {
            // OR d8
            let v = bus.read(reg.pc);
            reg.pc = reg.pc.wrapping_add(1);
            or(reg, v);
            8
        }
        0xF7 => {
            // RST 30h
            rst(reg, bus, 0x30);
            16
        }
        0xF8 => {
            // LD HL, SP+e
            let offset = bus.read(reg.pc) as i8;
            reg.pc = reg.pc.wrapping_add(1);
            let result = reg.sp.wrapping_add_signed(offset as i16);
            reg.set_h((reg.sp & 0x000F) + (offset as u8 as u16 & 0x000F) > 0x000F);
            reg.set_c((reg.sp & 0x00FF) + (offset as u8 as u16 & 0x00FF) > 0x00FF);
            reg.set_z(false);
            reg.set_n(false);
            reg.set_hl(result);
            12
        }
        0xF9 => {
            // LD SP, HL
            reg.sp = reg.hl();
            8
        }
        0xFA => {
            // LD A, (a16)
            let lo = bus.read(reg.pc) as u16;
            reg.pc = reg.pc.wrapping_add(1);
            let hi = bus.read(reg.pc) as u16;
            reg.pc = reg.pc.wrapping_add(1);
            reg.a = bus.read((hi << 8) | lo);
            16
        }
        0xFB => {
            // EI
            bus.set_ime(true);
            // EI is delayed by one instruction. Return value includes 4 cycles.
            4
        }
        0xFE => {
            // CP d8
            let v = bus.read(reg.pc);
            reg.pc = reg.pc.wrapping_add(1);
            cp(reg, v);
            8
        }
        0xFF => {
            // RST 38h
            rst(reg, bus, 0x38);
            16
        }
    }
}

// ── ALU helpers ─────────────────────────────────────────────────

fn inc8(v: u8, reg: &mut CpuRegisters) -> u8 {
    reg.set_h((v & 0x0F) == 0x0F);
    let result = v.wrapping_add(1);
    reg.set_z(result == 0);
    reg.set_n(false);
    result
}

fn dec8(v: u8, reg: &mut CpuRegisters) -> u8 {
    reg.set_h((v & 0x0F) == 0);
    let result = v.wrapping_sub(1);
    reg.set_z(result == 0);
    reg.set_n(true);
    result
}

fn add(reg: &mut CpuRegisters, v: u8) {
    let result = reg.a as u16 + v as u16;
    reg.set_h((reg.a & 0x0F) + (v & 0x0F) > 0x0F);
    reg.set_c(result > 0xFF);
    reg.a = result as u8;
    reg.set_z(reg.a == 0);
    reg.set_n(false);
}

fn adc(reg: &mut CpuRegisters, v: u8) {
    let carry = reg.c_flag() as u16;
    let result = reg.a as u16 + v as u16 + carry;
    reg.set_h((reg.a & 0x0F) + (v & 0x0F) + carry as u8 > 0x0F);
    reg.set_c(result > 0xFF);
    reg.a = result as u8;
    reg.set_z(reg.a == 0);
    reg.set_n(false);
}

fn sub(reg: &mut CpuRegisters, v: u8) {
    reg.set_h((reg.a & 0x0F) < (v & 0x0F));
    reg.set_c(reg.a < v);
    reg.a = reg.a.wrapping_sub(v);
    reg.set_z(reg.a == 0);
    reg.set_n(true);
}

fn sbc(reg: &mut CpuRegisters, v: u8) {
    let carry = reg.c_flag() as u8;
    let result = reg.a.wrapping_sub(v).wrapping_sub(carry);
    reg.set_h((reg.a & 0x0F) < (v & 0x0F) + carry);
    reg.set_c(reg.a < v.wrapping_add(carry));
    reg.a = result;
    reg.set_z(reg.a == 0);
    reg.set_n(true);
}

fn and(reg: &mut CpuRegisters, v: u8) {
    reg.a &= v;
    reg.set_z(reg.a == 0);
    reg.set_n(false);
    reg.set_h(true);
    reg.set_c(false);
}

fn or(reg: &mut CpuRegisters, v: u8) {
    reg.a |= v;
    reg.set_z(reg.a == 0);
    reg.set_n(false);
    reg.set_h(false);
    reg.set_c(false);
}

fn xor(reg: &mut CpuRegisters, v: u8) {
    reg.a ^= v;
    reg.set_z(reg.a == 0);
    reg.set_n(false);
    reg.set_h(false);
    reg.set_c(false);
}

fn cp(reg: &mut CpuRegisters, v: u8) {
    let r = reg.a;
    sub(reg, v);
    reg.a = r; // restore A (CP doesn't modify A, only flags)
    // sub already sets Z/N/H/C correctly for A-v
}

// ── stack / control flow helpers ────────────────────────────────

fn push(reg: &mut CpuRegisters, bus: &mut GbcMemoryBus, v: u16) {
    reg.sp = reg.sp.wrapping_sub(1);
    bus.write(reg.sp, (v >> 8) as u8);
    reg.sp = reg.sp.wrapping_sub(1);
    bus.write(reg.sp, v as u8);
}

fn pop(reg: &mut CpuRegisters, bus: &mut GbcMemoryBus) -> u16 {
    let lo = bus.read(reg.sp) as u16;
    reg.sp = reg.sp.wrapping_add(1);
    let hi = bus.read(reg.sp) as u16;
    reg.sp = reg.sp.wrapping_add(1);
    (hi << 8) | lo
}

fn call(reg: &mut CpuRegisters, bus: &mut GbcMemoryBus, addr: u16) {
    push(reg, bus, reg.pc);
    reg.pc = addr;
}

fn ret(reg: &mut CpuRegisters, bus: &mut GbcMemoryBus) {
    reg.pc = pop(reg, bus);
}

fn rst(reg: &mut CpuRegisters, bus: &mut GbcMemoryBus, addr: u16) {
    push(reg, bus, reg.pc);
    reg.pc = addr;
}

// ── CB prefix helper ────────────────────────────────────────────

// Runtime dispatch table for CB-prefixed ops (indirect instruction register)
enum CbTarget {
    B,
    C,
    D,
    E,
    H,
    L,
    HlIndirect,
    A,
}

impl CbTarget {
    fn from_opcode(op: u8) -> Self {
        match op & 0x07 {
            0 => CbTarget::B,
            1 => CbTarget::C,
            2 => CbTarget::D,
            3 => CbTarget::E,
            4 => CbTarget::H,
            5 => CbTarget::L,
            6 => CbTarget::HlIndirect,
            7 => CbTarget::A,
            _ => unreachable!(),
        }
    }

    fn read(&self, reg: &CpuRegisters, bus: &GbcMemoryBus) -> u8 {
        match self {
            CbTarget::B => reg.b,
            CbTarget::C => reg.c,
            CbTarget::D => reg.d,
            CbTarget::E => reg.e,
            CbTarget::H => reg.h,
            CbTarget::L => reg.l,
            CbTarget::HlIndirect => bus.read(reg.hl()),
            CbTarget::A => reg.a,
        }
    }

    fn write(&self, reg: &mut CpuRegisters, bus: &mut GbcMemoryBus, v: u8) {
        match self {
            CbTarget::B => reg.b = v,
            CbTarget::C => reg.c = v,
            CbTarget::D => reg.d = v,
            CbTarget::E => reg.e = v,
            CbTarget::H => reg.h = v,
            CbTarget::L => reg.l = v,
            CbTarget::HlIndirect => bus.write(reg.hl(), v),
            CbTarget::A => reg.a = v,
        }
    }

    fn cycles(&self) -> u32 {
        match self {
            CbTarget::HlIndirect => 16,
            _ => 8,
        }
    }
}

fn execute_cb(opcode: u8, reg: &mut CpuRegisters, bus: &mut GbcMemoryBus) -> u32 {
    let target = CbTarget::from_opcode(opcode);
    let group = opcode >> 6;
    let sub_op = (opcode >> 3) & 0x07;
    let cycles = target.cycles();

    match group {
        0x00 => match sub_op {
            0 => {
                let mut v = target.read(reg, bus);
                let c = v & 0x80 != 0;
                v = (v << 1) | c as u8;
                reg.set_z(v == 0);
                reg.set_n(false);
                reg.set_h(false);
                reg.set_c(c);
                target.write(reg, bus, v);
                cycles
            }
            1 => {
                let mut v = target.read(reg, bus);
                let c = v & 0x01 != 0;
                v = (v >> 1) | if c { 0x80 } else { 0 };
                reg.set_z(v == 0);
                reg.set_n(false);
                reg.set_h(false);
                reg.set_c(c);
                target.write(reg, bus, v);
                cycles
            }
            2 => {
                let mut v = target.read(reg, bus);
                let c = v & 0x80 != 0;
                v = (v << 1) | reg.c_flag() as u8;
                reg.set_z(v == 0);
                reg.set_n(false);
                reg.set_h(false);
                reg.set_c(c);
                target.write(reg, bus, v);
                cycles
            }
            3 => {
                let mut v = target.read(reg, bus);
                let c = v & 0x01 != 0;
                v = (v >> 1) | if reg.c_flag() { 0x80 } else { 0 };
                reg.set_z(v == 0);
                reg.set_n(false);
                reg.set_h(false);
                reg.set_c(c);
                target.write(reg, bus, v);
                cycles
            }
            4 => {
                let mut v = target.read(reg, bus);
                let c = v & 0x80 != 0;
                v <<= 1;
                reg.set_z(v == 0);
                reg.set_n(false);
                reg.set_h(false);
                reg.set_c(c);
                target.write(reg, bus, v);
                cycles
            }
            5 => {
                let mut v = target.read(reg, bus);
                let c = v & 0x01 != 0;
                v = (v >> 1) | (v & 0x80);
                reg.set_z(v == 0);
                reg.set_n(false);
                reg.set_h(false);
                reg.set_c(c);
                target.write(reg, bus, v);
                cycles
            }
            6 => {
                let mut v = target.read(reg, bus);
                v = v.rotate_right(4);
                reg.set_z(v == 0);
                reg.set_n(false);
                reg.set_h(false);
                reg.set_c(false);
                target.write(reg, bus, v);
                cycles
            }
            7 => {
                let mut v = target.read(reg, bus);
                let c = v & 0x01 != 0;
                v >>= 1;
                reg.set_z(v == 0);
                reg.set_n(false);
                reg.set_h(false);
                reg.set_c(c);
                target.write(reg, bus, v);
                cycles
            }
            _ => unreachable!(),
        },
        0x01 => {
            // BIT n, r
            let bit = sub_op;
            let v = target.read(reg, bus);
            reg.set_z(v & (1 << bit) == 0);
            reg.set_n(false);
            reg.set_h(true);
            cycles
        }
        0x02 => {
            // RES n, r
            let bit = sub_op;
            let mut v = target.read(reg, bus);
            v &= !(1 << bit);
            target.write(reg, bus, v);
            cycles
        }
        _ => {
            // SET n, r
            let bit = sub_op;
            let mut v = target.read(reg, bus);
            v |= 1 << bit;
            target.write(reg, bus, v);
            cycles
        }
    }
}
