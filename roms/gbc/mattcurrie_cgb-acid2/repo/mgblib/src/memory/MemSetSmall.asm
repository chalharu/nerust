IF !DEF(INC_MemSetSmall)
INC_MemSetSmall = 1

; Set a small amount memory < 256 bytes
; 
; @param a value to set
; @param hl source address
; @param c length in bytes
; @destroys f, c, hl
MemSetSmall::
.loop:
    ld [hl+], a
    dec c
    jr nz, .loop
    ret

ENDC