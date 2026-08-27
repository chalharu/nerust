; STAT interrupt (interrupt $02) timing is measured.
; Interrupt enable/disable edge cases are ignored here.
;
; TODO rewrite this with less LCD on/off switching
;
IF !DEF(INC_STAT_INT)
DEF INC_STAT_INT EQU 1

INCLUDE "test-setup.inc"



EXPECTED_TEST_RESULTS_DMG:
    ; number of test result rows
    DB 5
    ; mode 0 (line >0)
    DB $80, $82, $80, $82, $80, $82, $80, $82 ; SCX 0 - 3
    DB $80, $82, $80, $82, $80, $82, $80, $82 ; SCX 4 - 7
    DB $80, $82, $80, $82, $00, $00, $00, $00 ; SCX 8 - 9
    ; mode 2 (line >0)
    DB $80, $82, $80, $82, $00, $00, $00, $00 ; SCX 0, 7
    ; mode 1
    DB $80, $82, $80, $82, $00, $00, $00, $00 ; SCX 0, 7

EXPECTED_TEST_RESULTS_CGB:
    ; number of test result rows
    DB 10
    ;
    ; single speed
    ;
    ; mode 0 (line >0)
    DB $80, $82, $80, $82, $80, $82, $80, $82 ; SCX 0 - 3
    DB $80, $82, $80, $82, $80, $82, $80, $82 ; SCX 4 - 7
    DB $80, $82, $80, $82, $00, $00, $00, $00 ; SCX 8 - 9
    ; mode 2 (line >0)
    DB $80, $82, $80, $82, $00, $00, $00, $00 ; SCX 0, 7
    ; mode 1
    DB $80, $82, $80, $82, $00, $00, $00, $00 ; SCX 0, 7
    ;
    ; double speed
    ;
    ; mode 0 (line >0)
    DB $80, $82, $80, $82, $80, $82, $80, $82 ; SCX 0 - 3
    DB $80, $82, $80, $82, $80, $82, $80, $82 ; SCX 4 - 7
    DB $80, $82, $80, $82, $00, $00, $00, $00 ; SCX 8 - 9
    ; mode 2 (line >0)
    DB $80, $82, $80, $82, $00, $00, $00, $00 ; SCX 0, 7
    ; mode 1
    DB $80, $82, $80, $82, $00, $00, $00, $00 ; SCX 0, 7



PUSHS
SECTION "lcd-interrupt-handler", ROM0[$48]
lcd_stat_interupt_handler:
    ; turn off LCD interrupts for now
    xor a, a
    ldh [rSTAT], a
    ; just continue at hl,
    ; ignore the interrupted code and the call to wait_for_int
    add sp, 4
    jp hl
POPS

wait_for_int:
    ei
    ; Execute a bunch of nops that should be interrupted.
    ; We don't use the DELAY macro here as it:
    ;   - messes up the stack
    ;   - prevents interrupts from being served at 1 m-cycle edges
    NOPS 500
    di
    ret

; @param \1 scx
; @param \2 m-cycles before setting the rSTAT interrupt bit
; @param \3 the stat interrupt bit to set
; @param \4 m-cycles before reading rSTAT after interrupt
;
MACRO TEST_STAT_INT
    push hl
    ld hl, .read_stat\@

    call lcd_off
    ld a, \1
    ldh [rSCX], a
    xor a, a
    ldh [rIF], a
    ldh [rSTAT], a
    cpl
    ldh [rLYC], a
    ld a, IEF_STAT
    ldh [rIE], a
    ld a, LCDCF_ON | LCDCF_BGON
    ldh [rLCDC], a

    DELAY \2
    ld a, \3
    ldh [rSTAT], a
    call wait_for_int
    ; no interrupt -> fail
    xor a, a
    ldh [rSTAT], a
    ld a, $FF
    jp .save_result\@

.read_stat\@:
    DELAY \4
    ldh a, [rSTAT]
.save_result\@:
    pop hl
    ld [hl+], a
ENDM

MACRO PAD_RESULTS
    xor a
    REPT \1
        ld [hl+], a
    ENDR
ENDM



run_test:
    ld hl, TEST_RESULTS

    ; 230 m-cycles -> set rSTAT at the beginning of line 3 during mode 2
    TEST_STAT_INT 0, 230, STATF_MODE00, 34
    TEST_STAT_INT 0, 230, STATF_MODE00, 35
    TEST_STAT_INT 1, 230, STATF_MODE00, 34
    TEST_STAT_INT 1, 230, STATF_MODE00, 35
    TEST_STAT_INT 2, 230, STATF_MODE00, 34
    TEST_STAT_INT 2, 230, STATF_MODE00, 35
    TEST_STAT_INT 3, 230, STATF_MODE00, 33
    TEST_STAT_INT 3, 230, STATF_MODE00, 34
    TEST_STAT_INT 4, 230, STATF_MODE00, 33
    TEST_STAT_INT 4, 230, STATF_MODE00, 34
    TEST_STAT_INT 5, 230, STATF_MODE00, 33
    TEST_STAT_INT 5, 230, STATF_MODE00, 34
    TEST_STAT_INT 6, 230, STATF_MODE00, 33
    TEST_STAT_INT 6, 230, STATF_MODE00, 34
    TEST_STAT_INT 7, 230, STATF_MODE00, 32
    TEST_STAT_INT 7, 230, STATF_MODE00, 33
    TEST_STAT_INT 8, 230, STATF_MODE00, 34
    TEST_STAT_INT 8, 230, STATF_MODE00, 35
    TEST_STAT_INT 9, 230, STATF_MODE00, 34
    TEST_STAT_INT 9, 230, STATF_MODE00, 35
    PAD_RESULTS 4

    TEST_STAT_INT 0, 230, STATF_MODE10, 98
    TEST_STAT_INT 0, 230, STATF_MODE10, 99
    TEST_STAT_INT 7, 230, STATF_MODE10, 98
    TEST_STAT_INT 7, 230, STATF_MODE10, 99
    PAD_RESULTS 4

    TEST_STAT_INT 0, 230 + 140 * 456 / 4, STATF_MODE01, 1237
    TEST_STAT_INT 0, 230 + 140 * 456 / 4, STATF_MODE01, 1238
    TEST_STAT_INT 7, 230 + 140 * 456 / 4, STATF_MODE01, 1237
    TEST_STAT_INT 7, 230 + 140 * 456 / 4, STATF_MODE01, 1238
    PAD_RESULTS 4

    CP_CGB_FEATURES_USABLE
    jr z, .run_test_cgb
    ld hl, EXPECTED_TEST_RESULTS_DMG
    ret

.run_test_cgb:
    SWITCH_SPEED

    ; 460 m-cycles -> set rSTAT at the beginning of line 3 during mode 2
    TEST_STAT_INT 0, 460, STATF_MODE00, 85
    TEST_STAT_INT 0, 460, STATF_MODE00, 86
    TEST_STAT_INT 1, 460, STATF_MODE00, 84
    TEST_STAT_INT 1, 460, STATF_MODE00, 85
    TEST_STAT_INT 2, 460, STATF_MODE00, 84
    TEST_STAT_INT 2, 460, STATF_MODE00, 85
    TEST_STAT_INT 3, 460, STATF_MODE00, 83
    TEST_STAT_INT 3, 460, STATF_MODE00, 84
    TEST_STAT_INT 4, 460, STATF_MODE00, 83
    TEST_STAT_INT 4, 460, STATF_MODE00, 84
    TEST_STAT_INT 5, 460, STATF_MODE00, 82
    TEST_STAT_INT 5, 460, STATF_MODE00, 83
    TEST_STAT_INT 6, 460, STATF_MODE00, 82
    TEST_STAT_INT 6, 460, STATF_MODE00, 83
    TEST_STAT_INT 7, 460, STATF_MODE00, 81
    TEST_STAT_INT 7, 460, STATF_MODE00, 82
    TEST_STAT_INT 8, 460, STATF_MODE00, 85
    TEST_STAT_INT 8, 460, STATF_MODE00, 86
    TEST_STAT_INT 9, 460, STATF_MODE00, 84
    TEST_STAT_INT 9, 460, STATF_MODE00, 85
    PAD_RESULTS 4

    TEST_STAT_INT 0, 460, STATF_MODE10, 212
    TEST_STAT_INT 0, 460, STATF_MODE10, 213
    TEST_STAT_INT 7, 460, STATF_MODE10, 212
    TEST_STAT_INT 7, 460, STATF_MODE10, 213
    PAD_RESULTS 4

    TEST_STAT_INT 0, 460 + 140 * 456 / 2, STATF_MODE01, 2491
    TEST_STAT_INT 0, 460 + 140 * 456 / 2, STATF_MODE01, 2492
    TEST_STAT_INT 7, 460 + 140 * 456 / 2, STATF_MODE01, 2491
    TEST_STAT_INT 7, 460 + 140 * 456 / 2, STATF_MODE01, 2492
    PAD_RESULTS 4

    SWITCH_SPEED
    ld hl, EXPECTED_TEST_RESULTS_CGB
    ret



ENDC ; !DEF(INC_STAT_INT)
