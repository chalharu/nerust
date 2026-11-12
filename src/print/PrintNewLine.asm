IF !DEF(INC_PrintNewLine)
INC_PrintNewLine = 1

INCLUDE "src/macros.asm"

PUSHS
SECTION "print_cursor", WRAM0

wPrintCursorAddress::
    DS 2

wPrintScrolling::
    DS 1

POPS

; Advance cursor to point to the start of a new line
; 
; @destroys af, bc, hl
PrintNewLine::
    ld hl, wPrintCursorAddress
    ld a, [hl]
    or (32 - 1)
    inc a
    ld [hl], a

    ; check for zero flag because carry is not set by `inc a`
    jr nz, .scroll_check

    ; LSB overflowed so need to increment MSB
    inc hl
    inc [hl]

    ; bit 2 should never be set, as it means we reached $9c00, so unset it to wrap back to $9800
    res 2, [hl] 

    ; restore hl's value to wPrintCursorAddress
    dec hl

.scroll_check:
    ld a, [wPrintScrolling]
    or a
    jr nz, .scrolling

    ; check if need to start scrolling yet
    ; $9a40 = $9800 + 32 * 18 rows
    ld hl, wPrintCursorAddress + 1
    ld a, [hl-]
    cp $9a
    ret nz

    ld a, [hl]
    cp $40
    ret nz

    ; reached $9a40, so set wPrintScrolling to 1
    ld a, 1
    ld [wPrintScrolling], a

    
.scrolling:
    ; expects HL = wPrintCursorAddress

    ; load address from wPrintCursorAddress
    ld a, [hl+]
    ld h, [hl]
    ld l, a

    ; erase the next line with spaces
    ld c, 20
    ld b, " "   ; use B because wait_vram_accessible destroys A

.erase_loop:
    wait_vram_accessible
    ld [hl], b
    inc hl
    dec c
    jr nz, .erase_loop

    ; scroll up the new blank line into view
    ldh a, [rSCY]
    add 8
    ldh [rSCY], a

    ret

ENDC