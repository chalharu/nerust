; Test LY timing for different LCD alignments.
;

INCLUDE "test-setup.inc"



timed_ly_reads_ns:
    ; read within line 0
    ld a, [de]  ; 2 m-cycles
    ld [hl+], a ; 2 m-cycles
    ; read on the edge of line 0/1
    DELAY (454 / 4) - 13
    ld a, [de]
    ld [hl+], a
    ; read on the edge of line 1/2
    DELAY (456 / 4) - 4
    ld a, [de]
    ld [hl+], a
    ; read on the edge of line 2/3
    DELAY (456 / 4) - 4
    ld a, [de]
    ld [hl+], a
    ; read on the edge of line 143/144
    DELAY (456 / 4) - 4 + (456 / 4) * 140
    ld a, [de]
    ld [hl+], a
    ; read on the edge of line 152/153
    DELAY (456 / 4) - 4 + (456 / 4) * 8
    ld a, [de]
    ld [hl+], a
    ; read on the edge of line 153/0
    DELAY (456 / 4) - 4
    ld a, [de]
    ld [hl+], a
    ; read on the edge of line 0/1
    DELAY (456 / 4) - 4
    ld a, [de]
    ld [hl+], a
    ret

timed_ly_reads_ds:
    ; read within line 0
    ld a, [de]  ; 2 m-cycles
    ld [hl+], a ; 2 m-cycles
    ; read on the edge of line 0/1
    DELAY (454 / 2) - 14
    ld a, [de]
    ld [hl+], a
    ; read on the edge of line 1/2
    DELAY (456 / 2) - 4
    ld a, [de]
    ld [hl+], a
    ; read on the edge of line 2/3
    DELAY (456 / 2) - 4
    ld a, [de]
    ld [hl+], a
    ; read on the edge of line 143/144
    DELAY (456 / 2) - 4 + (456 / 2) * 140
    ld a, [de]
    ld [hl+], a
    ; read on the edge of line 152/153
    DELAY (456 / 2) - 4 + (456 / 2) * 8
    ld a, [de]
    ld [hl+], a
    ; read on the edge of line 153/0
    DELAY (456 / 2) - 4
    ld a, [de]
    ld [hl+], a
    ; read on the edge of line 0/1
    DELAY (456 / 2) - 4
    ld a, [de]
    ld [hl+], a
    ret

MACRO TEST
    call lcd_off
    ld a, LCDCF_ON | LCDCF_BGON
    ldh [rLCDC], a
REPT \1
    SWITCH_SPEED
ENDR
    DELAY \2
IF ((\1) % 2) == 0
    call timed_ly_reads_ns
ELSE
    call timed_ly_reads_ds
    SWITCH_SPEED ; back to normal speed
ENDC
ENDM



run_test:
    ld hl, LY_VALUES
    ld de, rLY

    ; lcd alignment offset 0 - normal speed
    TEST 0, 0
    TEST 0, 1
    TEST 0, 2
    TEST 0, 3
    TEST 0, 4

    ; lcd alignment offset 1 - double speed
    TEST 1, (456 * 11 - 380) / 2
    TEST 1, (456 * 11 - 378) / 2
    TEST 1, (456 * 11 - 376) / 2
    TEST 1, (456 * 11 - 374) / 2
    TEST 1, (456 * 11 - 372) / 2
    ; lcd alignment offset 1 - normal speed
    TEST 2, (456 * 31 - 148) / 4
    TEST 2, (456 * 31 - 144) / 4
    TEST 2, (456 * 31 - 140) / 4
    TEST 2, (456 * 31 - 136) / 4
    TEST 2, (456 * 31 - 132) / 4

    ; lcd alignment offset 2 - double speed
    TEST 3, (456 * 41 - 74) / 2
    TEST 3, (456 * 41 - 72) / 2
    TEST 3, (456 * 41 - 70) / 2
    TEST 3, (456 * 41 - 68) / 2
    TEST 3, (456 * 41 - 66) / 2
    ; lcd alignment offset 2 - normal speed
    TEST 4, (456 * 62 - 300) / 4
    TEST 4, (456 * 62 - 296) / 4
    TEST 4, (456 * 62 - 292) / 4
    TEST 4, (456 * 62 - 288) / 4
    TEST 4, (456 * 62 - 284) / 4

    ; lcd alignment offset 3 - double speed
    TEST 5, (456 * 72 - 222) / 2
    TEST 5, (456 * 72 - 220) / 2
    TEST 5, (456 * 72 - 218) / 2
    TEST 5, (456 * 72 - 216) / 2
    TEST 5, (456 * 72 - 114) / 2
    ; lcd alignment offset 3 - normal speed
    TEST 6, (456 * 93 - 448) / 4
    TEST 6, (456 * 93 - 444) / 4
    TEST 6, (456 * 93 - 440) / 4
    TEST 6, (456 * 93 - 436) / 4
    TEST 6, (456 * 93 - 432) / 4

    ; lcd alignment offset 4 - double speed
    TEST 7, (456 * 103 - 372) / 2
    TEST 7, (456 * 103 - 370) / 2
    TEST 7, (456 * 103 - 368) / 2
    TEST 7, (456 * 103 - 366) / 2
    TEST 7, (456 * 103 - 364) / 2
    ; lcd alignment offset 4 - normal speed
    TEST 8, (456 * 123 - 140) / 4
    TEST 8, (456 * 123 - 136) / 4
    TEST 8, (456 * 123 - 132) / 4
    TEST 8, (456 * 123 - 128) / 4
    TEST 8, (456 * 123 - 124) / 4

    ; compact LY values
    ld bc, LY_VALUES
    ld de, EXPECTED_LY_VALUES
    ld hl, TEST_RESULTS
    COMPACT_RESULTS_LINES 9, 5, 8
    ; finish test
    ld hl, EXPECTED_TEST_RESULTS
    ret



EXPECTED_TEST_RESULTS:
    DB 9
    DB $FF, $FF, $FF, $FF, $FF, $00, $00, $00
    DB $FF, $FF, $FF, $FF, $FF, $00, $00, $00
    DB $FF, $FF, $FF, $FF, $FF, $00, $00, $00
    DB $FF, $FF, $FF, $FF, $FF, $00, $00, $00
    DB $FF, $FF, $FF, $FF, $FF, $00, $00, $00
    DB $FF, $FF, $FF, $FF, $FF, $00, $00, $00
    DB $FF, $FF, $FF, $FF, $FF, $00, $00, $00
    DB $FF, $FF, $FF, $FF, $FF, $00, $00, $00
    DB $FF, $FF, $FF, $FF, $FF, $00, $00, $00

PUSHS
SECTION "ly-values", WRAM0
LY_VALUES: DS 9 * 5 * 8
POPS

IF DEF(CGB_E)
    DEF E99 EQU $99
ELSE
    DEF E99 EQU $00
ENDC

EXPECTED_LY_VALUES:
    ; lcd alignment offset 0 - normal speed
    DB $00, $00, $01, $02, $8F, $98, $00, $00
    DB $00, $01, $02, $03, $90, $99, $00, $01
    DB $00, $01, $02, $03, $90, E99, $00, $01
    DB $00, $01, $02, $03, $90, $00, $00, $01
    DB $00, $01, $02, $03, $90, $00, $00, $01
    ;
    ; lcd alignment offset 1 - double speed
    DB $00, $00, $01, $02, $8F, $98, $00, $00
    DB $00, $01, $02, $03, $90, $99, $00, $01
    DB $00, $01, $02, $03, $90, $99, $00, $01
    DB $00, $01, $02, $03, $90, $99, $00, $01
    DB $00, $01, $02, $03, $90, $00, $00, $01
    ; lcd alignment offset 1 - normal speed
    DB $00, $00, $01, $02, $8F, $98, $00, $00
    DB $00, $01, $02, $03, $90, $99, $00, $01
    DB $00, $01, $02, $03, $90, $00, $00, $01
    DB $00, $01, $02, $03, $90, $00, $00, $01
    DB $00, $01, $02, $03, $90, $00, $00, $01
    ;
    ; lcd alignment offset 2 - double speed
    DB $00, $00, $01, $02, $8F, $98, $00, $00
    DB $00, $00, $00, $02, $80, $98, $00, $00 ; glitch: LY & (LY + 1)
    DB $00, $01, $02, $03, $90, $99, $00, $01
    DB $00, $01, $02, $03, $90, $99, $00, $01
    DB $00, $01, $02, $03, $90, $00, $00, $01
    ; lcd alignment offset 2 - normal speed
    DB $00, $00, $01, $02, $8F, $98, $00, $00
IF DEF(CGB_E)
    DB $00, $00, $01, $02, $8F, $98, $00, $00
ELSE
    DB $00, $00, $00, $02, $80, $98, $00, $00 ; glitch: LY & (LY + 1)
ENDC
    DB $00, $01, $02, $03, $90, $99, $00, $01
    DB $00, $01, $02, $03, $90, $00, $00, $01
    DB $00, $01, $02, $03, $90, $00, $00, $01
    ;
    ; lcd alignment offset 3 - double speed
    DB $00, $00, $01, $02, $8F, $98, $00, $00
    DB $00, $01, $02, $03, $90, $99, $00, $01
    DB $00, $01, $02, $03, $90, $99, $00, $01
    DB $00, $01, $02, $03, $90, $99, $00, $01
    DB $00, $01, $02, $03, $90, $00, $00, $01
    ; lcd alignment offset 3 - normal speed
    DB $00, $00, $01, $02, $8F, $98, $00, $00
    DB $00, $00, $00, $02, $80, $98, $00, $00 ; glitch: LY & (LY + 1)
    DB $00, $01, $02, $03, $90, $99, $00, $01
    DB $00, $01, $02, $03, $90, $00, $00, $01
    DB $00, $01, $02, $03, $90, $00, $00, $01
    ;
    ; lcd alignment offset 4 - double speed
    DB $00, $00, $01, $02, $8F, $98, $00, $00
    DB $00, $00, $00, $02, $80, $98, $00, $00 ; glitch: LY & (LY + 1)
    DB $00, $01, $02, $03, $90, $99, $00, $01
    DB $00, $01, $02, $03, $90, $99, $00, $01
    DB $00, $01, $02, $03, $90, $00, $00, $01
    ; lcd alignment offset 4 - normal speed
    DB $00, $00, $01, $02, $8F, $98, $00, $00
    DB $00, $01, $02, $03, $90, $99, $00, $01
    DB $00, $01, $02, $03, $90, E99, $00, $01
    DB $00, $01, $02, $03, $90, $00, $00, $01
    DB $00, $01, $02, $03, $90, $00, $00, $01
