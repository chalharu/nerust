; Verified:
;   2021-11-12: CPU CGB E - CPU-CGB-06 (non-CGB mode)
;   2022-03-17: CPU CGB C - CPU-CGB-04 (non-CGB mode)
;   2021-11-12: CPU CGB B - CPU-CGB-02 (non-CGB mode)
;   2021-11-12: DMG-CPU C (blob) - DMG-CPU-08
;
INCLUDE "hardware.inc"
DEF CART_COMPATIBILITY EQU CART_COMPATIBLE_DMG
DEF CART_IS_VISUAL_TEST EQU 1
INCLUDE "test-setup.inc"

INCLUDE "vblank-frame-counter.inc"


run_test:
    call lcd_off
    call setup_palettes
    ; clear vram
    MEMSET _VRAM8000, 0, $2000
    ; enable frame counter & mode 2 interrupt
    call activate_vblank_frame_counter
    ld a, STATF_MODE10
    ldh [rSTAT], a
    SET_IE_BIT IEF_STAT
    ; init mode 2 interrupt handler
    ld hl, no_op
    ld c, rBGP - $FF00
    ldh a, [c]
    ld b, a
    ; LCD on
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
    jp hl ; 1 m-cycle
POPS

MACRO MODIFY_BGP
    ldh a, [rLY]  ; 3 m-cycles
    sub a, \1     ; 2 m-cycles
    ldh [rSCX], a ; 3 m-cycles
    ld a, $FF     ; 2 m-cycles
    NOPS \2
    ldh [c], a    ; 2 m-cycles
    ld a, b       ; 1 m-cycle
    ldh [c], a    ; 2 m-cycles
    ldh a, [rLY]  ; 3 m-cycles
    cp a, (\1) + 7
    jp nz, continue
    ld hl, \3
    jp continue
ENDM

no_op:
    ldh a, [rLY]
    cp a, 7
    jp nz, continue
    ld hl, modify_bgp1
    jp continue

modify_bgp1:
    MODIFY_BGP 8, 4, modify_bgp2

modify_bgp2:
    MODIFY_BGP 16, 5, modify_bgp3

modify_bgp3:
    MODIFY_BGP 24, 6, modify_bgp4

modify_bgp4:
    MODIFY_BGP 32, 7, modify_bgp5

modify_bgp5:
    MODIFY_BGP 40, 8, modify_bgp6

modify_bgp6:
    MODIFY_BGP 48, 9, modify_bgp7

modify_bgp7:
    MODIFY_BGP 56, 10, modify_bgp8

modify_bgp8:
    MODIFY_BGP 64, 11, modify_bgp9

modify_bgp9:
    MODIFY_BGP 72, 12, no_op

continue:
    pop af
    ei
    jp nop_field
