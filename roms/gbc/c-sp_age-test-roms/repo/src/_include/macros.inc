IF !DEF(INC_MACROS)
DEF INC_MACROS EQU 1

INCLUDE "hardware.inc"



; ----------------------------------------------------------------------------
;
;   delay
;
; ----------------------------------------------------------------------------

; Generate `N` `nop` instructions.
;
; @param \1 the number of `nop` instructions to generate
; @destroys
;
; @see DELAY, DELAY256
;
MACRO NOPS
    REPT \1
        nop
    ENDR
ENDM

; Generate a delay of N m-cycles.
; As this macro uses `DELAY256` internally,
; it is not suited to wait for an interrupt.
; See `DELAY256` for an explanation on this.
;
; @param \1 size of the delay in m-cycles
; @destroys
;
; @see DELAY256, NOPS
;
MACRO DELAY
    IF (\1) >= 256
        DELAY256 (\1) / 256
        NOPS (\1) % 256
    ELSE
        NOPS \1
    ENDC
ENDM

; Generate a delay of `N * 256` m-cycles.
; The delay is implemented as "busy wait" loop.
; It is thus not suited for measuring interrupt edges
; as interrupts cannot be served on each m-cycle.
;
; @param \1 the number of 256-`nop` delays to generate (0-255)
; @destroys
;
; @see DELAY, NOPS
;
MACRO DELAY256
    IF \1 > 0
        ; 8 m-cycles for this block
        push af ; 4 m-cycles
        push bc ; 4 m-cycles

        ; 8 m-cycles for this block
        ld b, \1                     ; 2 m-cycles, b = number of .loop\@ iterations
        ld c, (256 - 8 - 24 + 4) / 4 ; 2 m-cycles
        nop                          ; 1 m-cycle
        jr .loop_inner\@             ; 3 m-cycles

        ; it takes 256 m-cycles for one .loop\@ iteration
.loop\@:
        ; 4 m-cycles for this block
        ld c, (256 - 8) / 4 ; 2 m-cycles, c = number of .loop_inner\@ iterations
        nop                 ; 1 m-cycle
        nop                 ; 1 m-cycle

        ; it takes 4 m-cycles for one .loop_inner\@ iteration
.loop_inner\@:
        dec c                ; 1 m-cycle
        jr nz, .loop_inner\@ ; 3 m-cycles for jump, 2 m-cycles else
        nop                  ; 1 m-cycle to pad the jr's else-path to 3 cycles

        dec b          ; 1 m-cycle
        jr nz, .loop\@ ; 3 m-cycles for jump, 2 m-cycles else
        nop            ; 1 m-cycle to pad the jr's else-path to 3 cycles

        ; 8 m-cycles for this block
        nop    ; 1 m-cycle
        nop    ; 1 m-cycle
        pop bc ; 3 m-cycles
        pop af ; 3 m-cycles
    ENDC
ENDM



; ----------------------------------------------------------------------------
;
;    miscellaneous
;
; ----------------------------------------------------------------------------

; Turn off the sound.
;
; @destroys a
;
MACRO SOUND_OFF
    xor a
    ldh [rNR52], a
ENDM

; Turn off the timer.
;
; @destroys a
;
MACRO TIMER_OFF
    xor a, a
    ldh [rTAC], a
ENDM

; (Re-)start the timer with the specified frequency.
; We also reset the `DIV` and clear `IF`.
;
; @param \1 `TAC` frequency flags
; @destroys a
;
MACRO TIMER_RESTART_CLEAN
    ; turn off timer
    TIMER_OFF
    ldh [rTIMA], a
    ; clear all interrupts
    ldh [rIF], a
    ; make sure we have a well defined DIV state
    ; (to prevent arbitrary TIMA increments by future DIV resets)
    ldh [rDIV], a
    ; start timer
    ld a, TACF_START | \1
    ldh [rTAC], a
ENDM

; switch between CGB speed modes
;
; @destroys a
;
MACRO SWITCH_SPEED
	ld a, $30
    ldh [rP1], a
	ld a, 1
	ldh [rKEY1], a
	stop ; RGBDS automatically inserts a NOP after STOP
ENDM

; Set a specific bit in IE without modifying
; any other bits.
;
; @destroys af
;
MACRO SET_IE_BIT
    ld a, [rIE]
    or a, \1
    ld [rIE], a
ENDM



ENDC ; !DEF(INC_MACROS)
