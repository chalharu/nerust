; Tests TIMA timing on CGB speed switch
;
IF !DEF(INC_SPSW_TIMA)
DEF INC_SPSW_TIMA EQU 1

IF DEF(CGB_E)
    DEF OFS EQU 1
ELSE
    DEF OFS EQU 0
ENDC

INCLUDE "test-setup.inc"



EXPECTED_TEST_RESULTS:
    ; number of test result rows
    DB 8
    ; TIMA & IF for 4KHz, 262KHz, 65KHz, 16KHz
    DB $80, $E0,  $04, $E4,  $01, $E4,  $00, $E4
    ; 4KHz TIMA (immediate) increment edges
    DB $80, $80, $81, $81,  $80, $80, $81, $81
    DB $80, $80, $81, $81,  $81, $81, $82, $82
    ; 4KHz TIMA 1->0 edge
    DB $81, $81, $82, $82,  $81, $81, $82, $82
    DB $81, $81, $82, $82,  $00, $00, $00, $00
    ; 262KHz TIMA (immediate) increment edges
    DB $04, $04, $05, $05,  $05, $05, $06, $06
    ; 65KHz TIMA (immediate) increment edges
    DB $01, $01, $02, $02,  $02, $02, $03, $03
    ; 16KHz TIMA (immediate) increment edges
    DB $00, $00, $01, $01,  $01, $01, $02, $02



MACRO SAVE_TIMA
    ldh a, [rTIMA]
    ld [hl+], a
ENDM



; Start the timer, Switch to double speed and
; immediately read TIMA & IF.
; During the speed switch the timer keeps running
; while the CPU is inactive for a while.
MACRO TEST_DS_IF
    TIMER_RESTART_CLEAN \1
    SWITCH_SPEED ; switch to double speed
    SAVE_TIMA
    ldh a, [rIF]
    ld [hl+], a
    SWITCH_SPEED ; switch to single speed
ENDM

; The DIV reset automatically triggered when switching
; speeds may cause immediate TIMA increments.
; For the 4KHz timer the edge for immediate increments
; is 1 m-cycle late.
; However, this 1 m-cycle delay only affects the
; 1->0 edge and not the 0->1 edge.
MACRO TEST_INC_EDGE
    TIMER_RESTART_CLEAN \3
    DELAY \1
    SWITCH_SPEED ; switch to double speed
    DELAY \2
    SAVE_TIMA

    TIMER_RESTART_CLEAN \3
    DELAY \1
    SWITCH_SPEED ; switch to single speed
    DELAY \2
    SAVE_TIMA
ENDM



run_test:
    call lcd_off
    SOUND_OFF
    ld hl, TEST_RESULTS

    TEST_DS_IF TACF_4KHZ
    TEST_DS_IF TACF_262KHZ
    TEST_DS_IF TACF_65KHZ
    TEST_DS_IF TACF_16KHZ

    TEST_INC_EDGE   0, 252, TACF_4KHZ ; immediately after TIMA start
    TEST_INC_EDGE   0, 253, TACF_4KHZ ; immediately after TIMA start
    TEST_INC_EDGE 109, 252, TACF_4KHZ ; 2 m-cycles before immediate increment by DIV reset
    TEST_INC_EDGE 109, 253, TACF_4KHZ ; 2 m-cycles before immediate increment by DIV reset
    TEST_INC_EDGE 110, 252, TACF_4KHZ ; 1 m-cycle before immediate increment by DIV reset
    TEST_INC_EDGE 110, 253, TACF_4KHZ ; 1 m-cycle before immediate increment by DIV reset
    TEST_INC_EDGE 111, 252, TACF_4KHZ ; trigger immediate increment by DIV reset
    TEST_INC_EDGE 111, 253, TACF_4KHZ ; trigger immediate increment by DIV reset

    TEST_INC_EDGE 237, 252, TACF_4KHZ ; 1 m-cycle before the 1->0 edge of the respective DIV bit
    TEST_INC_EDGE 237, 253, TACF_4KHZ ; 1 m-cycle before the 1->0 edge of the respective DIV bit
    TEST_INC_EDGE 238, 252, TACF_4KHZ ; right on the 1->0 edge of the respective DIV bit
    TEST_INC_EDGE 238, 253, TACF_4KHZ ; right on the 1->0 edge of the respective DIV bit
    TEST_INC_EDGE 239, 252, TACF_4KHZ ; 1 m-cycle after the 1->0 edge of the respective DIV bit
    TEST_INC_EDGE 239, 253, TACF_4KHZ ; 1 m-cycle after the 1->0 edge of the respective DIV bit
    inc hl ; result padding
    inc hl
    inc hl
    inc hl

    TEST_INC_EDGE 3, 0, TACF_262KHZ ; 1 m-cycle before immediate increment by DIV reset
    TEST_INC_EDGE 3, 1, TACF_262KHZ ; 1 m-cycle before immediate increment by DIV reset
    TEST_INC_EDGE 4, 0, TACF_262KHZ ; trigger immediate increment by DIV reset
    TEST_INC_EDGE 4, 1, TACF_262KHZ ; trigger immediate increment by DIV reset

    ; Immediate TIMA increments by DIV reset work as expected
    ; on CPU CGB B for the 65 KHz and the 16 KHz timer when
    ; switching speeds.
    ; On CPU CGB E they are delayed by one m-cycle.

    TEST_INC_EDGE 5 + OFS, 12, TACF_65KHZ ; 1 m-cycle before immediate increment by DIV reset
    TEST_INC_EDGE 5 + OFS, 13, TACF_65KHZ ; 1 m-cycle before immediate increment by DIV reset
    TEST_INC_EDGE 6 + OFS, 12, TACF_65KHZ ; trigger immediate increment by DIV reset
    TEST_INC_EDGE 6 + OFS, 13, TACF_65KHZ ; trigger immediate increment by DIV reset

    TEST_INC_EDGE 13 + OFS, 60, TACF_16KHZ ; 1 m-cycle before immediate increment by DIV reset
    TEST_INC_EDGE 13 + OFS, 61, TACF_16KHZ ; 1 m-cycle before immediate increment by DIV reset
    TEST_INC_EDGE 14 + OFS, 60, TACF_16KHZ ; trigger immediate increment by DIV reset
    TEST_INC_EDGE 14 + OFS, 61, TACF_16KHZ ; trigger immediate increment by DIV reset

    TIMER_OFF
    ld hl, EXPECTED_TEST_RESULTS
    ret



ENDC ; !DEF(INC_SPSW_TIMA)
