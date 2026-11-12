IF !DEF(INC_MemSet)
INC_MemSet = 1

; Set some memory to a value
; 
; @param a value to set
; @param hl source address
; @param bc length in bytes
; @destroys f, bc, d, hl
MemSet::
    ld d, a

.loop:
    ld [hl+], a
    dec bc   
    ld a, b
    or c
    ld a, d
    jr nz, .loop

    ret

ENDC