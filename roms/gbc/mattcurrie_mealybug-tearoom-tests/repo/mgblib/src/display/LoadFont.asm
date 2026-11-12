IF !DEF(INC_LoadFont)
INC_LoadFont = 1

INCLUDE "src/memory/MemCopy.asm"

; Load the font into VRAM starting at address $8000.
; The LCD must be turned off before calling this function.
;
; @destroys all
LoadFont8000::

    ld de, $8000
    jr LoadFont.skip

; Load the font into VRAM starting at address $9000.
; The LCD must be turned off before calling this function.
;
; @destroys all
LoadFont::
    ld de, $9000

.skip:
    ld hl, OldSkoolOutlineThick
    ld bc, $800
    call MemCopy
    ret

ENDC