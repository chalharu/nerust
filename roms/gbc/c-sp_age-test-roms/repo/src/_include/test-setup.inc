IF !DEF(INC_TEST_SETUP)
DEF INC_TEST_SETUP EQU 1



; ----------------------------------------------------------------------------
;
;   The rom's entry point.
;   CGB flag and hardware compatibility is checked here.
;   The following flags can or have to be set by the code importing this file:
;
;       CART_COMPATIBILITY (required):
;           the cartridge's CGB flag
;
;       CART_IS_VISUAL_TEST (optional):
;           for 'visual' tests that comes with a screenshot
;
;       CART_REQUIRES_NON_CGB_MODE (optional):
;           for tests that are explicitly desinged to run in
;           non-CGB mode (will be verified).
;
; ----------------------------------------------------------------------------

IF !DEF(CART_COMPATIBILITY)
    FAIL "CART_COMPATIBILITY not defined"
ENDC

IF ((CART_COMPATIBILITY != CART_COMPATIBLE_GBC) \
    && (CART_COMPATIBILITY != CART_COMPATIBLE_DMG_GBC) \
    && (CART_COMPATIBILITY != CART_COMPATIBLE_DMG))
    FAIL STRFMT("invalid CART_COMPATIBILITY %#X", CART_COMPATIBILITY)
ENDC

IF ((CART_COMPATIBILITY != CART_COMPATIBLE_DMG) && DEF(CART_REQUIRES_NON_CGB_MODE))
    FAIL STRFMT("invalid CART_COMPATIBILITY %#X for CART_REQUIRES_NON_CGB_MODE", CART_COMPATIBILITY)
ENDC

SECTION "cart-cgb-flag", ROM0[$143]
DB CART_COMPATIBILITY



SECTION "entry", ROM0[$100]
    di ; interrupt handling should already be disabled, but just to be sure ...
    jp main

SECTION "main", ROM0[$150]
INCLUDE "utilities.inc"
main:
    call check_cgb_on_startup

IF DEF(CART_REQUIRES_NON_CGB_MODE)

    ; non-CGB mode required
    CP_IS_CGB
    jr z, .hardware_compatible
    ld hl, .FAIL_NON_CGB_MODE
    jp fail_test
.FAIL_NON_CGB_MODE:
    DB "   GAME BOY COLOR\n"
    DB "    REQUIRED FOR\n"
    DB "    NON-CGB MODE", 0

ELSE

    ; non-CGB mode not required,
    ; check cartridge compatibility
    CP_FAIL_CGB_REQUIRED
    jr nz, .hardware_compatible
    ld hl, .FAIL_CGB_REQUIRED
    jp fail_test
.FAIL_CGB_REQUIRED:
    DB "   GAME BOY COLOR\n"
    DB "      REQUIRED", 0

ENDC

.hardware_compatible:
    xor a, a
    ldh [rIE], a
    ldh [rIF], a



; ----------------------------------------------------------------------------
;
;   main (visual test)
;
;   - run_test has to be defined by the code that is including this file
;   - the test result is visible on screen only
;   - once the test result is 'stable',
;     INDICATE_FINISHED_TEST has to be executed
;   - checking for test success requires a separate screenshot
;     of the expecetd test result
;
; ----------------------------------------------------------------------------

IF DEF(CART_IS_VISUAL_TEST)
    jp run_test
ENDC



; ----------------------------------------------------------------------------
;
;   main (numerical test)
;
;   - run_test has to be defined by the code that is including this file
;   - run_test must return with hl pointing to the expected test results
;
; ----------------------------------------------------------------------------

IF !DEF(CART_IS_VISUAL_TEST)

PUSHS
SECTION "test-result", WRAM0
TEST_RESULTS: DS 16 * 8
POPS

    MEMSET TEST_RESULTS, 0, 16 * 8
    call run_test

    push hl ; address of expected test results in hl
    di
    call lcd_off
    MEMSET _SCRN0, 0, 18 * 32 ; clear tile map at $9800

    ld hl, TEST_SUCCESS
    ld [hl], 1

    ld bc, TEST_RESULTS
    pop de ; address of expected test results in de
    ld hl, _SCRN0 + 2 * 32

    ld a, [de] ; number of rows (8 bytes per row)
    inc de
    dec a
    cp a, 16 ; we expect 1-16 rows
    jr c, .valid_number_of_rows
    ld hl, TEST_SUCCESS
    ld [hl], 0
    jp show_final_screen

.valid_number_of_rows:
    inc a
    push af
.next_row:
    call .print_row
    pop af
    dec a
    jp z, show_final_screen ; all rows printed
    push af
    ld a, l
    and a, %11100000
    add a, 32
    ld l, a
    ld a, h
    adc a, 0
    ld h, a
    jr .next_row

.print_row:
    ld a, "0" - 32
    ld [hl+], a
    push bc
    ld a, c
    sub a, TEST_RESULTS & $FF
    ld b, 0 ; no font offset
    ld c, b ; no font offset
    call .print_byte
    pop bc
    xor a, a
    ld [hl+], a
.print_row_byte:
    ld a, [bc] ; next result byte
    inc bc
    push bc
    ld b, a
    ld a, [de] ; next expected result byte
    inc de
    cp a, b
    ld a, b ; a = current result byte
    jr z, .print_valid_byte

    push hl
    ld hl, TEST_SUCCESS
    ld [hl], 0
    pop hl

    ld b, 64  ; font-inverse
    ld c, 192 ; font-left-inverse
    call .print_byte
    jr .next_result_byte
.print_valid_byte:
    ld b, 0   ; font
    ld c, 128 ; font-left
    call .print_byte

.next_result_byte:
    pop bc
    ld a, l
    and a, %00011111
    cp a, 20
    jr c, .print_row_byte
    ret

.print_byte:
    push af
    push af
    swap a
    and a, $0F
    call .make_hex_char
    add a, b
    ld [hl+], a
    pop af
    and a, $0F
    call .make_hex_char
    add a, c
    ld [hl+], a
    pop af
    ret
.make_hex_char:
    cp a, 10
    jr nc, .hex_letter
    add a, "0" - 32
    ret
.hex_letter
    add a, "A" - 10 - 32
    ret

ENDC ; !DEF(CART_IS_VISUAL_TEST)



; ----------------------------------------------------------------------------
;
;   test utilities
;
; ----------------------------------------------------------------------------

; Indicate a finished test by executing a special no-op instruction.
;
MACRO INDICATE_FINISHED_TEST
    ld b, b
ENDM

PUSHS
SECTION "test-success", WRAM0
TEST_SUCCESS: DS 1
POPS



show_final_screen:
    call setup_gfx ; also turns off the lcd
    ld a, [TEST_SUCCESS]
    or a, a
    jr z, .test_failed
    ; print "test passed", indicate test success and freeze
    ld de, .TEST_PASSED
    call .print_and_show
    ld b, 3
    ld c, 5
    ld d, 8
    ld e, 13
    ld h, 21
    ld l, 34
    INDICATE_FINISHED_TEST
    jp freeze

.test_failed:
    ; print "test failed", indicate test failure and freeze
    ld de, .TEST_FAILED
    call .print_and_show
    ld b, 0
    INDICATE_FINISHED_TEST
    jp freeze

.print_and_show:
    ld hl, _SCRN0 + 4
    call print_ascii
    xor a, a
    ldh [rSCX], a
    ldh [rSCY], a
    ld a, LCDCF_ON | LCDCF_BG8000 | LCDCF_BGON
    ldh [rLCDC], a
    ret

.TEST_PASSED:
    DB "TEST PASSED!", 0
.TEST_FAILED:
    DB "TEST FAILED!", 0



fail_test:
    di
    call lcd_off
    ; clear tile map at $9800
    push hl
    MEMSET _SCRN0, 0, 18 * 32
    pop hl
    ; indicate test failure to show_final_screen
    xor a, a
    ld [TEST_SUCCESS], a
    ; print further details?
    ld a, h
    or a, l
    jp z, show_final_screen
    ; print further details!
    push hl
    pop de
    ld hl, _SCRN0 + 3 * 32
    call print_ascii
    jp show_final_screen



print_ascii:
    ld a, [de]
    or a, a
    ret z ; null-terminator found
    inc de
    cp a, "\n"
    jr nz, .check_char

    ; print newline
    ld a, l
    and a, %11100000
    add a, 32
    ld l, a
    ld a, h
    adc a, 0
    ld h, a
    jr print_ascii

    ; check & print character
.check_char:
    sub a, 32
    cp a, 64
    jr c, .print_char
    xor a, a ; replace invalid character by white space
.print_char:
    ld [hl+], a
    jr print_ascii



ENDC ; !DEF(INC_TEST_SETUP)
