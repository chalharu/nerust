IF !DEF(INC_MemCopy)
INC_MemCopy = 1

; Copy some memory
;
; @param hl source address
; @param de destination address
; @param bc length in bytes
; @destroys all
MemCopy::

.loop:
    ld a, [hl+]
    ld [de], a
    inc de

    dec bc
    ld a, b
    or c
    jr nz, .loop

    ret

; Copy some memory
;
; @param \1 source address
; @param \2 destination address
; @param \3 length in bytes
; @destroys all
memcpy: MACRO
    ld hl, \1
    ld de, \2
    ld bc, \3
    call memcpy
    ENDM

ENDC