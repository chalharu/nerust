; Test STAT mode timing.
; Mode 3 duration is measured for different SCX values but
; without any window or sprite rendering.
;
IF !DEF(INC_STAT_MODE)
DEF INC_STAT_MODE EQU 1

IF DEF(CGB_E)
    DEF M1E EQU $81 ; no mode-0 m-cycle at the end of mode-1 on CGB-E
ELSE
    DEF M1E EQU $80
ENDC

INCLUDE "stat-mode/stat-mode-utilities.inc"



EXPECTED_TEST_RESULTS:
    ; number of test result rows
    DB 14
    ; mode 0
    DB $FF, $FF, $FF, $00, $00, $00, $00, $00 ; SCX 0
    DB $FF, $FF, $FF, $00, $00, $00, $00, $00 ; SCX 1
    DB $FF, $FF, $FF, $00, $00, $00, $00, $00 ; SCX 2
    DB $FF, $FF, $FF, $00, $00, $00, $00, $00 ; SCX 3
    DB $FF, $FF, $FF, $00, $00, $00, $00, $00 ; SCX 4
    DB $FF, $FF, $FF, $00, $00, $00, $00, $00 ; SCX 5
    DB $FF, $FF, $FF, $00, $00, $00, $00, $00 ; SCX 6
    DB $FF, $FF, $FF, $00, $00, $00, $00, $00 ; SCX 7
    DB $FF, $FF, $FF, $00, $00, $00, $00, $00 ; SCX 8
    DB $FF, $FF, $FF, $00, $00, $00, $00, $00 ; SCX 9
    ; mode 1,2
    DB $FF, $FF, $FF, $00, $00, $00, $00, $00 ; SCX 0
    DB $FF, $FF, $FF, $00, $00, $00, $00, $00 ; SCX 7
    ; mode 3
    DB $FF, $FF, $00, $00, $00, $00, $00, $00 ; SCX 0
    DB $FF, $FF, $00, $00, $00, $00, $00, $00 ; SCX 7

EXPECTED_LINE_STATS:
    ; *includes lines of the next frame
    ;
    ; mode 0 beginning depends on SCX
    ; (and more, but this test is limited to SCX)
    ; -------------------------------------------
    ; line 0-3              line 142-145         line 152-155*
    DB $83, $83, $83, $83,  $83, $83, $81, $81,  $81, $81, $83, $83 ; SCX 0
    DB $80, $80, $80, $80,  $80, $80, $81, $81,  $81, $81, $80, $80
    DB $80, $80, $80, $80,  $80, $80, $81, $81,  $81, $81, $80, $80
    ; line 0-3              line 142-145         line 152-155*
    DB $83, $83, $83, $83,  $83, $83, $81, $81,  $81, $81, $83, $83 ; SCX 1
    DB $80, $80, $80, $80,  $80, $80, $81, $81,  $81, $81, $80, $80
    DB $80, $80, $80, $80,  $80, $80, $81, $81,  $81, $81, $80, $80
    ; line 0-3              line 142-145         line 152-155*
    DB $83, $83, $83, $83,  $83, $83, $81, $81,  $81, $81, $83, $83 ; SCX 2
    DB $83, $80, $80, $80,  $80, $80, $81, $81,  $81, $81, $80, $80
    DB $80, $80, $80, $80,  $80, $80, $81, $81,  $81, $81, $80, $80
    ; line 0-3              line 142-145         line 152-155*
    DB $83, $83, $83, $83,  $83, $83, $81, $81,  $81, $81, $83, $83 ; SCX 3
    DB $83, $80, $80, $80,  $80, $80, $81, $81,  $81, $81, $80, $80
    DB $80, $80, $80, $80,  $80, $80, $81, $81,  $81, $81, $80, $80
    ; line 0-3              line 142-145         line 152-155*
    DB $83, $83, $83, $83,  $83, $83, $81, $81,  $81, $81, $83, $83 ; SCX 4
    DB $83, $83, $83, $83,  $83, $83, $81, $81,  $81, $81, $83, $83
    DB $80, $80, $80, $80,  $80, $80, $81, $81,  $81, $81, $80, $80
    ; line 0-3              line 142-145         line 152-155*
    DB $83, $83, $83, $83,  $83, $83, $81, $81,  $81, $81, $83, $83 ; SCX 5
    DB $83, $83, $83, $83,  $83, $83, $81, $81,  $81, $81, $83, $83
    DB $80, $80, $80, $80,  $80, $80, $81, $81,  $81, $81, $80, $80
    ; line 0-3              line 142-145         line 152-155*
    DB $83, $83, $83, $83,  $83, $83, $81, $81,  $81, $81, $83, $83 ; SCX 6
    DB $83, $83, $83, $83,  $83, $83, $81, $81,  $81, $81, $83, $83
    DB $83, $80, $80, $80,  $80, $80, $81, $81,  $81, $81, $80, $80
    ; line 0-3              line 142-145         line 152-155*
    DB $83, $83, $83, $83,  $83, $83, $81, $81,  $81, $81, $83, $83 ; SCX 7
    DB $83, $83, $83, $83,  $83, $83, $81, $81,  $81, $81, $83, $83
    DB $83, $80, $80, $80,  $80, $80, $81, $81,  $81, $81, $80, $80
    ; line 0-3              line 142-145         line 152-155*
    DB $83, $83, $83, $83,  $83, $83, $81, $81,  $81, $81, $83, $83 ; SCX 8
    DB $80, $80, $80, $80,  $80, $80, $81, $81,  $81, $81, $80, $80
    DB $80, $80, $80, $80,  $80, $80, $81, $81,  $81, $81, $80, $80
    ; line 0-3              line 142-145         line 152-155*
    DB $83, $83, $83, $83,  $83, $83, $81, $81,  $81, $81, $83, $83 ; SCX 9
    DB $80, $80, $80, $80,  $80, $80, $81, $81,  $81, $81, $80, $80
    DB $80, $80, $80, $80,  $80, $80, $81, $81,  $81, $81, $80, $80
    ;
    ; mode 1,2 beginning does not depend on SCX
    ; -----------------------------------------
    ; line 0-3              line 142-145         line 152-155*
    DB $80, $80, $80, $80,  $80, $80, $81, $81,  $81, $81, $80, $80 ; SCX 0
    DB $80, $80, $80, $80,  $80, $80, $81, $81,  $81, M1E, $80, $80
    DB $82, $82, $82, $82,  $82, $81, $81, $81,  $81, $82, $82, $82
    ; line 0-3              line 142-145         line 152-155*
    DB $80, $80, $80, $80,  $80, $80, $81, $81,  $81, $81, $80, $80 ; SCX 7
    DB $80, $80, $80, $80,  $80, $80, $81, $81,  $81, M1E, $80, $80
    DB $82, $82, $82, $82,  $82, $81, $81, $81,  $81, $82, $82, $82
    ;
    ; mode 3 beginning does not depend on SCX
    ; ---------------------------------------
    ; line 0-3              line 142-145         line 152-155*
    DB $80, $82, $82, $82,  $82, $82, $81, $81,  $81, $81, $82, $82 ; SCX 0
    DB $83, $83, $83, $83,  $83, $83, $81, $81,  $81, $81, $83, $83
    ; line 0-3              line 142-145         line 152-155*
    DB $80, $82, $82, $82,  $82, $82, $81, $81,  $81, $81, $82, $82 ; SCX 7
    DB $83, $83, $83, $83,  $83, $83, $81, $81,  $81, $81, $83, $83

DEF M0_INITIAL_M_CYCLES  EQU ((80 + 172) / 4 - 12)
DEF M12_INITIAL_M_CYCLES EQU (456 / 4 - 13)
DEF M3_INITIAL_M_CYCLES  EQU (80 / 4 - 12)



run_test:
    ld hl, LINE_STATS
    FOR SCX, 10
        READ_LINES_STAT SCX, M0_INITIAL_M_CYCLES
        READ_LINES_STAT SCX, M0_INITIAL_M_CYCLES + 1
        READ_LINES_STAT SCX, M0_INITIAL_M_CYCLES + 2
    ENDR

    READ_LINES_STAT 0, M12_INITIAL_M_CYCLES
    READ_LINES_STAT 0, M12_INITIAL_M_CYCLES + 1
    READ_LINES_STAT 0, M12_INITIAL_M_CYCLES + 2
    READ_LINES_STAT 7, M12_INITIAL_M_CYCLES
    READ_LINES_STAT 7, M12_INITIAL_M_CYCLES + 1
    READ_LINES_STAT 7, M12_INITIAL_M_CYCLES + 2

    READ_LINES_STAT 0, M3_INITIAL_M_CYCLES
    READ_LINES_STAT 0, M3_INITIAL_M_CYCLES + 1
    READ_LINES_STAT 7, M3_INITIAL_M_CYCLES
    READ_LINES_STAT 7, M3_INITIAL_M_CYCLES + 1

    ld bc, LINE_STATS
    ld de, EXPECTED_LINE_STATS
    ld hl, TEST_RESULTS
    COMPACT_RESULTS_LINES 12, 3, BYTES_PER_LINE
    COMPACT_RESULTS_LINES 2, 2, BYTES_PER_LINE

    ld hl, EXPECTED_TEST_RESULTS
    ret



ENDC ; !DEF(INC_STAT_MODE)
