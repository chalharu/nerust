IF !DEF(INC_MODE2_OBJ_SIZE)
DEF INC_MODE2_OBJ_SIZE EQU 1

INCLUDE "hardware.inc"
IF DEF(DOUBLE_SPEED)
    DEF CART_COMPATIBILITY EQU CART_COMPATIBLE_GBC
ELSE
    DEF CART_COMPATIBILITY EQU CART_COMPATIBLE_DMG_GBC
ENDC

DEF CART_IS_VISUAL_TEST EQU 1
INCLUDE "test-setup.inc"



MACRO RUN_TEST
    IF DEF(DOUBLE_SPEED)
        SWITCH_SPEED
    ENDC

    ; initialize tile data & OAM RAM
    call setup_gfx ; also turns off the lcd
    MEMCPY _OAMRAM, OBJ_DATA, OAM_COUNT * sizeof_OAM_ATTRS

    ; initialize frame counter
    ld a, 2
    ld [FRAME_COUNTER], a

    ; initialize CPU registers for rLCDC writing
    ld c, rLCDC & $FF
    ld d, LCDCF_ON | \1
    ld e, LCDCF_ON | \2

    ; activate v-blank interrupt
    xor a, a
    ldh [rIF], a
    ld a, IEF_VBLANK
    ldh [rIE], a

    ; switch on LCD and wait for interrupts
    ld a, d
    ldh [rLCDC], a

    DELAY 142 * 456 / M_CYCLE_SIZE
    ei
    jp nop_field
ENDM



nop_field:
    NOPS 512
    ld hl, .NO_INT
    jp fail_test
.NO_INT:
    DB " V-BLANK INTERRUPT\n"
    DB "   NOT TRIGGERED", 0



; ----------------------------------------------------------------------------
;
;   v-blank interrupt handler
;
; ----------------------------------------------------------------------------

PUSHS

SECTION "wram", WRAM0
FRAME_COUNTER: DS 1

SECTION "v-blank-interrupt-handler", ROM0[$40]
    jp vblank_handler

POPS

IF DEF(DOUBLE_SPEED)
    DEF M_CYCLE_SIZE EQU 2
    DEF LINE_DELAY EQU 184
    DEF WRITE_COUNT EQU 6
ELSE
    DEF M_CYCLE_SIZE EQU 4
    DEF LINE_DELAY EQU 88
    DEF WRITE_COUNT EQU 3
ENDC

MACRO LCDC_WRITES
.loop\@:
    REPT WRITE_COUNT
        ld a, \1   ; 1 m-cycle
        ldh [c], a ; 2 m-cycles
        ld a, \2   ; 1 m-cycle
        ldh [c], a ; 2 m-cycles
    ENDR
    ld a, \1   ; 1 m-cycle
    ldh [c], a ; 2 m-cycles
    DELAY LINE_DELAY
    dec h
    jp nz, .loop\@
ENDM

vblank_handler:
    ld h, 24
    ld l, 24
    ; delay until line 4 mode 2 begins
    DELAY 14 * 456 / M_CYCLE_SIZE - 16
    LCDC_WRITES d, e
    ld h, l
    LCDC_WRITES e, d

    ld a, [FRAME_COUNTER]
    or a, a
    jr z, .continue
    dec a
    ld [FRAME_COUNTER], a
    jr nz, .continue
    INDICATE_FINISHED_TEST
.continue:

    DELAY 91 * 456 / M_CYCLE_SIZE
    add sp, 2
    ei
    jp nop_field



; ----------------------------------------------------------------------------
;
;   object data
;
; ----------------------------------------------------------------------------

OBJ_DATA:
    ;   Y    X   Tile Flags
    DB $14, $08, $22, $00
    DB $14, $10, $22, $00
    DB $14, $18, $22, $00
    DB $14, $20, $22, $00
    DB $14, $28, $22, $00
    DB $14, $30, $22, $00
    DB $14, $38, $22, $00
    DB $14, $40, $22, $00
    DB $14, $48, $22, $00
    DB $14, $50, $22, $00
    ;   Y    X  Tile  Flags
    DB $1C, $58, $22, $00
    DB $1C, $60, $22, $00
    DB $1C, $68, $22, $00
    DB $1C, $70, $22, $00
    DB $1C, $78, $22, $00
    DB $1C, $80, $22, $00
    DB $1C, $88, $22, $00
    DB $1C, $90, $22, $00
    DB $1C, $98, $22, $00
    DB $1C, $A0, $22, $00
    ;   Y    X  Tile  Flags
    DB $2C, $08, $22, $00
    DB $2C, $10, $22, $00
    DB $2C, $18, $22, $00
    DB $2C, $20, $22, $00
    DB $2C, $28, $22, $00
    DB $2C, $30, $22, $00
    DB $2C, $38, $22, $00
    DB $2C, $40, $22, $00
    DB $2C, $48, $22, $00
    DB $2C, $50, $22, $00
    ;   Y    X  Tile  Flags
    DB $34, $58, $22, $00
    DB $34, $60, $22, $00
    DB $34, $68, $22, $00
    DB $34, $70, $22, $00
    DB $34, $78, $22, $00
    DB $34, $80, $22, $00
    DB $34, $88, $22, $00
    DB $34, $90, $22, $00
    DB $34, $98, $22, $00
    DB $34, $A0, $22, $00

ENDC ; !DEF(INC_MODE2_OBJ_SIZE)
