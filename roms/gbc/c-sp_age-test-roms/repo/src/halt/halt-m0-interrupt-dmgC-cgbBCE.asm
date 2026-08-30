; Verified:
;   2022-01-21 pass: CPU CGB E - CPU-CGB-06
;   2022-03-16 pass: CPU CGB C - CPU-CGB-04
;   2022-01-21 pass: CPU CGB B - CPU-CGB-02
;   2022-01-21 pass: DMG-CPU C (blob) - DMG-CPU-08
;
INCLUDE "hardware.inc"
DEF CART_COMPATIBILITY EQU CART_COMPATIBLE_DMG_GBC
INCLUDE "test-setup.inc"

EXPECTED_TEST_RESULTS:
    ; number of test result rows
    DB 1
    ; results
    DB $02, $02, $02, $01, $01, $01, $01, $01

PUSHS
SECTION "lcd-stat-interrupt-handler", ROM0[$48]
    jp hl
POPS



run_test:
    call lcd_off
    xor a
    ldh [rIF], a
    ld a, IEF_STAT
    ldh [rIE], a
    ld a, STATF_MODE10
    ldh [rSTAT], a

    ld de, TEST_RESULTS
    ld hl, wait
    ld b, 0 ; scx in b

    ld a, LCDCF_ON
    ldh [rLCDC], a
    ei
    call nop_field

    ld hl, EXPECTED_TEST_RESULTS
    ret

nop_field:
    ; 11 lines at double speed
    NOPS 456 * 11 / 2
    ; there should have been an interrupt by now
    di
    ld hl, .NO_INT
    jp fail_test
.NO_INT:
    DB "  V-BLANK OR MODE2\n"
    DB "     INTERRUPT\n"
    DB "   NOT TRIGGERED", 0

wait:
    ldh a, [rLY]
    cp a, 10
    jp nz, next_m2_int
    ld hl, test_halt
    jp next_m2_int

test_halt:
    ld a, b
    ldh [rSCX], a
    ld a, STATF_MODE00
    ldh [rSTAT], a
    xor a
    ldh [rIF], a
    DELAY 44
    halt
    inc a
    ld [de], a
    inc de
    inc b
    ld a, b
    cp a, 8
    jp z, terminate_int_loop
    ld a, STATF_MODE10
    ldh [rSTAT], a
    xor a
    ldh [rIF], a
    jp next_m2_int

next_m2_int:
    pop af
    ei
    jp nop_field

terminate_int_loop:
    pop af
    ret
