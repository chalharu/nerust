; Verified:
;   2022-02-12 pass: CPU CGB E - CPU-CGB-06
;   2022-03-16 pass: CPU CGB C - CPU-CGB-04
;   2022-02-12 pass: CPU CGB B - CPU-CGB-02
;
INCLUDE "hardware.inc"
DEF CART_COMPATIBILITY EQU CART_COMPATIBLE_GBC
INCLUDE "test-setup.inc"

EXPECTED_TEST_RESULTS:
    ; number of test result rows
    DB 1
    ; result
    DB $00, $80, $04
    ; result padding
    DB $00, $00, $00, $00, $00

run_test:
    ld hl, TEST_RESULTS
    ld b, $03
    ld c, $04
    ; clear all interrupt flags
    xor a
    ldh [rIF], a
    ldh [rIE], a
    ; disable all user input
	ld a, $30
    ldh [rP1], a
    ; tma == "ret" instruction
    ld a, $C9
    ldh [rTMA], a


    ; Test OP code prefetching for STOP on CPU speed switch:
    ; is the next OP code to execute after STOP (after speed switch) being
    ; fetched before or after the CPU regains control?
    ;
    ; stop any potentially running timer & reset div
    xor a
    ldh [rTAC], a
    ldh [rDIV], a
    ; prepare speed switch
    ld a, 1
    ldh [rKEY1], a
    ; start 4 KHz timer
    ld a, TACF_START | TACF_4KHZ
    ldh [rTAC], a
    ; wait for div == $10
    DELAY $10 * $100 / 4
    ; set tima = 0
    xor a
    ldh [rTIMA], a
    ; call:
    ;    [div] $10        stop (trigger speed switch)
    ;   [tima] $00 / $80  nop / add b
    ;    [tma] $c9        ret
    call rDIV
    ; save test result
    ld [hl+], a
    ldh a, [rTIMA]
    ld [hl+], a
    ; back to normal speed
    SWITCH_SPEED


    ; Test OP code prefetching for STOP on CPU speed switch:
    ; is the next OP code to execute after STOP (after speed switch) being
    ; fetched on STOP's first machine cycle or one machine cycle later?
    ;
    ; stop timer & reset div
    xor a
    ldh [rTAC], a
    ldh [rDIV], a
    ; prepare speed switch
    ld a, 1
    ldh [rKEY1], a
    ; start 65 KHz timer (div is incremented at 16 KHz)
    ld a, TACF_START | TACF_65KHZ
    ldh [rTAC], a
    ; wait for div == $10
    DELAY ($10 * $100 - 28) / 4
    ; set tima = 0
    xor a
    ldh [rTIMA], a
    ; call:
    ;    [div] $10        stop (trigger speed switch)
    ;   [tima] $00 / $01  nop / rlc c
    ;    [tma] $c9        ret
    call rDIV
    ; save test result
    ld a, c
    ld [hl+], a
    ; back to normal speed
    SWITCH_SPEED


    ld hl, EXPECTED_TEST_RESULTS
    ret
