IF !DEF(INC_STAT_MODE_WINDOW)
DEF INC_STAT_MODE_WINDOW EQU 1

INCLUDE "test-setup.inc"

EXPECTED_LINE_STATS:
IF DEF(DOUBLE_SPEED)
    ; WX = 0
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+0
    DB $A0, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+1
    DB $A0, $A0, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+2
    DB $A0, $A0, $A0, $A0, $A3, $A3, $A3, $A3 ; NOPS+3
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A3, $A3 ; NOPS+4
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; NOPS+5
    ; WX = 1
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+0
    DB $A0, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+1
    DB $A0, $A0, $A0, $A3, $A3, $A3, $A3, $A3 ; NOPS+2
    DB $A0, $A0, $A0, $A0, $A0, $A3, $A3, $A3 ; NOPS+3
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A3 ; NOPS+4
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; NOPS+5
    ; WX = 2
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+0
    DB $A0, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+1
    DB $A0, $A0, $A0, $A3, $A3, $A3, $A3, $A3 ; NOPS+2
    DB $A0, $A0, $A0, $A0, $A0, $A3, $A3, $A3 ; NOPS+3
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A3 ; NOPS+4
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; NOPS+5
    ; WX = 3
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+0
    DB $A0, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+1
    DB $A0, $A0, $A0, $A3, $A3, $A3, $A3, $A3 ; NOPS+2
    DB $A0, $A0, $A0, $A0, $A0, $A3, $A3, $A3 ; NOPS+3
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A3 ; NOPS+4
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; NOPS+5
    ; WX = 4
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+0
    DB $A0, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+1
    DB $A0, $A0, $A0, $A3, $A3, $A3, $A3, $A3 ; NOPS+2
    DB $A0, $A0, $A0, $A0, $A0, $A3, $A3, $A3 ; NOPS+3
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A3 ; NOPS+4
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; NOPS+5
    ; WX = 5
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+0
    DB $A0, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+1
    DB $A0, $A0, $A0, $A3, $A3, $A3, $A3, $A3 ; NOPS+2
    DB $A0, $A0, $A0, $A0, $A0, $A3, $A3, $A3 ; NOPS+3
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A3 ; NOPS+4
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; NOPS+5
    ; WX = 6
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+0
    DB $A0, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+1
    DB $A0, $A0, $A0, $A3, $A3, $A3, $A3, $A3 ; NOPS+2
    DB $A0, $A0, $A0, $A0, $A0, $A3, $A3, $A3 ; NOPS+3
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A3 ; NOPS+4
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; NOPS+5
    ; WX = 7
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+0
    DB $A0, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+1
    DB $A0, $A0, $A0, $A3, $A3, $A3, $A3, $A3 ; NOPS+2
    DB $A0, $A0, $A0, $A0, $A0, $A3, $A3, $A3 ; NOPS+3
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A3 ; NOPS+4
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; NOPS+5
    ; WX = 8
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+0
    DB $A0, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+1
    DB $A0, $A0, $A0, $A3, $A3, $A3, $A3, $A3 ; NOPS+2
    DB $A0, $A0, $A0, $A0, $A0, $A3, $A3, $A3 ; NOPS+3
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A3 ; NOPS+4
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; NOPS+5
    ; WX = 9
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+0
    DB $A0, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+1
    DB $A0, $A0, $A0, $A3, $A3, $A3, $A3, $A3 ; NOPS+2
    DB $A0, $A0, $A0, $A0, $A0, $A3, $A3, $A3 ; NOPS+3
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A3 ; NOPS+4
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; NOPS+5
    ; WX = 162
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+0
    DB $A0, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+1
    DB $A0, $A0, $A0, $A3, $A3, $A3, $A3, $A3 ; NOPS+2
    DB $A0, $A0, $A0, $A0, $A0, $A3, $A3, $A3 ; NOPS+3
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A3 ; NOPS+4
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; NOPS+5
    ; WX = 163
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+0
    DB $A0, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+1
    DB $A0, $A0, $A0, $A3, $A3, $A3, $A3, $A3 ; NOPS+2
    DB $A0, $A0, $A0, $A0, $A0, $A3, $A3, $A3 ; NOPS+3
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A3 ; NOPS+4
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; NOPS+5
    ; WX = 164
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+0
    DB $A0, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+1
    DB $A0, $A0, $A0, $A3, $A3, $A3, $A3, $A3 ; NOPS+2
    DB $A0, $A0, $A0, $A0, $A0, $A3, $A3, $A3 ; NOPS+3
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A3 ; NOPS+4
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; NOPS+5
    ; WX = 165
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+0
    DB $A0, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+1
    DB $A0, $A0, $A0, $A3, $A3, $A3, $A3, $A3 ; NOPS+2
    DB $A0, $A0, $A0, $A0, $A0, $A3, $A3, $A3 ; NOPS+3
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A3 ; NOPS+4
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; NOPS+5
    ; WX = 166
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+0
    DB $A0, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+1
    DB $A0, $A0, $A0, $A3, $A3, $A3, $A3, $A3 ; NOPS+2
    DB $A0, $A0, $A0, $A0, $A0, $A3, $A3, $A3 ; NOPS+3
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A3 ; NOPS+4
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; NOPS+5
    ; WX = 167
    DB $A0, $A0, $A0, $A0, $A0, $A3, $A3, $A3 ; NOPS+0
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A3 ; NOPS+1
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; NOPS+2
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; NOPS+3
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; NOPS+4
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; NOPS+5
ELSE
    ; WX = 0
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+0
    DB $A0, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+1
    DB $A0, $A0, $A0, $A0, $A0, $A3, $A3, $A3 ; NOPS+2
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; NOPS+3
    ; WX = 1
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+0
    DB $A0, $A0, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+1
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A3, $A3 ; NOPS+2
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; NOPS+3
    ; WX = 2
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+0
    DB $A0, $A0, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+1
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A3, $A3 ; NOPS+2
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; NOPS+3
    ; WX = 3
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+0
    DB $A0, $A0, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+1
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A3, $A3 ; NOPS+2
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; NOPS+3
    ; WX = 4
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+0
    DB $A0, $A0, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+1
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A3, $A3 ; NOPS+2
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; NOPS+3
    ; WX = 5
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+0
    DB $A0, $A0, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+1
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A3, $A3 ; NOPS+2
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; NOPS+3
    ; WX = 6
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+0
    DB $A0, $A0, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+1
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A3, $A3 ; NOPS+2
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; NOPS+3
    ; WX = 7
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+0
    DB $A0, $A0, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+1
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A3, $A3 ; NOPS+2
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; NOPS+3
    ; WX = 8
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+0
    DB $A0, $A0, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+1
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A3, $A3 ; NOPS+2
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; NOPS+3
    ; WX = 9
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+0
    DB $A0, $A0, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+1
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A3, $A3 ; NOPS+2
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; NOPS+3
    ; WX = 162
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+0
    DB $A0, $A0, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+1
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A3, $A3 ; NOPS+2
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; NOPS+3
    ; WX = 163
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+0
    DB $A0, $A0, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+1
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A3, $A3 ; NOPS+2
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; NOPS+3
    ; WX = 164
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+0
    DB $A0, $A0, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+1
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A3, $A3 ; NOPS+2
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; NOPS+3
    ; WX = 165
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+0
    DB $A0, $A0, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+1
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A3, $A3 ; NOPS+2
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; NOPS+3
    ; WX = 166
IF DEF(CGB)
    DB $A3, $A3, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+0
    DB $A0, $A0, $A3, $A3, $A3, $A3, $A3, $A3 ; NOPS+1
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A3, $A3 ; NOPS+2
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; NOPS+3
ELSE
    DB $A0, $A0, $A0, $A0, $A3, $A3, $A3, $A3 ; NOPS+0
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; NOPS+1
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; NOPS+2
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; NOPS+3
ENDC
    ; WX = 167
    DB $A0, $A0, $A0, $A0, $A3, $A3, $A3, $A3 ; NOPS+0
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; NOPS+1
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; NOPS+2
    DB $A0, $A0, $A0, $A0, $A0, $A0, $A0, $A0 ; NOPS+3
ENDC

IF DEF(DOUBLE_SPEED)
    DEF NPS       EQU 115
    DEF NOP_LINES EQU 6
    DEF DSF       EQU $FF
ELSE
    DEF NPS       EQU 50
    DEF NOP_LINES EQU 4
    DEF DSF       EQU $00
ENDC

EXPECTED_TEST_RESULTS:
    ; number of test result rows
    DB 16
    ; WX 0 - 9
    DB $FF, $FF, $FF, $FF, DSF, DSF, $00, $00
    DB $FF, $FF, $FF, $FF, DSF, DSF, $00, $00
    DB $FF, $FF, $FF, $FF, DSF, DSF, $00, $00
    DB $FF, $FF, $FF, $FF, DSF, DSF, $00, $00
    DB $FF, $FF, $FF, $FF, DSF, DSF, $00, $00
    DB $FF, $FF, $FF, $FF, DSF, DSF, $00, $00
    DB $FF, $FF, $FF, $FF, DSF, DSF, $00, $00
    DB $FF, $FF, $FF, $FF, DSF, DSF, $00, $00
    DB $FF, $FF, $FF, $FF, DSF, DSF, $00, $00
    DB $FF, $FF, $FF, $FF, DSF, DSF, $00, $00
    ; WX 166 - 171
    DB $FF, $FF, $FF, $FF, DSF, DSF, $00, $00
    DB $FF, $FF, $FF, $FF, DSF, DSF, $00, $00
    DB $FF, $FF, $FF, $FF, DSF, DSF, $00, $00
    DB $FF, $FF, $FF, $FF, DSF, DSF, $00, $00
    DB $FF, $FF, $FF, $FF, DSF, DSF, $00, $00
    DB $FF, $FF, $FF, $FF, DSF, DSF, $00, $00

PUSHS
SECTION "line-stats", WRAM0
LINE_STATS: DS 16 * NOP_LINES * 8
POPS

run_test:
    call lcd_off
IF DEF(DOUBLE_SPEED)
    SWITCH_SPEED
ENDC
    ld a, $FF
    ldh [rLYC], a
    xor a, a
    ldh [rIF], a
    ldh [rWX], a
    ld a, 8
    ldh [rWY], a
    ld de, LINE_STATS
    ld hl, wait
    ld a, IEF_STAT
    ldh [rIE], a
    ld a, STATF_MODE10
    ldh [rSTAT], a
    ei
    ld a, LCDCF_ON | LCDCF_WINON
    ldh [rLCDC], a
nop_field:
    ; 11 lines at double speed
    NOPS 456 * 12 / 2
    ; there should have been an interrupt by now
    di
    ld hl, .NO_INT
    jp fail_test
.NO_INT:
    DB "  MODE2 INTERRUPT\n"
    DB "   NOT TRIGGERED", 0



MACRO READ_STAT_NOPS
    NOPS (\1)
    ldh a, [rSTAT]     ; 3 m-cycles
    ld [de], a         ; 2 m-cycles
    inc de             ; 2 m-cycles
    ldh a, [rLY]       ; 3 m-cycles
    cp a, (\2)         ; 2 m-cycles
    jp nz, next_m2_int ; 4 m-cycles for jump, 3 m-cycles else
    ld hl, (\3)        ; 3 m-cycles
    jp next_m2_int     ; 4 m-cycles
ENDM

PUSHS
SECTION "stat-interrupt-handler", ROM0[$48]
                  ; 5 m-cycles for interrupt handling
    ldh a, [rLY]  ; 3 m-cycles
    ldh [rSCX], a ; 3 m-cycles
    jp hl         ; 1 m-cycle
POPS

wait:
    cp a, 7
    jp nz, next_m2_int
    ld hl, read_stat_nops0
    jp next_m2_int

read_stat_nops0:
    READ_STAT_NOPS NPS + 0, 15, read_stat_nops1

read_stat_nops1:
    READ_STAT_NOPS NPS + 1, 23, read_stat_nops2

read_stat_nops2:
    READ_STAT_NOPS NPS + 2, 31, read_stat_nops3

IF DEF(DOUBLE_SPEED)
read_stat_nops3:
    READ_STAT_NOPS NPS + 3, 39, read_stat_nops4

read_stat_nops4:
    READ_STAT_NOPS NPS + 4, 47, read_stat_nops5

read_stat_nops5:
    READ_STAT_NOPS NPS + 5, 55, prepare_next_frame
ELSE
read_stat_nops3:
    READ_STAT_NOPS NPS + 3, 39, prepare_next_frame
ENDC

prepare_next_frame:
    ldh a, [rWX]
    cp a, 9
    jr z, .set_large_wx
    cp a, 167
    jp z, finish_test
    inc a
    ldh [rWX], a ; WX = 0 - 9, 163 - 167
    jr .now_wait
.set_large_wx:
    ld a, 162
    ldh [rWX], a ; WX = 162
.now_wait:
    ld hl, wait
    jp next_m2_int

next_m2_int:
    pop af        ; 3 m-cycles
    ei            ; 1 m-cycle
    jp nop_field  ; 4 m-cycles

finish_test:
    pop af
    ld bc, LINE_STATS
    ld de, EXPECTED_LINE_STATS
    ld hl, TEST_RESULTS
    COMPACT_RESULTS_LINES 16, NOP_LINES, 8
    ld hl, EXPECTED_TEST_RESULTS
    ret



ENDC ; !DEF(INC_STAT_MODE_WINDOW)
