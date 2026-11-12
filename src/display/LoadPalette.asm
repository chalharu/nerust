IF !DEF(INC_LoadPalette)
INC_LoadPalette = 1

; Load CGB object palette data
; Must be called either during Vblank or with the LCD turned off.
;
; @param hl address of palette data
; @param a the palette index to start writing to
; @param b the length of the palette data
;
; @destroys af, bc, hl
LoadObjectPalette::
    ld c, LOW(rOCPS)
    jr LoadPalette

; Load CGB background palette data
; Must be called either during Vblank or with the LCD turned off.
;
; @param hl address of palette data
; @param a the palette index to start writing to
; @param b the length of the palette data
;
; @destroys af, bc, hl
LoadBackgroundPalette::
    ld c, LOW(rBCPS)

LoadPalette::
    ; set the auto increment flag
    or $80
    ld [c], a

    ; C = LOW(rBCPD)
    inc c
    
.loop:
    ld a, [hl+]
    ld [c], a
    dec b
    jr nz, .loop

    ret

ENDC