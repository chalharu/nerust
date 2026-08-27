; Test rDIV timing when switching between CGB speeds.
; The number of machine cycles until the first DIV increment
; after switching speeds is always the same.
; rDIV always reads 0 (zero) right after speed switching.
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
    ; number of test result rows
    DB 5
    ; double speed
    DB $00, $01,  $00, $01,  $00, $01,  $00, $01
    ; double speed -> single speed
    DB $00, $01,  $00, $01,  $00, $01,  $00, $01
    DB $00, $01,  $00, $01,  $00, $01,  $00, $01
    ; double speed -> single speed -> double speed
    DB $00, $01,  $00, $01,  $00, $01,  $00, $01
    DB $00, $01,  $00, $01,  $00, $01,  $00, $01



MACRO SAVE_DIV
    ldh a, [rDIV]
    ld [hl+], a
ENDM

MACRO TEST_DS
    ldh [rDIV], a ; reset div
    DELAY \1      ; wait before switching to double speed
    SWITCH_SPEED  ; switch to double speed
    DELAY \2      ; wait before reading rDIV
    SAVE_DIV      ; read rDIV
    SWITCH_SPEED  ; switch to single speed
ENDM

MACRO TEST_DS_SS
    ldh [rDIV], a ; reset div
    SWITCH_SPEED  ; switch to double speed
    DELAY \1      ; wait before switching to single speed
    SWITCH_SPEED  ; switch to single speed
    DELAY \2      ; wait before reading rDIV
    SAVE_DIV      ; read rDIV
ENDM

MACRO TEST_DS_SS_DS
    ldh [rDIV], a ; reset div
    SWITCH_SPEED  ; switch to double speed
    DELAY \1      ; wait before switching to single speed
    SWITCH_SPEED  ; switch to single speed
    DELAY \2      ; wait before switching to double speed
    SWITCH_SPEED  ; switch to double speed
    DELAY \3      ; wait before reading rDIV
    SAVE_DIV      ; read rDIV
    SWITCH_SPEED  ; switch to single speed
ENDM



run_test:
    call lcd_off
    SOUND_OFF
    ld hl, TEST_RESULTS

    ; double speed

    TEST_DS $00, $3C
    TEST_DS $00, $3D
    TEST_DS $01, $3C
    TEST_DS $01, $3D
    TEST_DS $C0, $3C
    TEST_DS $C0, $3D
    TEST_DS $C1, $3C
    TEST_DS $C1, $3D

    ; double speed -> single speed

    TEST_DS_SS $00, $3C
    TEST_DS_SS $00, $3D
    TEST_DS_SS $01, $3C
    TEST_DS_SS $01, $3D
    TEST_DS_SS $02, $3C
    TEST_DS_SS $02, $3D
    TEST_DS_SS $03, $3C
    TEST_DS_SS $03, $3D

    TEST_DS_SS $C0, $3C
    TEST_DS_SS $C0, $3D
    TEST_DS_SS $C1, $3C
    TEST_DS_SS $C1, $3D
    TEST_DS_SS $C2, $3C
    TEST_DS_SS $C2, $3D
    TEST_DS_SS $C3, $3C
    TEST_DS_SS $C3, $3D

    ; double speed -> single speed -> double speed

    TEST_DS_SS_DS $00, $00, $3C
    TEST_DS_SS_DS $00, $00, $3D
    TEST_DS_SS_DS $01, $00, $3C
    TEST_DS_SS_DS $01, $00, $3D
    TEST_DS_SS_DS $02, $00, $3C
    TEST_DS_SS_DS $02, $00, $3D
    TEST_DS_SS_DS $03, $00, $3C
    TEST_DS_SS_DS $03, $00, $3D

    TEST_DS_SS_DS $00, $01, $3C
    TEST_DS_SS_DS $00, $01, $3D
    TEST_DS_SS_DS $01, $01, $3C
    TEST_DS_SS_DS $01, $01, $3D
    TEST_DS_SS_DS $02, $01, $3C
    TEST_DS_SS_DS $02, $01, $3D
    TEST_DS_SS_DS $03, $01, $3C
    TEST_DS_SS_DS $03, $01, $3D

    ld hl, EXPECTED_TEST_RESULTS
    ret
