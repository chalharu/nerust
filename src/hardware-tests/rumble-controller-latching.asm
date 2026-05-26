// A test to determine what the Rumble Controller does when the controller is
// latched (pin 3 (OUT0) of the controller port is pulsed).
//
// Mesen, MiSTer and the PDF spec document have different behaviours when
// `JOYOUT.bit0` is pulsed.
//
//  * The [2.0 spec doc](https://github.com/LimitedRunGames-Tech/snes-rumble/tree/main/docs)
//    says the 16-bit buffer should be cleared.
//    It does not state if the motors should stop.
//
//  * SNES_MiSTer clears the 16-bit buffer and stops the motors.
//    https://github.com/MiSTer-devel/SNES_MiSTer/blob/1613eea55235eb5f0aca16c73c5200161ced3cb2/rtl/ioport.sv#L74
//
//  * Mesen does neither.
//    https://github.com/nesdev-org/MesenCE/blob/4b8669d34bba11b4dae057ac0b234714a7f7638d/Core/SNES/Input/SnesRumbleController.cpp#L28
//
//
// This test contains 4 different write tests to determine the latching
// behaviour of a real Rumble Controller:
//
//  * **D-PAD*: Writes 16 bits of data after auto-read has completed.
//
//  * **ABXY**: Writes 8 bits of data, latches the controller,
//    then writes the final 8 bits of data.
//
//  * **L/R**:  Write 16 bits of data then latches the controller.
//
//  * **SELECT**: Tests if the rumble controller clears the internal 16-bit
//    shift register after activating the rumble motors.
//
//    It writes `$727200` to the shift register.
//      * If the shift register is cleared after activating the motors, the SR
//        will be `$0000` and the motors will remain active.
//      * If the shift register is not cleared after activating the motors, the
//        SR will be `$7200` and the motors will stop.
//
// If the controller continues to rumble after the buttons have been released,
// the **START** button will send a no-rumble command to the rumble controller.
//
//
// SPDX-FileCopyrightText: © 2026 Marcus Rowe <undisbeliever@gmail.com>
// SPDX-License-Identifier: Zlib
//
// Copyright © 2024 Marcus Rowe <undisbeliever@gmail.com>
//
// This software is provided 'as-is', without any express or implied warranty.
// In no event will the authors be held liable for any damages arising from the
// use of this software.
//
// Permission is granted to anyone to use this software for any purpose, including
// commercial applications, and to alter it and redistribute it freely, subject to
// the following restrictions:
//
//    1. The origin of this software must not be misrepresented; you must not
//       claim that you wrote the original software. If you use this software in
//       a product, an acknowledgment in the product documentation would be
//       appreciated but is not required.
//
//    2. Altered source versions must be plainly marked as such, and must not be
//       misrepresented as being the original software.
//
//    3. This notice may not be removed or altered from any source distribution.


define MEMORY_MAP = LOROM
define ROM_SIZE = 1
define ROM_SPEED = fast
define REGION = Japan
define ROM_NAME = "RUMBLE CONTROLLER"
define VERSION = 2

architecture wdc65816-strict

include "../common.inc"


createCodeBlock(code,       0x808000, 0x80ffaf)
createCodeBlock(rodata0,    0x818000, 0x81ffff)

createRamBlock(zeropage,        0x00,     0xff)
createRamBlock(lowram,      0x7e0100, 0x7e1f7f)
createRamBlock(stack,       0x7e1f80, 0x7e1fff)
createRamBlock(wram7e,      0x7e2000, 0xfeffff)


constant VRAM_BG1_MAP_WADDR   = 0x0000
constant VRAM_BG1_TILES_WADDR = 0x1000

constant VRAM_TEXTBUFFER_MAP_WADDR   = VRAM_BG1_MAP_WADDR
constant VRAM_TEXTBUFFER_TILES_WADDR = VRAM_BG1_TILES_WADDR


include "../reset_handler.inc"
include "../break_handler.inc"
include "../dma_forceblank.inc"
include "../textbuffer.inc"


// zero-page temporary word variables
allocate(zpTmp0, zeropage, 2)
allocate(zpTmp1, zeropage, 2)
allocate(zpTmp2, zeropage, 2)
allocate(zpTmp3, zeropage, 2)

// zero-page temporary far pointer
allocate(zpTmpPtr, zeropage, 3)



constant RUMBLE_SENTRY = %01110010


// Write the rumble data normally.
//
// IN: A = rumble data
a8()
i16()
// DB = $80
code()
function WriteNormally {
    pha

    lda.b   #RUMBLE_SENTRY
    jsr     WriteIoByte

    pla
    jmp     WriteIoByte
}


// Write the 8-bit rumble sentry, latch the controller then write
// the 8-bit rumble data.
//
// IN: A = rumble data
a8()
i16()
// DB = $80
code()
function Write8LatchWrite8 {
    pha

    lda.b   #RUMBLE_SENTRY
    jsr     WriteIoByte

    // Delay
    ldx.w   #200
    -
        dex
        bne -


    // Latch the joypad inbetween the two writes
    lda.b   #JOYSER0.latch
    sta.w   JOYSER0
    stz.w   JOYSER0

    // Delay
    ldx.w   #200
    -
        dex
        bne -

    pla
    jmp     WriteIoByte
}


// Write the rumble data normally, them immediately latch the controller.
//
// IN: A = rumble data
a8()
i16()
// DB = $80
code()
function Write16Latch {
    pha

    lda.b   #RUMBLE_SENTRY
    jsr     WriteIoByte

    pla
    jsr     WriteIoByte

    // Latch the joypad
    lda.b   #JOYSER0.latch
    sta.w   JOYSER0
    stz.w   JOYSER0

    rts
}


// Test if the internal 16 bit shift register is cleared after
// the rumble motors are active (SELECT test).
//
// IN: A = rumble data
a8()
i16()
// DB = $80
code()
function SrClearOnRumbleTest {
    lda.b   #RUMBLE_SENTRY
    jsr     WriteIoByte

    lda.b   #RUMBLE_SENTRY
    jsr     WriteIoByte

    // Write 8 zero bits really quickly
    stz.w   WRIO
    bit.w   JOYSER0
    bit.w   JOYSER0
    bit.w   JOYSER0
    bit.w   JOYSER0
    bit.w   JOYSER0
    bit.w   JOYSER0
    bit.w   JOYSER0
    bit.w   JOYSER0

    lda.b   #0xff
    sta.w   WRIO

    rts
}


// Writes 8 bits of data to the rumble controller.
//
// NOTE: This is not the fastest way to write the data.
// It is designed to only modify bit 6 of WRIO.
//
// IN: A = byte
a8()
i16()
// DB = $80
code()
function WriteIoByte {
    sep     #$30
a8()
i8()

    ldy.b   #8
    Loop:
        asl
        ldx.b   #~$40
        bcc     +
            ldx.b   #$ff
        +
        stx.w   WRIO
        bit.w   JOYSER0

        dey
        bne     Loop

    ldx.b   #$ff
    stx.w   WRIO

    rep     #$10
i16()
    rts
}


TestInstructions:
    db "Rumble Controller Latch Test"
    db "v{VERSION}\n"
    db "\n"
    db "\n"
    db "\n"
    db "D-PAD:  Write normally\n"
    db "\n"
    db "ABXY:   Latch after 8 bits\n"
    db "\n"
    db "L/R:    Latch after 16 bits\n"
    db "\n"
    db "SELECT: SR cleared test\n"
    db "\n"
    db "START:  Stop rumble\n"
    db 0


a8()
i16()
// DB = $80
code()
function RunTest {
    lda.w   joypadCurrent + 1
    bit.b   #JOYH.up
    beq     +
        lda.b   #0xff
        jmp     WriteNormally
    +
    bit.b   #JOYH.down
    beq     +
        lda.b   #0x22
        jmp     WriteNormally
    +
    bit.b   #JOYH.left
    beq     +
        lda.b   #0x0f
        jmp     WriteNormally
    +
    bit.b   #JOYH.right
    beq     +
        lda.b   #0xf0
        jmp     WriteNormally
    +

    bit.b   #JOYH.b
    beq     +
        lda.b   #0x22
        jmp     Write8LatchWrite8
    +
    bit.b   #JOYH.y
    beq     +
        lda.b   #0x0f
        jmp     Write8LatchWrite8
    +


    lda.w   joypadCurrent

    bit.b   #JOYL.x
    beq     +
        lda.b   #0xff
        jmp     Write8LatchWrite8
    +
    bit.b   #JOYL.a
    beq     +
        lda.b   #0xf0
        jmp     Write8LatchWrite8
    +

    bit.b   #JOYL.l
    beq     +
        lda.b   #0x0f
        jmp     Write16Latch
    +
    bit.b   #JOYL.r
    beq     +
        lda.b   #0xf0
        jmp     Write16Latch
    +


    lda.w   joypadCurrent + 1
    bit.b   #JOYH.select
    beq     +
        jmp     SrClearOnRumbleTest
    +
    bit.b   #JOYH.start
    beq     +
        lda.b   #0
        jmp     WriteNormally
    +

    rts
}



// VBlank routine
//
// REQUIRES: 8 bit A, 16 bit Index, DB = 0x80, DP = 0
macro VBlank() {
    assert8a()
    assert16i()

    TextBuffer.VBlank()
}

define VBLANK_READS_JOYPAD = 1

include "../vblank_interrupts.inc"



au()
iu()
// DB = $80
code()
function Main {
    rep     #$30
    sep     #$20
a8()
i16()
    // Setup PPU
    lda.b   #INIDISP.force | 0x0f
    sta.w   INIDISP

    lda.b   #BGMODE.mode0
    sta.w   BGMODE

    lda.b   #TM.bg1
    sta.w   TM

    lda.b   #(VRAM_BG1_MAP_WADDR / BGXSC.base.walign) << BGXSC.base.shift | BGXSC.map.s32x32
    sta.w   BG1SC

    lda.b   #(VRAM_BG1_TILES_WADDR / BG12NBA.walign) << BG12NBA.bg1.shift
    sta.w   BG12NBA

    stz.w   CGADD
    Dma.ForceBlank.ToCgram(Resources.Palette)

    jsr     TextBuffer.InitAndTransferToVram


    TextBuffer.SetCursor(0, 1)
    TextBuffer.PrintString(TestInstructions)

    EnableVblankInterrupts()

    jsr     WaitFrame

    lda.b   #0x0f
    sta.w   INIDISP

    MainLoop:
        jsr     WaitFrame

        jsr     RunTest

        jmp     MainLoop
}


namespace Resources {

Palette:
    dw  ToPalette(0, 0, 0)
    dw  ToPalette(31, 31, 31)
constant Palette.size = pc() - Palette
}

finalizeMemory()

