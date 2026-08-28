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
    DB $01, $06, $01, $07, $04, $01
    ; result row padding
    DB $00, $00



MACRO RUN_TEST
    ; prepare ie & if (pending v-blank interrupt)
    ld a, 1
    ldh [rIF], a
    ldh [rIE], a
    ; tma == "ret" instruction
    ld a, $C9
    ldh [rTMA], a
    ; stop any potentially running timer & reset div
    xor a
    ldh [rTAC], a
    ldh [rDIV], a

    ; wait for DIV == $76 (halt)
    DELAY $76 * $100 / 4
    ; start 65536 hz timer (div is incremented at 16384 hz)
    ld a, $05
    ldh [rTIMA], a
    ld a, $06
    ldh [rTAC], a
    ; prepare registers
    ld a, 1
    ld b, a
    ; delay until tima increment
    DELAY \1
    ; call:
    ;    [div] $76      halt
    ;   [tima] $06/$07  ld b, n / rlca
    ;    [tma] $c9      ret
    ;
    ; depending on the timing, this is executed either
    ;   as $06 $06  ld b, $06
    ;   or $06 $07  ld b, $07
    ;   or $07 $07  rlca rlca
    call rDIV

    ; save results
    ld [hl+], a
    ld a, b
    ld [hl+], a
ENDM

run_test:
    call lcd_off ; no regular v-blank interrupt
    ld hl, TEST_RESULTS

    RUN_TEST 9
    RUN_TEST 10
    RUN_TEST 11

    ld hl, EXPECTED_TEST_RESULTS
    ret
