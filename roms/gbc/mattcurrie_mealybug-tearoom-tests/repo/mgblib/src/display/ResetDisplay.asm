IF !DEF(INC_ResetDisplay)
INC_ResetDisplay = 1

INCLUDE "src/memory/MemSet.asm"
INCLUDE "src/memory/MemSetSmall.asm"
INCLUDE "src/display/LoadPalette.asm"

; Resets the display - VRAM, OAM and PPU registers.
; Sets a default background palette.
; Sprites are disabled.
;
; @destroys af, bc, hl
ResetDisplay::
    call LcdOffSafe

    xor a
    ld [rSTAT], a
    ld [rSCX], a
    ld [rSCY], a

    ; set window Y and LYC with an off-screen value so they have no effect
    ld a, 200
    ld [rLYC], a
    ld [rWY], a

    ; set a default palette
    ld a, $e4
    ld [rBGP], a

    ld hl, $0143
    bit 7, [hl]
    jr z, .notCGB

    ld a, $80
    ldh [rBCPS], a

    xor a
    ld hl, ShadesOfGrayPalette
    ld b, ShadesOfGrayPalette.end - ShadesOfGrayPalette
    call LoadBackgroundPalette

    ; select bank 1 of video ram
    ld a, 1
    ldh [rVBK], a

    xor a
    ld hl, _VRAM
    ld bc, $2000
    call MemSet

    ; select bank 0 of video ram
    xor a
    ldh [rVBK], a

.notCGB::

    xor a
    ld hl, _VRAM
    ld bc, $2000
    call MemSet

    xor a
    ld hl, _OAMRAM
    ld c, 160
    call MemSetSmall

    ld a, LCDCF_OFF | LCDCF_WIN9C00 | LCDCF_WINOFF | LCDCF_BG8800 | LCDCF_BG9800 | LCDCF_OBJ8 | LCDCF_OBJOFF | LCDCF_BGON
    ld [rLCDC], a

    ret

ShadesOfGrayPalette::
    rgb 31, 31, 31
    rgb 21, 21, 21
    rgb 11, 11, 11
    rgb 0, 0, 0    
.end:

ENDC
