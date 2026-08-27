IF !DEF(INC_STAT_MODE_UTILITIES)
DEF INC_STAT_MODE_UTILITIES EQU 1

INCLUDE "test-setup.inc"



; TODO rewrite this with less LCD on/off switching



MACRO _READ_LINES_STAT
    STATIC_ASSERT (\2 == 4) || (\2 == 2)
    ld c, \1            ; 2 m-cycles
.loop\@:
    ldh a, [rSTAT]      ; 3 m-cycles
    ld [hl+], a         ; 2 m-cycles
    DELAY 456 / \2 - 10
    dec c               ; 1 m-cycle
    jp nz, .loop\@      ; 4 m-cycles for jump, 3 m-cycles else
    nop                 ; 1 m-cycle to make jp always use 4 m-cycles
ENDM

MACRO _READ_LINES_STAT_SPARSE
    _READ_LINES_STAT 4, \1 ; read first 4 lines

    DELAY (138 * 456) / \1 - 2 ; skip 138 lines
    _READ_LINES_STAT 4, \1     ; read 4 lines (142 - 145)

    DELAY (6 * 456) / \1 - 2 ; skip 6 lines
    _READ_LINES_STAT  4, \1  ; read 4 lines (152 - 155, includes 2 lines of the next frame)
ENDM

DEF BYTES_PER_LINE EQU 12

PUSHS
SECTION "line-stats", WRAM0
LINE_STATS: DS 16 * 8 * BYTES_PER_LINE
POPS

read_lines_stat:
    _READ_LINES_STAT_SPARSE 4
    ret

read_lines_stat_ds:
    _READ_LINES_STAT_SPARSE 2
    ret

PURGE _READ_LINES_STAT, _READ_LINES_STAT_SPARSE



MACRO _INIT_READ_LINE_STAT
    call lcd_off
    xor a, a
    ldh [rSTAT], a
    cpl
    ldh [rLYC], a
    ld a, \1
    ldh [rSCX], a
    ld a, LCDCF_ON | LCDCF_BGON
    ldh [rLCDC], a
ENDM

; @param \1 scx
; @param \2 initial delay (m-cycles)
;
MACRO READ_LINES_STAT
    _INIT_READ_LINE_STAT \1
    DELAY \2
    call read_lines_stat
ENDM

; @param \1 scx
; @param \2 initial delay (m-cycles)
;
MACRO READ_LINES_STAT_DS
    _INIT_READ_LINE_STAT \1
    DELAY \2
    call read_lines_stat_ds
ENDM



ENDC ; !DEF(INC_STAT_MODE_UTILITIES)
