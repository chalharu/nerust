IF !DEF(INC_PrintDecimal)
INC_PrintDecimal = 1

INCLUDE "src/print/PrintCharacter.asm"

; Print an 8-bit number in decimal
;
; @param a number to print
; @destroys all
PrintDecimal::
    ld e, a
    cp 10
    jr c, .units

    ld b, 100
    cp b
    jr c, .tens
    
    ; value is >= 100
    ld d, "1"
    cp 200
    jr c, .one_hundreds

    inc d       ; value is 2xx so change d to "2"
    sub b       ; subtract 100 from value

.one_hundreds:
    sub b       ; subtract 100 from value
    ld e, a     ; store the value
    ld a, d     ; set a to "1" or "2"
    call PrintCharacter
    ld a, e     ; restore value from E

.tens:
    ld d, -1
    ld b, 10
.tens_loop:
    inc d
    sub b
    jr nc, .tens_loop

    add b       ; add 10 back to value
    ld e, a
    ld a, "0"
    add d
    call PrintCharacter

.units:
    ld a, "0"
    add e
    call PrintCharacter
    ret

ENDC