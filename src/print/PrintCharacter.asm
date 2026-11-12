IF !DEF(INC_PrintCharacter)
INC_PrintCharacter = 1

INCLUDE "src/macros.asm"
INCLUDE "src/print/PrintNewLine.asm"
INCLUDE "src/hardware.inc"

; Print a character and advance the cursor.
; Supports new line characters.
;
; @param a character to print
; @destroys af, bc, hl
PrintCharacter::
    cp "\n"
    jp z, PrintNewLine

    ld b, a     ; store the character in b
    
    ; read cursor position
    ld hl, wPrintCursorAddress
    ld a, [hl+]
    ld h, [hl]
    ld l, a

    wait_vram_accessible

    ld [hl], b  ; write character

    ; advance cursor and write cursor position
    inc hl

    ; save cursor position
    ld a, l
    ld [wPrintCursorAddress], a
    ld a, h
    ld [wPrintCursorAddress + 1], a

    ; if cursor has moved past the end of the line, 
    ; then jump to the new line case. otherwise return
    ld a, l
    and $1f
    cp 20

    ret c
    jp PrintNewLine

ENDC