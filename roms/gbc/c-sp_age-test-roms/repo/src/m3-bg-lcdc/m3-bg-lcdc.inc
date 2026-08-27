IF !DEF(INC_M3_LCDC_BG)
DEF INC_M3_LCDC_BG EQU 1

DEF CART_IS_VISUAL_TEST EQU 1
INCLUDE "test-setup.inc"
INCLUDE "vblank-frame-counter.inc"

run_test:
    call lcd_off
    call setup_palettes
    ; clear tile data
    MEMSET _VRAM8000, 0, $1800
    ; fill tile map 1 ($9800 - $9BFF) with tile %01010101
    MEMSET _SCRN0, %01010101, $400
    ; fill tile map 2 ($9C00 - $9FFF) with tile %00001111
    MEMSET _SCRN1, %00001111, $400
    ; set two tile $2 lines around the "toggle-LCDC-area"
    ; for color comparison
    MEMSET _SCRN0 + 3 * SCRN_VX_B, 2, SCRN_VX_B
    MEMSET _SCRN0 + 12 * SCRN_VX_B, 2, SCRN_VX_B
    ; fill tile %01010101 (LCDC.4=0) with the darkest shade
    MEMSET _VRAM9000 + (%01010101 * $10), $FF, $10
    ; fill tile %00001111 (LCDC.4=0/1) with the darkest shade
    MEMSET _VRAM8000 + (%00001111 * $10), $FF, $10
    ; fill tile $2 (LCDC.4=1) with vertical lines of all 4 shades
    ld hl, _VRAM8000 + $20
    ld d, %00110011
    ld e, %00001111
    ld c, 8
.fill_tile2:
    ld a, d
    ld [hl+], a
    ld a, e
    ld [hl+], a
    dec c
    jr nz, .fill_tile2
    ; enable frame counter & mode 2 interrupt
    call activate_vblank_frame_counter
    ld a, STATF_MODE10
    ldh [rSTAT], a
    SET_IE_BIT IEF_STAT
    ; init mode 2 interrupt handler
    ld hl, no_op
    ld c, rLCDC - $FF00
    ; LCD on
IF DEF(DOUBLE_SPEED)
    SWITCH_SPEED
ENDC
    ld a, LCDCF_ON | LCDCF_BGON | LCDCF_BG9800 | LCDCF_BG8000
    ldh [rLCDC], a

nop_field:
    ; 11 lines at double speed
    NOPS 456 * 11 / 2
    ; there should have been an interrupt until now
    di
    ld hl, .NO_INT
    jp fail_test
.NO_INT:
    DB "  V-BLANK OR MODE2\n"
    DB "     INTERRUPT\n"
    DB "   NOT TRIGGERED", 0



PUSHS
SECTION "stat-interrupt-handler", ROM0[$48]
    jp on_mode2_interrupt
POPS

IF DEF(DOUBLE_SPEED)
    DEF LINE_NOPS EQU 28
    DEF REPETITIONS EQU 10
ELSE
    DEF LINE_NOPS EQU 3
    DEF REPETITIONS EQU 5
ENDC

on_mode2_interrupt:
    ldh a, [rLY]  ; 3 m-cycles
    srl a         ; 2 m-cycles
    ldh [rSCX], a ; 3 m-cycles
    NOPS LINE_NOPS
    jp hl         ; 1 m-cycle

no_op:
    ldh a, [rLY]
    ; start toggling the BG tile map on line 32
    cp a, 31
    jp nz, continue
    ld hl, toggle_bg_map
    ld b, LCDCF_BG9C00
    jp continue

toggle_bg_map:
    ; toggle the BG tile map a few times during this line
    ldh a, [c]     ; 2 m-cycles
    REPT REPETITIONS
        xor a, b  ; 1 m-cycle
        ldh [c], a ; 2 m-cycles
        nop       ; 1 m-cycle
        xor a, b  ; 1 m-cycle
        ldh [c], a ; 2 m-cycles
    ENDR
    ; start toggling the BG tile data area on line 64
    ldh a, [rLY]
    cp a, 63
    jp nz, continue
    ld hl, toggle_bg_data
    ld b, LCDCF_BG8000
    jp continue

toggle_bg_data:
    ; toggle the BG tile data area a few times during this line
    ldh a, [c]     ; 2 m-cycles
    REPT REPETITIONS
        xor a, b  ; 1 m-cycle
        ldh [c], a ; 2 m-cycles
        nop       ; 1 m-cycle
        xor a, b  ; 1 m-cycle
        ldh [c], a ; 2 m-cycles
    ENDR
    ; do nothing after line 95
    ldh a, [rLY]
    cp a, 95
    jp nz, continue
    ld hl, no_op
    jp continue

continue:
    pop af
    ei
    jp nop_field



ENDC ; !DEF(INC_M3_LCDC_BG)
