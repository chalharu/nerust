// Fixed subroutine arguments after the JSR/JSL instruction examples.
//
// These example subroutines read a 16 or 24 bit argument after the `JSR` or
// `JSL` instruction and then pass that value to the `TextBuffer.PrintString`
// subroutine.
//
//
// SPDX-FileCopyrightText: © 2026 Marcus Rowe <undisbeliever@gmail.com>
// SPDX-License-Identifier: Zlib
//
// Copyright © 2022 Marcus Rowe <undisbeliever@gmail.com>
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
define ROM_NAME = "ARGUMENTS AFTER JSR"
define VERSION = 0

architecture wdc65816-strict

include "../common.inc"


createCodeBlock(code,       0x808000, 0x80ffaf)
createCodeBlock(code1,      0x818000, 0x81ffff)
createCodeBlock(rodata0,    0x828000, 0x82ffff)

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



// VBlank routine
//
// REQUIRES: 8 bit A, 16 bit Index, DB = 0x80, DP = 0
macro VBlank() {
    assert8a()
    assert16i()

    TextBuffer.VBlank()
}

include "../vblank_interrupts.inc"



au()
iu()
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


    EnableVblankInterrupts()

    jsr     WaitFrame

    lda.b   #0x0f
    sta.w   INIDISP



    MainLoop:
        TextBuffer.SetCursor(0, 0)


        jsr     PrintString_Word_LongIndexed
            dw      HelloWorld

        jsl     PrintString_Word_AddrIndexed__far
            dw      HelloWorld


        jsr     PrintString_Long_StackIndirect
            dl      HelloWorld

        jsl     PrintString_Long_StackIndirect__far
            dl      HelloWorld


        jsr     PrintString_Long_DpIndirectLong
            dl      HelloWorld

        jsl     PrintString_Long_DpIndirectLong__far
            dl      HelloWorld


        jsr     WaitFrame

        jmp     MainLoop
}



// Reads a 16 bit address argument after the `JSR` and passes it to
// `TextBuffer.PrintString`.
//
// This example uses the `Absolute Long Indexed, X` addressing to read the
// subroutine argument.
//
// INPUT:
//  * dw 16-bit string address in STRING_BANK
//
// DB = low-RAM
a8()
i16()
code()
function PrintString_Word_LongIndexed {
    rep     #$31
a16()
    // Load and increment return address
    lda     1,s
    tax
    // carry clear
    adc.w   #2
    sta     1,s

    // X = 1 byte before the arguments

    // Read arguments
    lda.l   (pc() & 0xff0000) + 1,x
    tax

    sep     #$20
a8()

    lda.b   #STRING_BANK
    jmp     TextBuffer.PrintString
}



// Reads a 16 bit address argument after the `JSL` and passes it to
// `TextBuffer.PrintString`.
//
// This example uses the `Absolute Indexed, X` addressing to read the
// subroutine argument.
//
// INPUT:
//  * dw 16-bit string address in STRING_BANK
//
// DB = low-RAM
a8()
i16()
code(code1)
function PrintString_Word_AddrIndexed__far {
    // Confirm caller and callee banks are different
    assert((pc() >> 16) & 0x3f != 0)

    phb

    // Set DB to return bank
    lda     4,s
    pha
    plb
// DB = return bank

    rep     #$31
a16()
    // Load and increment return address
    lda     2,s
    tax
    // carry clear
    adc.w   #2
    sta     2,s

    // X = 1 byte before the arguments

    // Read argument
    lda.w   1,x
    tax

    sep     #$20
a8()

    plb
// DB restored

    lda.b   #STRING_BANK
    jml     _TB_PrintString__far
}



// Reads a 24 bit address argument after the `JSR` and passes it to
// `TextBuffer.PrintString`.
//
// This example uses the `SR Indirect Indexed, Y` addressing to read the
// subroutine argument.
//
// INPUT:
//  * dl 24-bit string address
//
//
// DB = low-RAM
a8()
i16()
code()
function PrintString_Long_StackIndirect {
    phb

    phk
    plb
// DB = PK

    rep     #$31
a16()
    // Read word address argument
    ldy.w   #1
    lda     (2,s),y
    iny
    iny

    tax

    // Read bank argument
    lda     (2,s),y
    tay

    // Increment return address
    lda     2,s
    // carry clear
    adc.w   #3
    sta     2,s

    sep     #$20
a8()

    plb
// DB restored

    tya
    jmp     TextBuffer.PrintString
}



// Reads a 24 bit address argument after the `JSL` and passes it to
// `TextBuffer.PrintString`.
//
// This example uses the `SR Indirect Indexed, Y` addressing to read the
// subroutine argument.
//
// DB = low-RAM
a8()
i16()
code(code1)
function PrintString_Long_StackIndirect__far {
    // Confirm caller and callee banks are different
    assert((pc() >> 16) & 0x3f != 0)

    phb

    // Set DB to caller's program bank
    lda     4,s
    pha
    plb

    rep     #$31
a16()
    // Read bank argument
    ldy.w   #1
    lda     (2,s),y
    iny
    iny

    tax

    // Read bank argument
    lda     (2,s),y
    tay

    // Increment return address
    lda     2,s
    // carry clear
    adc.w   #3
    sta     2,s

    sep     #$20
a8()

    plb

    tya
    jml     _TB_PrintString__far
}



// Reads a 24 bit address argument after the `JSR` and passes it to
// `TextBuffer.PrintString`.
//
// This example uses the DP Indirect Long addressing modes to read the
// subroutine argument.
//
// DP = 0
// DB = low-RAM
a8()
i16()
code()
function PrintString_Long_DpIndirectLong {
constant _ptr = zpTmpPtr

    lda.b   #pc() >> 16
    sta.b   _ptr + 2

    rep     #$31
a16()
    // Get and increment return address
    lda     1,s
    inc
    sta.b   _ptr
    // carry clear
    adc.w   #3 - 1
    sta     1,s

    // Read word address argument
    lda     [_ptr]
    tax

    sep     #$20
a8()

    // Read bank argument
    ldy.w   #2
    lda     [_ptr],y

    jmp     TextBuffer.PrintString
}



// Reads a 24 bit address argument after the `JSL` and passes it to
// `TextBuffer.PrintString`.
//
// This example uses the DP Indirect Long addressing modes to read the
// subroutine argument.
//
// DP = 0
// DB = low-RAM
a8()
i16()
code(code1)
function PrintString_Long_DpIndirectLong__far {
    // Confirm caller and callee banks are different
    assert((pc() >> 16) & 0x3f != 0)

constant _ptr = zpTmpPtr

    // Get return bank
    lda     3,s
    sta.b   _ptr + 2

    rep     #$31
a16()
    // Get and increment return address
    lda     1,s
    inc
    sta.b   _ptr
    // carry clear
    adc.w   #3 - 1
    sta     1,s


    // Read subroutine argument
    lda     [_ptr]
    tax

    sep     #$20
a8()
    ldy.w   #2
    lda     [_ptr],y


    jml     _TB_PrintString__far
}


// Hack to call `TextBuffer.PrintString` from program bank 0x81
//
// DB = low-RAM
a8()
i16()
code()
function _TB_PrintString__far {
    jsr     TextBuffer.PrintString
    rtl
}


rodata(rodata0)

constant STRING_BANK = pc() >> 16

HelloWorld:
    db  "Hello World!\n", 0


namespace Resources {

Palette:
    dw  ToPalette(0, 0, 0)
    dw  ToPalette(31, 31, 31)
constant Palette.size = pc() - Palette
}

finalizeMemory()

