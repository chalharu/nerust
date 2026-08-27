IF !DEF(INC_STAT_MODE_SPRITES)
DEF INC_STAT_MODE_SPRITES EQU 1

INCLUDE "test-setup.inc"

IF DEF(DOUBLE_SPEED)
    DEF INITIAL_DELAY EQU 108
    DEF NUM_NOPS      EQU 8
ELSE
    DEF INITIAL_DELAY EQU 43
    DEF NUM_NOPS      EQU 5
ENDC



run_test:
    call lcd_off
IF DEF(DOUBLE_SPEED)
    SWITCH_SPEED
ENDC
    ; prepare OAM
    MEMCPY _OAMRAM, OAM_DATA, 16 * 4
    ; set test result destination
    ld hl, STAT_VALUES
    ; turn off LYC to prevent funny STAT values
    ld a, $FF
    ldh [rLYC], a
    ; prepare mode 2 interrupt handling
    ld de, wait_for_first_line
    ld bc, read_stat
    xor a, a
    ldh [rIF], a
    ld a, STATF_MODE10
    ldh [rSTAT], a
    ; enable STAT interrupt
    ld a, IEF_STAT
    ldh [rIE], a
    ei
    ; switch on LCD
    ld a, LCDCF_ON | LCDCF_OBJON
    ldh [rLCDC], a
nop_field:
    ; wait for the next interrupt
    ; (enough NOPs for 12 LCD lines at double speed)
    NOPS 456 * 12 / 2
    ; there should have been an interrupt by now
    di
    ld hl, .NO_INT
    jp fail_test
.NO_INT:
    DB "  MODE2 INTERRUPT\n"
    DB "   NOT TRIGGERED", 0



PUSHS
SECTION "stat-interrupt-handler", ROM0[$48]
    ldh a, [rLY]
    ldh [rSCX], a
    push de
    ret     ; jp de
POPS

wait_for_first_line:
    ; skip the first 7 LCD lines
    cp a, 7
    jp nz, next_m2_int
    ; switch to "read_stat" on line 8
    push bc ; current read_stat with N nops
    pop de
    dec bc  ; read_stat with N+1 NOPs on the next frame
    ; finish test after 8 NOPs
    ld a, read_stat & $FF
    sub a, c
    cp a, NUM_NOPS + 1
    jp nc, finish_test
    jp next_m2_int

read_stat_nops:
    nop
    nop
    nop
    nop
    nop
    nop
    nop
    nop
read_stat:
    NOPS INITIAL_DELAY
    ldh a, [rSTAT]
    ld [hl+], a
    ldh a, [rLY]
    cp a, 7 + 16 * 8
    jr nz, next_m2_int
    ld de, wait_for_first_line
next_m2_int:
    ; remove interrupt return address
    pop af
    ; wait for the next mode 2 interrupt
    ei
    jp nop_field

finish_test:
    ; remove interrupt return address
    pop af
    ; compact STAT values
    ld bc, STAT_VALUES
    ld de, EXPECTED_STAT_VALUES
    ld hl, TEST_RESULTS
    COMPACT_RESULTS_LINES NUM_NOPS * 2, 8, 8
    ; finish test
    ld hl, EXPECTED_TEST_RESULTS
    ret



OAM_DATA:
    ;   Y    X  tile  attributes
    DB $18, $00, $00, $00
    DB $20, $01, $00, $00
    DB $28, $02, $00, $00
    DB $30, $03, $00, $00
    DB $38, $04, $00, $00
    DB $40, $05, $00, $00
    DB $48, $06, $00, $00
    DB $50, $07, $00, $00
    ;   Y    X  tile  attributes
    DB $58, $08, $00, $00
    DB $60, $09, $00, $00
    DB $68, $0A, $00, $00
    DB $70, $0B, $00, $00
    DB $78, $0C, $00, $00
    DB $80, $0D, $00, $00
    DB $88, $0E, $00, $00
    DB $90, $0F, $00, $00

EXPECTED_TEST_RESULTS:
    ; number of test result rows
    DB NUM_NOPS * 2
    ; compacted results
REPT NUM_NOPS * 2
    DB $FF, $FF, $FF, $FF, $FF, $FF, $FF, $FF
ENDR

PUSHS
SECTION "line-stats", WRAM0
STAT_VALUES: DS 16 * 8 * 8
POPS

EXPECTED_STAT_VALUES:
IF DEF(DOUBLE_SPEED)
    ; NOPS + 0 (line cycle 256)
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  0
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  1
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  2
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  3
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  4
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  5
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  6
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  7
    ; NOPS + 0 (line cycle 256)
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  8
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  9
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX 10
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX 11
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX 12
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX 13
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX 14
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX 15

    ; NOPS + 1 (line cycle 258)
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  0
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  1
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  2
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  3
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  4
    DB $A0, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  5
    DB $A0, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  6
    DB $A0, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  7
    ; NOPS + 1 (line cycle 258)
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  8
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  9
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX 10
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX 11
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX 12
    DB $A0, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX 13
    DB $A0, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX 14
    DB $A0, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX 15

    ; NOPS + 2 (line cycle 260)
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  0
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  1
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  2
    DB $A0, $A0, $A0, $A3, $A3, $A3, $A3, $A3 ; SPX  3
    DB $A0, $A0, $A0, $A3, $A3, $A3, $A3, $A3 ; SPX  4
    DB $A0, $A0, $A0, $A3, $A3, $A3, $A3, $A3 ; SPX  5
    DB $A0, $A0, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  6
    DB $A0, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  7
    ; NOPS + 2 (line cycle 260)
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  8
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  9
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX 10
    DB $A0, $A0, $A0, $A3, $A3, $A3, $A3, $A3 ; SPX 11
    DB $A0, $A0, $A0, $A3, $A3, $A3, $A3, $A3 ; SPX 12
    DB $A0, $A0, $A0, $A3, $A3, $A3, $A3, $A3 ; SPX 13
    DB $A0, $A0, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX 14
    DB $A0, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX 15

    ; NOPS + 3 (line cycle 262)
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  0
    DB $A0, $A0, $A0, $A0, $A0, $A3, $A3, $A3 ; SPX  1
    DB $A0, $A0, $A0, $A0, $A0, $A3, $A3, $A3 ; SPX  2
    DB $A0, $A0, $A0, $A0, $A0, $A3, $A3, $A3 ; SPX  3
    DB $A0, $A0, $A0, $A0, $A3, $A3, $A3, $A3 ; SPX  4
    DB $A0, $A0, $A0, $A3, $A3, $A3, $A3, $A3 ; SPX  5
    DB $A0, $A0, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  6
    DB $A0, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  7
    ; NOPS + 3 (line cycle 262)
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  8
    DB $A0, $A0, $A0, $A0, $A0, $A3, $A3, $A3 ; SPX  9
    DB $A0, $A0, $A0, $A0, $A0, $A3, $A3, $A3 ; SPX 10
    DB $A0, $A0, $A0, $A0, $A0, $A3, $A3, $A3 ; SPX 11
    DB $A0, $A0, $A0, $A0, $A3, $A3, $A3, $A3 ; SPX 12
    DB $A0, $A0, $A0, $A3, $A3, $A3, $A3, $A3 ; SPX 13
    DB $A0, $A0, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX 14
    DB $A0, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX 15

    ; NOPS + 4 (line cycle 264)
    DB $A0, $A0, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  0
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A3 ; SPX  1
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A3, $A3 ; SPX  2
    DB $A0, $A0, $A0, $A0, $A0, $A3, $A3, $A3 ; SPX  3
    DB $A0, $A0, $A0, $A0, $A3, $A3, $A3, $A3 ; SPX  4
    DB $A0, $A0, $A0, $A3, $A3, $A3, $A3, $A3 ; SPX  5
    DB $A0, $A0, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  6
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A3 ; SPX  7
    ; NOPS + 4 (line cycle 264)
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A3 ; SPX  8
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A3 ; SPX  9
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A3, $A3 ; SPX 10
    DB $A0, $A0, $A0, $A0, $A0, $A3, $A3, $A3 ; SPX 11
    DB $A0, $A0, $A0, $A0, $A3, $A3, $A3, $A3 ; SPX 12
    DB $A0, $A0, $A0, $A3, $A3, $A3, $A3, $A3 ; SPX 13
    DB $A0, $A0, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX 14
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A3 ; SPX 15

    ; NOPS + 5 (line cycle 266)
    DB $A0, $A0, $A0, $A0, $A3, $A3, $A3, $A3 ; SPX  0
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A3 ; SPX  1
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A3, $A3 ; SPX  2
    DB $A0, $A0, $A0, $A0, $A0, $A3, $A3, $A3 ; SPX  3
    DB $A0, $A0, $A0, $A0, $A3, $A3, $A3, $A3 ; SPX  4
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX  5
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX  6
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX  7
    ; NOPS + 5 (line cycle 266)
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX  8
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A3 ; SPX  9
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A3, $A3 ; SPX 10
    DB $A0, $A0, $A0, $A0, $A0, $A3, $A3, $A3 ; SPX 11
    DB $A0, $A0, $A0, $A0, $A3, $A3, $A3, $A3 ; SPX 12
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX 13
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX 14
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX 15

    ; NOPS + 6 (line cycle 268)
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A3, $A3 ; SPX  0
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A3 ; SPX  1
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A3, $A3 ; SPX  2
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX  3
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX  4
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX  5
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX  6
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX  7
    ; NOPS + 6 (line cycle 268)
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX  8
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A3 ; SPX  9
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A3, $A3 ; SPX 10
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX 11
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX 12
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX 13
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX 14
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX 15

    ; NOPS + 7 (line cycle 270)
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX  0
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX  1
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX  2
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX  3
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX  4
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX  5
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX  6
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX  7
    ; NOPS + 7 (line cycle 270)
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX  8
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX  9
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX 10
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX 11
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX 12
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX 13
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX 14
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX 15
ELSE
    ; NOPS + 0 (line cycle 255)
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  0
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  1
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  2
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  3
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  4
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  5
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  6
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  7
    ; NOPS + 0 (line cycle 255)
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  8
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  9
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX 10
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX 11
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX 12
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX 13
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX 14
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX 15

    ; NOPS + 1 (line cycle 259)
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  0
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  1
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  2
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  3
    DB $A0, $A0, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  4
    DB $A0, $A0, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  5
    DB $A0, $A0, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  6
    DB $A0, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  7
    ; NOPS + 1 (line cycle 259)
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  8
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  9
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX 10
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX 11
    DB $A0, $A0, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX 12
    DB $A0, $A0, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX 13
    DB $A0, $A0, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX 14
    DB $A0, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX 15

    ; NOPS + 2 (line cycle 263)
    DB $A0, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  0
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A3, $A3 ; SPX  1
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A3, $A3 ; SPX  2
    DB $A0, $A0, $A0, $A0, $A0, $A3, $A3, $A3 ; SPX  3
    DB $A0, $A0, $A0, $A0, $A3, $A3, $A3, $A3 ; SPX  4
    DB $A0, $A0, $A0, $A3, $A3, $A3, $A3, $A3 ; SPX  5
    DB $A0, $A0, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  6
    DB $A0, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX  7
    ; NOPS + 2 (line cycle 263)
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A3, $A3 ; SPX  8
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A3, $A3 ; SPX  9
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A3, $A3 ; SPX 10
    DB $A0, $A0, $A0, $A0, $A0, $A3, $A3, $A3 ; SPX 11
    DB $A0, $A0, $A0, $A0, $A3, $A3, $A3, $A3 ; SPX 12
    DB $A0, $A0, $A0, $A3, $A3, $A3, $A3, $A3 ; SPX 13
    DB $A0, $A0, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX 14
    DB $A0, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; SPX 15

    ; NOPS + 3 (line cycle 267)
    DB $A0, $A0, $A0, $A0, $A0, $A3, $A3, $A3 ; SPX  0
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A3 ; SPX  1
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A3, $A3 ; SPX  2
    DB $A0, $A0, $A0, $A0, $A0, $A3, $A3, $A3 ; SPX  3
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX  4
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX  5
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX  6
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX  7
    ; NOPS + 3 (line cycle 267)
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX  8
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A3 ; SPX  9
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A3, $A3 ; SPX 10
    DB $A0, $A0, $A0, $A0, $A0, $A3, $A3, $A3 ; SPX 11
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX 12
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX 13
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX 14
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX 15

    ; NOPS + 4 (line cycle 271)
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX  0
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX  1
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX  2
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX  3
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX  4
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX  5
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX  6
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX  7
    ; NOPS + 4 (line cycle 271)
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX  8
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX  9
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX 10
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX 11
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX 12
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX 13
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX 14
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; SPX 15
ENDC

ENDC ; !DEF(INC_STAT_MODE_SPRITES)
