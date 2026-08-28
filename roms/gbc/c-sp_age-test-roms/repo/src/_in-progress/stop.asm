INCLUDE "hardware.inc"
DEF CART_COMPATIBILITY EQU CART_COMPATIBLE_DMG_GBC
INCLUDE "test-setup.inc"

EXPECTED_TEST_RESULTS:
    ; number of test result rows
    DB 1
    ; results
    DB $10, $21, $00, $30, $00, $00, $00, $00

run_test:
    call setup_gfx
    ld bc, TEST_RESULTS

    ; show info screen & wait for pressed button
    ld hl, .test1_msg
    call show_info_screen
    ; reset the DIV & wait for DIV == $10
    xor a, a
    ldh [rDIV], a
    NOPS $10 * 256 / 4
    ldh a, [rDIV]
    ld [bc], a
    inc bc
    ; enter stop mode (terminated by d-pad)
    call enter_stop_mode
    ; check for skipped byte after stop
    ld [bc], a
    inc bc
    ; save DIV (should be zero, seems to be reset by STOP)
    ldh a, [rDIV]
    ld [bc], a
    inc bc
    ; next test
    jp test2
.test1_msg:
    DB "TEST 1", 0

test2:
    ; show info screen & wait for pressed button
    ld hl, .test2_msg
    call show_info_screen
    ; reset the DIV & wait for DIV == $30
    xor a, a
    ldh [rDIV], a
    NOPS $30 * 256 / 4
    ldh a, [rDIV]
    ld [bc], a
    inc bc
    ; enter stop mode (terminated by d-pad)
    call enter_stop_mode
    ; save DIV (should be zero, seems to be reset by STOP)
    ldh a, [rDIV]
    ld [bc], a
    inc bc
    ; done
    jp done
.test2_msg:
    DB "TEST 2", 0

done:
    ld hl, EXPECTED_TEST_RESULTS
    ret


enter_stop_mode:
    ld a, $20
    ldh [rP1], a
    stop
    inc a ; should be skipped as the byte after STOP is ignored
    ret

show_info_screen:
    push hl
    call lcd_off

    ld hl, _SCRN0
    ld de, .blank_screen_hint
    call print_ascii
    pop de
    call print_ascii
    ld de, .dot
    call print_ascii

    xor a, a
    ldh [rSCX], a
    ldh [rSCY], a
    ld a, LCDCF_ON | LCDCF_BG8000 | LCDCF_BGON
    ldh [rLCDC], a

.wait_for_button_down:
    call read_buttons
    or a
    jr z, .wait_for_button_down
.wait_for_button_up:
    call read_buttons
    or a
    jr nz, .wait_for_button_up
    ret

.blank_screen_hint:
    DB "\n"
    DB "\n"
    DB "  PLEASE PRESS ANY\n"
    DB "  BUTTON WHEN THE\n"
    DB "  SCREEN IS BLANK.\n"
    DB "\n"
    DB "\n"
    DB "PRESS ANY BUTTON TO\n"
    DB "    START ", 0
.dot:
    DB ".", 0


read_buttons:
    push bc
    ld a, $20 ; read d-pad
    ldh [rP1], a
    ld a, [rP1]
    ld a, [rP1]
    ld a, [rP1]
    ld a, [rP1]
    ld a, [rP1]
    ld a, [rP1]
    cpl
    and a, $0f
    ld b, a
    ld a, $10 ; read buttons
    ldh [rP1], a
    ld a, [rP1]
    ld a, [rP1]
    ld a, [rP1]
    ld a, [rP1]
    ld a, [rP1]
    ld a, [rP1]
    cpl
    and a, $0f
    swap a
    or a, b
    push af
    ld a, $30 ; clear lines
    ldh [rP1], a
    ld a, [rP1]
    ld a, [rP1]
    ld a, [rP1]
    ld a, [rP1]
    ld a, [rP1]
    ld a, [rP1]
    pop af
    pop bc
    ret
