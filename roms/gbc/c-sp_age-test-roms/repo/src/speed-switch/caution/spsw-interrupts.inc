IF !DEF(INC_SPSW_INTERRUPTS)
DEF INC_SPSW_INTERRUPTS EQU 1

IF DEF(CGB_E)
    DEF OFS_B EQU 0
ELSE
    DEF OFS_B EQU 1
ENDC

INCLUDE "test-setup.inc"



EXPECTED_TEST_RESULTS:
    ; number of test result rows
    DB 11
    ; timer interrupt
    ; (DIV, DIV after STOP, TIMA, IF)
    DB $0F, $10, $08, $E0
    DB $0F, $10, $08, $E0
    DB $0F, $10, $09, $E0
    DB $0F, $10, $09, $E0
    DB $10, $10, $09, $E0
    DB $10, $10, $09, $E0
    ; immediate interrupt - DIV timing
    DB $00, $00, $01, $01, $00, $00, $01, $01
    DB $00, $00, $01, $01, $00, $00, $01, $01
    DB $00, $00, $01, $01, $00, $00, $00, $00
    ; immediate interrupt - TIMA timing
    DB $E0, $01, $E0, $01, $E0, $02, $E0, $02
    DB $E0, $01, $E0, $01, $E0, $02, $E0, $02
    DB $E0, $01, $E0, $01, $E0, $02, $E0, $02
    DB $E0, $01, $E0, $01, $E0, $02, $E0, $02
    DB $E0, $01, $E0, $01, $E0, $02, $E0, $02

run_test:
    SOUND_OFF
    call lcd_off
    ld hl, TEST_RESULTS

    call test_timer_interrupt_handling
    call test_immediate_interrupt_div
    inc hl ; pad results
    inc hl
    inc hl
    inc hl
    call test_immediate_interrupt_tima

    TIMER_OFF
    ld hl, EXPECTED_TEST_RESULTS
    ret

; all interrupt handlers basically execute a JP DE
PUSHS
SECTION "vblank-interrupt-handler", ROM0[$40]
    push de
    ret
SECTION "stat-interrupt-handler", ROM0[$48]
    push de
    ret
SECTION "timer-interrupt-handler", ROM0[$50]
    push de
    ret
SECTION "serial-interrupt-handler", ROM0[$58]
    push de
    ret
SECTION "joypad-interrupt-handler", ROM0[$60]
    push de
    ret
POPS

; Trigger a CPU speed switch with a custom pending interrupt.
;
; @param a value to write to IF and IE
; @param de the address to jump when handling the interrupt
; @destroys af
;
switch_speed_immediate_interrupt:
    ; prepare interrupt
    ; (interrupts are disabled at the moment)
    ld [rIF], a
    ld [rIE], a
    ; prepare speed switch
    ld a, $30
    ldh [rP1], a
    ld a, 1
    ldh [rKEY1], a
    ; enable interrupts after speed switch
    ei
    stop
    ; disable interrupts (just to be sure)
    di
    ret



test_timer_interrupt_handling:
    ld de, .timer_int1
    call .speed_switch_with_timer_interrupt
    ld de, .timer_int2
    call .speed_switch_with_timer_interrupt
    ld de, .timer_int3
    call .speed_switch_with_timer_interrupt
    ret

.timer_int1:
    ldh a, [rDIV] ; $0F
    ld [hl+], a
    ret ; leave interrupts disabled
.timer_int2:
    DELAY $06
    ldh a, [rDIV] ; $0F
    ld [hl+], a
    ret ; leave interrupts disabled
.timer_int3:
    DELAY $07
    ldh a, [rDIV] ; $10
    ld [hl+], a
    ret ; leave interrupts disabled

.speed_switch_with_timer_interrupt:
    ; start 262 KHz timer
    ; (will overflow during during speed switch)
    TIMER_RESTART_CLEAN TACF_262KHZ
    ; enable timer interrupt
    xor a
    ldh [rIF], a
    ld a, IEF_TIMER
    ldh [rIE], a
    ei
    ; switch to double speed
    SWITCH_SPEED
    ; disable interrupts (just to be sure)
    di
    ; store DIV, TIMA & IF
    ldh a, [rDIV]
    ld [hl+], a
    ldh a, [rTIMA]
    ld [hl+], a
    ldh a, [rIF]
    ld [hl+], a
    ; start 262 KHz timer
    ; (will overflow during during speed switch)
    TIMER_RESTART_CLEAN TACF_262KHZ
    ; enable timer interrupt
    xor a
    ldh [rIF], a
    ld a, IEF_TIMER
    ldh [rIE], a
    ei
    ; switch to normal speed
    SWITCH_SPEED
    ; disable interrupts (just to be sure)
    di
    ; store DIV, TIMA & IF
    ldh a, [rDIV]
    ld [hl+], a
    ldh a, [rTIMA]
    ld [hl+], a
    ldh a, [rIF]
    ld [hl+], a
    ret



; Test the timing of handling interrupts that are flagged
; right on speed switch.
; Timing is tested using DIV.

MACRO IMMEDIATE_INTERRUPT_DIV
    ; test by switching to double speed
    ld a, \1
    ld de, .interrupt\@
    call switch_speed_immediate_interrupt
    ; test by switching to normal speed
    ld a, \1
    call switch_speed_immediate_interrupt
    jp .done\@
.interrupt\@:
    DELAY \2
    ldh a, [rDIV]
    ld [hl+], a
    ret ; leave interrupts disabled
.done\@:
ENDM

test_immediate_interrupt_div:
    IMMEDIATE_INTERRUPT_DIV %00000001, $30 + OFS_B
    IMMEDIATE_INTERRUPT_DIV %00000001, $31 + OFS_B
    IMMEDIATE_INTERRUPT_DIV %00000010, $30 + OFS_B
    IMMEDIATE_INTERRUPT_DIV %00000010, $31 + OFS_B
    IMMEDIATE_INTERRUPT_DIV %00000100, $30 + OFS_B
    IMMEDIATE_INTERRUPT_DIV %00000100, $31 + OFS_B
    IMMEDIATE_INTERRUPT_DIV %00001000, $30 + OFS_B
    IMMEDIATE_INTERRUPT_DIV %00001000, $31 + OFS_B
    IMMEDIATE_INTERRUPT_DIV %00010000, $30 + OFS_B
    IMMEDIATE_INTERRUPT_DIV %00010000, $31 + OFS_B
    ret



; Test the timing of handling interrupts that are flagged
; right on speed switch.
; Timing is tested using TIMA.
; IF is checked to make sure no TIMA overflow occurred.

MACRO IMMEDIATE_INTERRUPT_TIMA
    ; start 16 KHz timer
    TIMER_RESTART_CLEAN TACF_16KHZ
    ; set custom interrupt flag and switch speed
    ld a, \1
    ld de, .interrupt\@
    call switch_speed_immediate_interrupt
    ; start 16 KHz timer
    TIMER_RESTART_CLEAN TACF_16KHZ
    ; set custom interrupt flag and switch speed
    ld a, \1
    call switch_speed_immediate_interrupt
    jp .done\@
.interrupt\@:
    ldh a, [rIF]
    ld [hl+], a
    DELAY \2
    ldh a, [rTIMA]
    ld [hl+], a
    ret ; leave interrupts disabled
.done\@:
ENDM

test_immediate_interrupt_tima:
    IMMEDIATE_INTERRUPT_TIMA %00000001, $2B + OFS_B
    IMMEDIATE_INTERRUPT_TIMA %00000001, $2C + OFS_B
    IMMEDIATE_INTERRUPT_TIMA %00000010, $2B + OFS_B
    IMMEDIATE_INTERRUPT_TIMA %00000010, $2C + OFS_B
    IMMEDIATE_INTERRUPT_TIMA %00000100, $2B + OFS_B
    IMMEDIATE_INTERRUPT_TIMA %00000100, $2C + OFS_B
    IMMEDIATE_INTERRUPT_TIMA %00001000, $2B + OFS_B
    IMMEDIATE_INTERRUPT_TIMA %00001000, $2C + OFS_B
    IMMEDIATE_INTERRUPT_TIMA %00010000, $2B + OFS_B
    IMMEDIATE_INTERRUPT_TIMA %00010000, $2C + OFS_B
    ret



ENDC ; INC_SPSW_INTERRUPTS
