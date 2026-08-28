; Verified:
;   2022-01-21 pass: CPU CGB E - CPU-CGB-06
;   2022-03-16 pass: CPU CGB C - CPU-CGB-04
;   2022-01-21 pass: CPU CGB B - CPU-CGB-02
;   2022-01-21 pass: DMG-CPU C (blob) - DMG-CPU-08
;
INCLUDE "hardware.inc"
DEF CART_COMPATIBILITY EQU CART_COMPATIBLE_DMG_GBC
INCLUDE "test-setup.inc"

EXPECTED_TEST_RESULTS:
    ; number of test result rows
    DB 1
    ; result (A, IF): test_one_pending_interrupt
    DB $42, $E1, $00
    ; result (A, IF): test_two_pending_interrupts
    DB $84, $E2, $00
    ; result row padding
    DB $00, $00



run_test:
    ld hl, TEST_RESULTS

    call test_one_pending_interrupt
    call test_two_pending_interrupts

    ld hl, EXPECTED_TEST_RESULTS
    ret



PUSHS
SECTION "vblank-interrupt-handler", ROM0[$40]
    pop bc ; save interrupt return address to BC
    push bc
    inc a
    ret
SECTION "lcd-stat-interrupt-handler", ROM0[$48]
    pop bc ; save interrupt return address to BC
    push bc
    inc a
    ret
POPS

test_one_pending_interrupt:
    ; call lcd_off ; keep LCD on as we need the v-blank interrupt
    ; prepare interrupt handling
    ; (interrupt handling is disabled)
    ld a, 2
    ldh [rIF], a ; pending LCD STAT interrupt
    inc a
    ldh [rIE], a ; allow LCD STAT & v-blank interrupt
    ; prepare A
    ld a, $20
    ; enable interrupts after the next machine cycle
    ei
    ; first halt: with IME == 0 and pending interrupts
    ; second halt: with IME == 0 and NO pending interrupts
    ;              (terminated by regular v-blank interrupt)
.halt_addrees:
    halt
    rlca
    ; save results
    ld [hl+], a
    ldh a, [rIF]
    ld [hl+], a
    ld de, .halt_addrees
    ld a, e
    sub a, c
    ld [hl+], a
    ret

test_two_pending_interrupts:
    ; disable regular v-blank interrupt
    call lcd_off
    ; prepare interrupt handling
    ; (interrupt handling is disabled)
    ld a, 3
    ldh [rIF], a ; two pending interrupts
    ldh [rIE], a ; allow all pending interrupts
    ; prepare A
    ld a, $20
    ; enable interrupts after the next machine cycle
    ei
    ; first halt: with IME == 0 and pending interrupts
    ; second halt: with IME == 0 and pending interrupts
.halt_addrees:
    halt
    ; single-byte OP code -> rlca executed twice due to "halt bug"
    rlca
    ; save results
    ld [hl+], a
    ldh a, [rIF]
    ld [hl+], a
    ld de, .halt_addrees
    ld a, e
    sub a, c
    ld [hl+], a
    ret
