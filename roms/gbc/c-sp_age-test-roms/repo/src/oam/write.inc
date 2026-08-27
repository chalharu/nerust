IF !DEF(INC_WRITE)
DEF INC_WRITE EQU 1

INCLUDE "test-setup.inc"



DEF BYTES_PER_LINE EQU 12

PUSHS
SECTION "timing-results", WRAM0
TIMING_RESULTS: DS 16 * 8 * BYTES_PER_LINE
POPS

MACRO _TEST
    call prepare_memory

    push hl
    xor a, a
    ldh [rSTAT], a
    cpl
    ldh [rLYC], a
    ld a, \1
    ld [rSCX], a
    ld a, LCDCF_ON | LCDCF_BGON
    ldh [rLCDC], a
    DELAY \3
    call \2
    pop hl

    call lcd_off
    call copy_memory
ENDM

prepare_memory:
    push hl
    xor a, a
    ld c, BYTES_PER_LINE
.set_byte:
    ld [hl+], a
    dec c
    jr nz, .set_byte
    pop hl
    ret

copy_memory:
    push hl
    ld c, BYTES_PER_LINE
.copy_byte:
    ld a, [hl+]
    ld [de], a
    inc de
    dec c
    jr nz, .copy_byte
    pop hl
    ret

compact_test_results:
    ld bc, TIMING_RESULTS
    ld de, EXPECTED_TIMING_RESULTS
    ld hl, TEST_RESULTS
    COMPACT_RESULTS_LINES 8, 5, BYTES_PER_LINE
    COMPACT_RESULTS_LINES 8, 7, BYTES_PER_LINE
    ret



MACRO RUN_TEST
    call lcd_off
    ld de, TIMING_RESULTS
    ld hl, \1

    FOR SCX, 8
        _TEST SCX, timed_writes, 0
        _TEST SCX, timed_writes, 1
        _TEST SCX, timed_writes, 2
        _TEST SCX, timed_writes, 3
        _TEST SCX, timed_writes, 4
    ENDR

    CP_CGB_FEATURES_USABLE
    jp z, .run_test_cgb\@
    call compact_test_results
    ld hl, EXPECTED_TEST_RESULTS_DMG
    ret

.run_test_cgb\@:
    SWITCH_SPEED

    FOR SCX, 8
        _TEST SCX, timed_writes_ds, 0
        _TEST SCX, timed_writes_ds, 1
        _TEST SCX, timed_writes_ds, 2
        _TEST SCX, timed_writes_ds, 3
        _TEST SCX, timed_writes_ds, 4
        _TEST SCX, timed_writes_ds, 5
        _TEST SCX, timed_writes_ds, 6
    ENDR

    SWITCH_SPEED
    call compact_test_results
    ld hl, EXPECTED_TEST_RESULTS_CGB
    ret
ENDM



ENDC ; !DEF(INC_WRITE)
