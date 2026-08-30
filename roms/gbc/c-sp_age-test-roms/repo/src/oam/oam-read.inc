; Test OAM read access
;
IF !DEF(INC_OAM_READ)
DEF INC_OAM_READ EQU 1

IF DEF(CGB_E)
    DEF EFF EQU $FF
ELSE
    DEF EFF EQU $00
ENDC

INCLUDE "oam/read.inc"



EXPECTED_TEST_RESULTS_DMG:
    DB 8
    DB $FF, $FF, $FF, $FF, $FF, $00, $00, $00
    DB $FF, $FF, $FF, $FF, $FF, $00, $00, $00
    DB $FF, $FF, $FF, $FF, $FF, $00, $00, $00
    DB $FF, $FF, $FF, $FF, $FF, $00, $00, $00
    DB $FF, $FF, $FF, $FF, $FF, $00, $00, $00
    DB $FF, $FF, $FF, $FF, $FF, $00, $00, $00
    DB $FF, $FF, $FF, $FF, $FF, $00, $00, $00
    DB $FF, $FF, $FF, $FF, $FF, $00, $00, $00

EXPECTED_TEST_RESULTS_CGB:
    DB 16
    ; single speed
    DB $FF, $FF, $FF, $FF, $FF, $00, $00, $00
    DB $FF, $FF, $FF, $FF, $FF, $00, $00, $00
    DB $FF, $FF, $FF, $FF, $FF, $00, $00, $00
    DB $FF, $FF, $FF, $FF, $FF, $00, $00, $00
    DB $FF, $FF, $FF, $FF, $FF, $00, $00, $00
    DB $FF, $FF, $FF, $FF, $FF, $00, $00, $00
    DB $FF, $FF, $FF, $FF, $FF, $00, $00, $00
    DB $FF, $FF, $FF, $FF, $FF, $00, $00, $00
    ; double speed
    DB $FF, $FF, $FF, $FF, $FF, $FF, $FF, $00
    DB $FF, $FF, $FF, $FF, $FF, $FF, $FF, $00
    DB $FF, $FF, $FF, $FF, $FF, $FF, $FF, $00
    DB $FF, $FF, $FF, $FF, $FF, $FF, $FF, $00
    DB $FF, $FF, $FF, $FF, $FF, $FF, $FF, $00
    DB $FF, $FF, $FF, $FF, $FF, $FF, $FF, $00
    DB $FF, $FF, $FF, $FF, $FF, $FF, $FF, $00
    DB $FF, $FF, $FF, $FF, $FF, $FF, $FF, $00

EXPECTED_TIMING_RESULTS:
    ;
    ; single speed
    ;
    ; line 0, 1, 143, 144                       line 0, 1 (next frame)
    DB $00, $FF, $00, $FF, $00, $FF, $00, $00,  $00, $FF, $00, $FF ; SCX 0
    DB $00, $FF, $00, $FF, $00, $FF, $00, $00,  $00, $FF, $00, $FF
    DB $FF, $00, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    DB $FF, $00, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    DB $FF, $00, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    ; line 0, 1, 143, 144                       line 0, 1 (next frame)
    DB $00, $FF, $00, $FF, $00, $FF, $00, $00,  $00, $FF, $00, $FF ; SCX 1
    DB $00, $FF, $00, $FF, $00, $FF, $00, $00,  $00, $FF, $00, $FF
    DB $FF, EFF, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    DB $FF, $00, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    DB $FF, $00, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    ; line 0, 1, 143, 144                       line 0, 1 (next frame)
    DB $00, $FF, $00, $FF, $00, $FF, $00, $00,  $00, $FF, $00, $FF ; SCX 2
    DB $00, $FF, $00, $FF, $00, $FF, $00, $00,  $00, $FF, $00, $FF
    DB $FF, $FF, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    DB $FF, $00, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    DB $FF, $00, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    ; line 0, 1, 143, 144                       line 0, 1 (next frame)
    DB $00, $FF, $00, $FF, $00, $FF, $00, $00,  $00, $FF, $00, $FF ; SCX 3
    DB $00, $FF, $00, $FF, $00, $FF, $00, $00,  $00, $FF, $00, $FF
    DB $FF, $FF, $FF, EFF, $FF, EFF, $00, $00,  $FF, EFF, $FF, EFF
    DB $FF, $00, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    DB $FF, $00, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    ; line 0, 1, 143, 144                       line 0, 1 (next frame)
    DB $00, $FF, $00, $FF, $00, $FF, $00, $00,  $00, $FF, $00, $FF ; SCX 4
    DB $00, $FF, $00, $FF, $00, $FF, $00, $00,  $00, $FF, $00, $FF
    DB $FF, $FF, $FF, $FF, $FF, $FF, $00, $00,  $FF, $FF, $FF, $FF
    DB $FF, $00, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    DB $FF, $00, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    ; line 0, 1, 143, 144                       line 0, 1 (next frame)
    DB $00, $FF, $00, $FF, $00, $FF, $00, $00,  $00, $FF, $00, $FF ; SCX 5
    DB $00, $FF, $00, $FF, $00, $FF, $00, $00,  $00, $FF, $00, $FF
    DB $FF, $FF, $FF, $FF, $FF, $FF, $00, $00,  $FF, $FF, $FF, $FF
    DB $FF, EFF, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    DB $FF, $00, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    ; line 0, 1, 143, 144                       line 0, 1 (next frame)
    DB $00, $FF, $00, $FF, $00, $FF, $00, $00,  $00, $FF, $00, $FF ; SCX 6
    DB $00, $FF, $00, $FF, $00, $FF, $00, $00,  $00, $FF, $00, $FF
    DB $FF, $FF, $FF, $FF, $FF, $FF, $00, $00,  $FF, $FF, $FF, $FF
    DB $FF, $FF, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    DB $FF, $00, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    ; line 0, 1, 143, 144                       line 0, 1 (next frame)
    DB $00, $FF, $00, $FF, $00, $FF, $00, $00,  $00, $FF, $00, $FF ; SCX 7
    DB $00, $FF, $00, $FF, $00, $FF, $00, $00,  $00, $FF, $00, $FF
    DB $FF, $FF, $FF, $FF, $FF, $FF, $00, $00,  $FF, $FF, $FF, $FF
    DB $FF, $FF, $FF, EFF, $FF, EFF, $00, $00,  $FF, EFF, $FF, EFF
    DB $FF, $00, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    ;
    ; double speed
    ;
    ; line 0, 1, 143, 144                       line 0, 1 (next frame)
    DB $00, $FF, $00, $FF, $00, $FF, $00, $00,  $00, $FF, $00, $FF ; SCX 0
    DB $00, $FF, EFF, $FF, EFF, $FF, $00, $00,  EFF, $FF, EFF, $FF
    DB $FF, $00, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    DB $FF, $00, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    DB $FF, $00, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    DB $FF, $00, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    DB $FF, $00, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    ; line 0, 1, 143, 144                       line 0, 1 (next frame)
    DB $00, $FF, $00, $FF, $00, $FF, $00, $00,  $00, $FF, $00, $FF ; SCX 1
    DB $00, $FF, EFF, $FF, EFF, $FF, $00, $00,  EFF, $FF, EFF, $FF
    DB $FF, $FF, $FF, $FF, $FF, $FF, $00, $00,  $FF, $FF, $FF, $FF
    DB $FF, $00, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    DB $FF, $00, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    DB $FF, $00, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    DB $FF, $00, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    ; line 0, 1, 143, 144                       line 0, 1 (next frame)
    DB $00, $FF, $00, $FF, $00, $FF, $00, $00,  $00, $FF, $00, $FF ; SCX 2
    DB $00, $FF, EFF, $FF, EFF, $FF, $00, $00,  EFF, $FF, EFF, $FF
    DB $FF, $FF, $FF, $FF, $FF, $FF, $00, $00,  $FF, $FF, $FF, $FF
    DB $FF, $00, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    DB $FF, $00, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    DB $FF, $00, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    DB $FF, $00, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    ; line 0, 1, 143, 144                       line 0, 1 (next frame)
    DB $00, $FF, $00, $FF, $00, $FF, $00, $00,  $00, $FF, $00, $FF ; SCX 3
    DB $00, $FF, EFF, $FF, EFF, $FF, $00, $00,  EFF, $FF, EFF, $FF
    DB $FF, $FF, $FF, $FF, $FF, $FF, $00, $00,  $FF, $FF, $FF, $FF
    DB $FF, $FF, $FF, $FF, $FF, $FF, $00, $00,  $FF, $FF, $FF, $FF
    DB $FF, $00, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    DB $FF, $00, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    DB $FF, $00, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    ; line 0, 1, 143, 144                       line 0, 1 (next frame)
    DB $00, $FF, $00, $FF, $00, $FF, $00, $00,  $00, $FF, $00, $FF ; SCX 4
    DB $00, $FF, EFF, $FF, EFF, $FF, $00, $00,  EFF, $FF, EFF, $FF
    DB $FF, $FF, $FF, $FF, $FF, $FF, $00, $00,  $FF, $FF, $FF, $FF
    DB $FF, $FF, $FF, $FF, $FF, $FF, $00, $00,  $FF, $FF, $FF, $FF
    DB $FF, $00, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    DB $FF, $00, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    DB $FF, $00, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    ; line 0, 1, 143, 144                       line 0, 1 (next frame)
    DB $00, $FF, $00, $FF, $00, $FF, $00, $00,  $00, $FF, $00, $FF ; SCX 5
    DB $00, $FF, EFF, $FF, EFF, $FF, $00, $00,  EFF, $FF, EFF, $FF
    DB $FF, $FF, $FF, $FF, $FF, $FF, $00, $00,  $FF, $FF, $FF, $FF
    DB $FF, $FF, $FF, $FF, $FF, $FF, $00, $00,  $FF, $FF, $FF, $FF
    DB $FF, $FF, $FF, $FF, $FF, $FF, $00, $00,  $FF, $FF, $FF, $FF
    DB $FF, $00, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    DB $FF, $00, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    ; line 0, 1, 143, 144                       line 0, 1 (next frame)
    DB $00, $FF, $00, $FF, $00, $FF, $00, $00,  $00, $FF, $00, $FF ; SCX 6
    DB $00, $FF, EFF, $FF, EFF, $FF, $00, $00,  EFF, $FF, EFF, $FF
    DB $FF, $FF, $FF, $FF, $FF, $FF, $00, $00,  $FF, $FF, $FF, $FF
    DB $FF, $FF, $FF, $FF, $FF, $FF, $00, $00,  $FF, $FF, $FF, $FF
    DB $FF, $FF, $FF, $FF, $FF, $FF, $00, $00,  $FF, $FF, $FF, $FF
    DB $FF, $00, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    DB $FF, $00, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00
    ; line 0, 1, 143, 144                       line 0, 1 (next frame)
    DB $00, $FF, $00, $FF, $00, $FF, $00, $00,  $00, $FF, $00, $FF ; SCX 7
    DB $00, $FF, EFF, $FF, EFF, $FF, $00, $00,  EFF, $FF, EFF, $FF
    DB $FF, $FF, $FF, $FF, $FF, $FF, $00, $00,  $FF, $FF, $FF, $FF
    DB $FF, $FF, $FF, $FF, $FF, $FF, $00, $00,  $FF, $FF, $FF, $FF
    DB $FF, $FF, $FF, $FF, $FF, $FF, $00, $00,  $FF, $FF, $FF, $FF
    DB $FF, $FF, $FF, $FF, $FF, $FF, $00, $00,  $FF, $FF, $FF, $FF
    DB $FF, $00, $FF, $00, $FF, $00, $00, $00,  $FF, $00, $FF, $00



timed_reads:
    ; read at the edge of line 0 mode 3
    DELAY 10
    ld a, [de]  ; 2 m-cycles
    ld [hl+], a ; 2 m-cycles
    ; read at the edge of line 0 mode 0
    DELAY 172 / 4 - 4
    ld a, [de]
    ld [hl+], a

    ; read at the edge of line 1 mode 2
    DELAY 46
    ld a, [de]
    ld [hl+], a
    ; read at the edge of line 1 mode 0
    DELAY (4 + 80 + 172) / 4 - 4
    ld a, [de]
    ld [hl+], a

    ; read at the edge of line 143 mode 2
    DELAY 46 + (141 * 456) / 4
    ld a, [de]
    ld [hl+], a
    ; read at the edge of line 143 mode 0
    DELAY (4 + 80 + 172) / 4 - 4
    ld a, [de]
    ld [hl+], a

    ; read during line 144 (was the edge of mode 2 for the lines before)
    DELAY 46
    ld a, [de]
    ld [hl+], a
    ; read during line 144 (was the edge of mode 0 for the lines before)
    DELAY (4 + 80 + 172) / 4 - 4
    ld a, [de]
    ld [hl+], a

    ; read at the edge of line 154 mode 2 (line 0 next frame)
    DELAY 46 + (9 * 456) / 4
    ld a, [de]
    ld [hl+], a
    ; read at the edge of line 154 mode 0 (line 0 next frame)
    DELAY (4 + 80 + 172) / 4 - 4
    ld a, [de]
    ld [hl+], a

    ; read at the edge of line 155 mode 2 (line 1 next frame)
    DELAY 46
    ld a, [de]
    ld [hl+], a
    ; read at the edge of line 155 mode 0 (line 1 next frame)
    DELAY (4 + 80 + 172) / 4 - 4
    ld a, [de]
    ld [hl+], a
    ret

timed_reads_ds:
    ; read at the edge of line 0 mode 3
    DELAY 30
    ld a, [de]        ; 2 m-cycles
    ld [hl+], a       ; 2 m-cycles
    ; read at the edge of line 0 mode 0
    DELAY 172 / 2 - 4 ; 82 m-cycles
    ld a, [de]        ; 2 m-cycles
    ld [hl+], a       ; 2 m-cycles

    ; read at the edge of line 1 mode 2
    DELAY 97
    ld a, [de]
    ld [hl+], a
    ; read at the edge of line 1 mode 0
    DELAY (80 + 172) / 2 - 4
    ld a, [de]
    ld [hl+], a

    ; read at the edge of line 143 mode 2
    DELAY 98 + (141 * 456) / 2
    ld a, [de]
    ld [hl+], a
    ; read at the edge of line 143 mode 0
    DELAY (80 + 172) / 2 - 4
    ld a, [de]
    ld [hl+], a

    ; read during line 144 (was the edge of mode 2 for the lines before)
    DELAY 98
    ld a, [de]
    ld [hl+], a
    ; read during line 144 (was the edge of mode 0 for the lines before)
    DELAY (80 + 172) / 2 - 4
    ld a, [de]
    ld [hl+], a

    ; read at the edge of line 154 mode 2 (line 0 next frame)
    DELAY 98 + (9 * 456) / 2
    ld a, [de]
    ld [hl+], a
    ; read at the edge of line 154 mode 0 (line 0 next frame)
    DELAY (80 + 172) / 2 - 4
    ld a, [de]
    ld [hl+], a

    ; read at the edge of line 155 mode 2 (line 1 next frame)
    DELAY 98
    ld a, [de]
    ld [hl+], a
    ; read at the edge of line 155 mode 0 (line 1 next frame)
    DELAY (80 + 172) / 2 - 4
    ld a, [de]
    ld [hl+], a
    ret

run_test:
    RUN_TEST _OAMRAM



ENDC ; !DEF(INC_OAM_READ)
