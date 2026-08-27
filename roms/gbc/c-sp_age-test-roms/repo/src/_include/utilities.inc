IF !DEF(INC_UTILITIES)
DEF INC_UTILITIES EQU 1

INCLUDE "hardware.inc"
INCLUDE "macros.inc"



; ----------------------------------------------------------------------------
;
;   CGB features & cartridge ompatibility
;
; ----------------------------------------------------------------------------

PUSHS
SECTION "cgb-features", WRAM0
IS_CGB: DS 1              ; running on a CGB
FAIL_CGB_REQUIRED: DS 1   ; running a CGB-exclusive cartridge on a DMG
CGB_FEATURES_USABLE: DS 1 ; running a CGB-compatible cartridge on a CGB
POPS

; Set the zero flag,
; if we're running on a CGB.
; Reset the zero flag otherwise.
;
; check_cgb_on_startup must have been called before.
;
; @destroys af
;
MACRO CP_IS_CGB
    ld a, [IS_CGB]
    cp a, 1
ENDM

; Set the zero flag,
; if this cartridge is CGB exclusive but we're running on a DMG.
; Reset the zero flag otherwise.
;
; check_cgb_on_startup must have been called before.
;
; @destroys af
;
MACRO CP_FAIL_CGB_REQUIRED
    ld a, [FAIL_CGB_REQUIRED]
    cp a, 1
ENDM

; Set the zero flag,
; if CGB features are usable
; (i.e. if we're running a CGB compatible cartridge on a Game Boy Color).
; Reset the zero flag otherwise.
;
; check_cgb_on_startup must have been called before.
;
; @destroys af
;
MACRO CP_CGB_FEATURES_USABLE
    ld a, [CGB_FEATURES_USABLE]
    cp a, 1
ENDM

; Check the availability of Game Boy Color features
; and the cartridge compatibility.
; This method must be called with the initial value
; of CPU register a.
;
; @destroys af, b
;
check_cgb_on_startup:
    push af
    xor a, a
    ld [IS_CGB], a
    ld [FAIL_CGB_REQUIRED], a
    ld [CGB_FEATURES_USABLE], a
    pop af
    ; check the initial CPU register value: is this a Game Boy Color?
    cp a, $11
    jr nz, .no_cgb
    ; Game Boy Color recognized
    ld a, 1
    ld [IS_CGB], a
    ; check cartridge compatibility
    ld a, [$143] ; cartridge CGB flag
    cp a, CART_COMPATIBLE_DMG_GBC
    ret c ; cartridge not CGB compatible => CGB features not available
    ld a, 1
    ld [CGB_FEATURES_USABLE], a ; cartridge CGB compatible => CGB features available
    ret
.no_cgb:
    ; this is no Game Boy Color,
    ; check cartridge compatibility
    ld a, [$143] ; cartridge CGB flag
    cp a, CART_COMPATIBLE_GBC
    ret nz
    ; cartridge not DMG compatible
    ld a, 1
    ld [FAIL_CGB_REQUIRED], a
    ret



; ----------------------------------------------------------------------------
;
;   memory
;
; ----------------------------------------------------------------------------

; set memory (see memset)
;
; @param \1 destination address
; @param \2 the value to set
; @param \3 number of bytes to set
; @destroys af, bc, d, hl
;
MACRO MEMSET
    ld hl, \1
    ld d, \2
    ld bc, \3
    call memset
ENDM

; set memory
;
; @param bc number of bytes to set
; @param d the value to set
; @param hl destination address
; @destroys af, bc, d, hl
;
memset:
    ld a, d
    ld [hl+], a
    dec bc
    ld a, b
    or a, c
    jr nz, memset
    ret

; copy non-overlapping memory (see memcpy)
;
; @param \1 destination address
; @param \2 source address
; @param \3 number of bytes to copy
; @destroys af, bc, de, hl
;
MACRO MEMCPY
    ld hl, \1
    ld de, \2
    ld bc, \3
    call memcpy
ENDM

; copy non-overlapping memory
;
; @param bc number of bytes to copy
; @param de source address
; @param hl destination address
; @destroys af, bc, de, hl
;
memcpy:
    ld a, [de]
    ld [hl+], a
    inc de
    dec bc
    ld a, b
    or a, c
    jr nz, memcpy
    ret



; ----------------------------------------------------------------------------
;
;   LCD
;
; ----------------------------------------------------------------------------

; Wait for v-blank beginning.
; To reduce AGE log spam we try to minimize the number of rLY reads.
;
; @destroys
;
wait_for_vblank:
    push af
    push bc
.ly_loop:
    ldh a, [rLY]     ; 3 m-cycles
    cp a, 144        ; 2 m-cycles
    jp z, .finish    ; 2 m-cycles if not jumping
    cp a, 142        ; 2 m-cycles
    jr c, .long_wait ; 2 m-cycles if not jumping, 3 m-cycles else
    ; if we're near line 143,
    ; use short delays to reach line 144
    DELAY 50         ; 50 m-cycles
    jr .ly_loop      ; 3 m-cycles
    ; if we're well before line 143,
    ; use long delays to minimize reading rLY
.long_wait:
    ld b, a
    ld a, 142
    sub a, b
.long_loop:
    DELAY 456 / 4 - 4
    dec a
    jr nz, .long_loop
    jp .ly_loop
    ; line 144 reached
.finish:
    pop bc
    pop af
    ret

; Wait for V-Blank and turn of the LCD.
;
; @see wait_for_vblank
; @destroys
;
lcd_off:
    push af
    push hl
    ld hl, rLCDC
    bit 7, [hl]
    jr z, .already_off
    call wait_for_vblank
    res 7, [hl]
.already_off:
    pop hl
    pop af
    ret

; Turn off the LCD, copy tile data into video ram and setup palettes.
;
; @destroys af, bc, de, hl
;
setup_gfx:
    call lcd_off
    ; copy tile data for ASCII characters 32 - 95 in different styles
    MEMCPY _VRAM8000, .FONT_DATA_BEGIN, .FONT_DATA_END - .FONT_DATA_BEGIN
    ; setup palettes
    call setup_palettes
    ret

; We use the "Cellphone Font":
; https://opengameart.org/content/ascii-bitmap-font-cellphone
; originally created by domsson:
; https://opengameart.org/users/domsson
;
.FONT_DATA_BEGIN:
    INCBIN "font.2bpp"
    INCBIN "font-inverse.2bpp"
    INCBIN "font-left.2bpp"
    INCBIN "font-left-inverse.2bpp"
.FONT_DATA_END:

; Setup palettes.
;
; @destroys af, c
;
setup_palettes:
    ; setup DMG palettes
    ld a, %11100100
    ldh [rBGP], a
    ldh [rOBP0], a
    ldh [rOBP1], a

    ; setup CGB background palette
    ld a, %10000000
    ldh [rBCPS], a
    ld c, rBCPD & $FF
    ld a, %11111111 ; r = g = b = %11111
    ldh [c], a
    ld a, %01111111
    ldh [c], a
    ld a, %10110101 ; r = g = b = %10101
    ldh [c], a
    ld a, %01010110
    ldh [c], a
    ld a, %01001010 ; r = g = b = %01010
    ldh [c], a
    ld a, %00101001
    ldh [c], a
    xor a, a        ; r = g = b = %00000
    ldh [c], a
    ldh [c], a

    ; setup CGB object palette
    ld a, %10000000
    ldh [rOCPS], a
    ld c, rOCPD & $FF
    ld a, %11111111 ; r = g = b = %11111
    ldh [c], a
    ld a, %01111111
    ldh [c], a
    ld a, %10110101 ; r = g = b = %10101
    ldh [c], a
    ld a, %01010110
    ldh [c], a
    ld a, %01001010 ; r = g = b = %01010
    ldh [c], a
    ld a, %00101001
    ldh [c], a
    xor a, a        ; r = g = b = %00000
    ldh [c], a
    ldh [c], a
    ret


; ----------------------------------------------------------------------------
;
;   test result compaction
;
; ----------------------------------------------------------------------------

MACRO COMPACT_RESULTS_LINES
    REPT \1
        COMPACT_RESULTS_LINE \2, \3
    ENDR
ENDM

MACRO COMPACT_RESULTS_LINE
    STATIC_ASSERT (\1 > 0) && (\1 <= 8)
    STATIC_ASSERT \2 > 0

    FOR I, \1
        ld a, \2
        call compact_results
    ENDR
    REPT 8 - \1
        inc hl
    ENDR
ENDM

; @param a  number of bytes to compact
; @param bc address of the test results to compact
; @param de address of the expected test results
; @param hl address to store the compaction result
;
; @return bc input bc incremented by a
; @return de input de incremented by a
; @return hl input hl incremented by one
;
; @destroys af
;
compact_results:
    push hl
    push af
    ld l, a
.compare_byte:
    ld a, [de]
    ld h, a
    ld a, [bc]
    cp a, h
    jr z, .next_byte
    ; test result mismatch:
    ; write the incorrect byte's index to [hl]
    pop af
    sub l
.fast_forward:
    inc de
    inc bc
    dec l
    jr nz, .fast_forward
    jr .return
.next_byte:
    inc de
    inc bc
    dec l
    jr nz, .compare_byte
    ; test result matches:
    ; write $FF to [hl]
    pop af
    ld a, $FF
.return:
    pop hl
    ld [hl+], a
    ret



; ----------------------------------------------------------------------------
;
;   miscellaneous
;
; ----------------------------------------------------------------------------

freeze:
    ; deny all interrupts from being handled
    xor a, a
    ldh [rIE], a
    ; enable interrupt handling so that
    ; "halt" does not terminate immediately
    ei
    nop
    ; halt CPU
    halt
    nop
    ; just in case: repeat
    jr freeze



ENDC ; !DEF(INC_UTILITIES)
