IF !DEF(INC_READ)
DEF INC_READ EQU 1

INCLUDE "test-setup.inc"



DEF BYTES_PER_LINE EQU 12

PUSHS
SECTION "timing-results", WRAM0
TIMING_RESULTS: DS 16 * 8 * BYTES_PER_LINE
POPS

MACRO _TEST
    call lcd_off
    ld a, \1
    ld [rSCX], a
    ld a, LCDCF_ON | LCDCF_BGON
    ldh [rLCDC], a
    DELAY \3
    call \2
ENDM

compact_test_results:
    ld bc, TIMING_RESULTS
    ld de, EXPECTED_TIMING_RESULTS
    ld hl, TEST_RESULTS
    COMPACT_RESULTS_LINES 8, 5, BYTES_PER_LINE
    COMPACT_RESULTS_LINES 8, 7, BYTES_PER_LINE
    ret



MACRO RUN_TEST
    call lcd_off
    xor a, a
    ld hl, _OAMRAM
    ld [hl], a
    ld hl, _SCRN0
    ld [hl], a
    ld hl, TIMING_RESULTS
    ld de, \1

    FOR SCX, 8
        _TEST SCX, timed_reads, 0
        _TEST SCX, timed_reads, 1
        _TEST SCX, timed_reads, 2
        _TEST SCX, timed_reads, 3
        _TEST SCX, timed_reads, 4
    ENDR

    CP_CGB_FEATURES_USABLE
    jp z, .run_test_cgb\@
    call compact_test_results
    ld hl, EXPECTED_TEST_RESULTS_DMG
    ret

.run_test_cgb\@:
    SWITCH_SPEED

    FOR SCX, 8
        _TEST SCX, timed_reads_ds, 0
        _TEST SCX, timed_reads_ds, 1
        _TEST SCX, timed_reads_ds, 2
        _TEST SCX, timed_reads_ds, 3
        _TEST SCX, timed_reads_ds, 4
        _TEST SCX, timed_reads_ds, 5
        _TEST SCX, timed_reads_ds, 6
    ENDR

    SWITCH_SPEED
    call compact_test_results
    ld hl, EXPECTED_TEST_RESULTS_CGB
    ret
ENDM



ENDC ; !DEF(INC_READ)
