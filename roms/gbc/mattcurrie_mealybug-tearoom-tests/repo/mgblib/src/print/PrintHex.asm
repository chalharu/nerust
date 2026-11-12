IF !DEF(INC_PrintHex)
INC_PrintHex = 1

INCLUDE "src/print/PrintCharacter.asm"

; Print a 16-bit value in hexidecimal, 4 digits with leading dollar symbol
;
; @param de number to print
; @destroys af, bc, hl
PrintHexU16::
    ld a, "$"
    call PrintCharacter
PrintHexU16NoDollar::
    ld a, d
    call PrintHexU8NoDollar
    ld a, e
    jp PrintHexU8NoDollar


; Print a 8-bit value in hexidecimal, 2 digits with leading dollar symbol
;
; @param a number to print
; @destroys af, bc, hl
PrintHexU8::
    push af
    ld a, "$"
    call PrintCharacter
    pop af
PrintHexU8NoDollar::
    push af
    swap a
    call PrintHexNibble
    pop af
    
    ; jp PrintHexNibble  - fall through to PrintHexNibble


; Print a 4-bit value in hexidecimal
;
; @param a value to print (only low nibble is used)
; @destroys af, bc, hl
PrintHexNibble::
    and $0f
    cp $0a
    jr nc, .letter
  
    add "0"         ; printing a number
    jp PrintCharacter

.letter:
    add "A" - 10    ; printing a letter
    jp PrintCharacter

ENDC