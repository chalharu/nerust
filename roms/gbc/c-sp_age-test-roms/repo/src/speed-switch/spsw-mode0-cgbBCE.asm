; The LCD-to-CPU alignment can change when switching
; to double speed and back to single speed.
;
; Verified:
;   2021-10-21 pass: CPU CGB E - CPU-CGB-06
;   2022-03-16 pass: CPU CGB C - CPU-CGB-04
;   2021-10-21 pass: CPU CGB B - CPU-CGB-02
;   2021-10-21 fail: DMG-CPU C (blob) - DMG-CPU-08
;
INCLUDE "hardware.inc"
DEF CART_COMPATIBILITY EQU CART_COMPATIBLE_GBC
INCLUDE "test-setup.inc"



EXPECTED_TEST_RESULTS:
    DB 15
    ; single speed
    DB $02, $03, $03, $04, $83, $80, $83, $80
    DB $83, $80, $83, $80, $83, $80, $83, $80
    DB $83, $80, $83, $80, $83, $80, $83, $80
    ; double speed
    DB $0D, $0E, $0E, $0F, $83, $80, $83, $80
    DB $83, $80, $83, $80, $83, $80, $83, $80
    DB $83, $80, $83, $80, $83, $80, $83, $80
    ; single speed
    DB $0E, $0F, $0F, $10, $83, $80, $83, $80
    DB $83, $80, $83, $80, $83, $80, $83, $80
    DB $83, $80, $83, $80, $83, $80, $83, $80
    ; double speed
    DB $19, $1A, $1A, $1B, $83, $80, $83, $80
    DB $83, $80, $83, $80, $83, $80, $83, $80
    DB $83, $80, $83, $80, $83, $80, $83, $80
    ; single speed
    DB $1B, $1C, $1C, $1D, $83, $80, $83, $80
    DB $83, $80, $83, $80, $83, $80, $83, $80
    DB $83, $80, $83, $80, $83, $80, $83, $80



MACRO LY_TEST
    DELAY \1
    ld de, rLY   ; 3 m-cycles
    ld a, [de]   ; 2 m-cycles
    ld [hl+], a  ; 2 m-cycles
    ld a, [de]   ; 2 m-cycles
    ld [hl+], a  ; 2 m-cycles
    DELAY \2
    ld a, [de]   ; 2 m-cycles
    ld [hl+], a  ; 2 m-cycles
    ld a, [de]   ; 2 m-cycles
    ld [hl+], a  ; 2 m-cycles
ENDM

MACRO SCX_TEST
    ld a, \1      ; 2 m-cycles
    ldh [rSCX], a ; 3 m-cycles
    DELAY \2
    ld a, [de]    ; 2 m-cycles
    ld [hl+], a   ; 2 m-cycles
    DELAY \3
    ld a, [de]    ; 2 m-cycles
    ld [hl+], a   ; 2 m-cycles
ENDM



run_test:
    call lcd_off
    ld hl, TEST_RESULTS
    ld a, LCDCF_ON | LCDCF_BGON
    ldh [rLCDC], a

    LY_TEST 456 * 3 / 4 - 10, 456 / 4 - 5
    ld de, rSTAT
    SCX_TEST 0,  48, 456 / 4 - 3
    SCX_TEST 1, 104, 456 / 4 - 3
    SCX_TEST 2, 104, 456 / 4 - 3
    SCX_TEST 3, 104, 456 / 4 - 3
    SCX_TEST 4, 105, 456 / 4 - 3
    SCX_TEST 5, 104, 456 / 4 - 3
    SCX_TEST 6, 104, 456 / 4 - 3
    SCX_TEST 7, 104, 456 / 4 - 3
    SCX_TEST 8, 103, 456 / 4 - 3
    SCX_TEST 9, 104, 456 / 4 - 3

    ; switch to double speed
    SWITCH_SPEED
    LY_TEST 125, 456 / 2 - 5
    ld de, rSTAT
    SCX_TEST 0, 111, 456 / 2 - 3
    SCX_TEST 1, 219, 456 / 2 - 3
    SCX_TEST 2, 218, 456 / 2 - 3
    SCX_TEST 3, 219, 456 / 2 - 3
    SCX_TEST 4, 218, 456 / 2 - 3
    SCX_TEST 5, 219, 456 / 2 - 3
    SCX_TEST 6, 218, 456 / 2 - 3
    SCX_TEST 7, 219, 456 / 2 - 3
    SCX_TEST 8, 214, 456 / 2 - 3
    SCX_TEST 9, 219, 456 / 2 - 3

    ; switch to single speed
    ; TODO try this on different 2Mhz edges?
    ;      (currently on a %4 == 2 edge)
    SWITCH_SPEED
    LY_TEST 98, 456 / 4 - 5
    ld de, rSTAT
    SCX_TEST 0,  47, 456 / 4 - 3
    SCX_TEST 1, 105, 456 / 4 - 3
    SCX_TEST 2, 104, 456 / 4 - 3
    SCX_TEST 3, 104, 456 / 4 - 3
    SCX_TEST 4, 104, 456 / 4 - 3
    SCX_TEST 5, 105, 456 / 4 - 3
    SCX_TEST 6, 104, 456 / 4 - 3
    SCX_TEST 7, 104, 456 / 4 - 3
    SCX_TEST 8, 102, 456 / 4 - 3
    SCX_TEST 9, 105, 456 / 4 - 3

    ; switch to double speed
    SWITCH_SPEED
    LY_TEST 125, 456 / 2 - 5
    ld de, rSTAT
    SCX_TEST 0, 111, 456 / 2 - 3
    SCX_TEST 1, 218, 456 / 2 - 3
    SCX_TEST 2, 219, 456 / 2 - 3
    SCX_TEST 3, 218, 456 / 2 - 3
    SCX_TEST 4, 219, 456 / 2 - 3
    SCX_TEST 5, 218, 456 / 2 - 3
    SCX_TEST 6, 219, 456 / 2 - 3
    SCX_TEST 7, 218, 456 / 2 - 3
    SCX_TEST 8, 215, 456 / 2 - 3
    SCX_TEST 9, 218, 456 / 2 - 3

    ; switch to single speed
    ; TODO try this on different 2Mhz edges?
    ;      (currently on a %4 == 2 edge)
    SWITCH_SPEED
    ; TODO without the extra delay of "456 / 4"
    ;      my CGB-B reads "1A 1B 18(!) 1C" for some reason
    ;      (not my CGB-E though)
    LY_TEST 98 + 456 / 4, 456 / 4 - 5
    ld de, rSTAT
    SCX_TEST 0,  48, 456 / 4 - 3
    SCX_TEST 1, 104, 456 / 4 - 3
    SCX_TEST 2, 104, 456 / 4 - 3
    SCX_TEST 3, 104, 456 / 4 - 3
    SCX_TEST 4, 105, 456 / 4 - 3
    SCX_TEST 5, 104, 456 / 4 - 3
    SCX_TEST 6, 104, 456 / 4 - 3
    SCX_TEST 7, 104, 456 / 4 - 3
    SCX_TEST 8, 103, 456 / 4 - 3
    SCX_TEST 9, 104, 456 / 4 - 3

    ld hl, EXPECTED_TEST_RESULTS
    ret
