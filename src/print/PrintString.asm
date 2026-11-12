IF !DEF(INC_PrintString)
INC_PrintString = 1

INCLUDE "src/print/PrintCharacter.asm"

; Print a NUL terminated string
;
; @param de address of string to print
; @destroys all
PrintString::
.loop:
    ld a, [de]
    or a
    ret z
    call PrintCharacter
    inc de
    jr .loop


; Macro to print a string literal. 
; NUL terminator is appended automatically.
; 
; @param \1 the string to print
; @destroys all
print_string_literal: MACRO
    ld de, .string\@
    call PrintString
    jr .end\@
.string\@:
    DB \1, $00
.end\@:
    ENDM

ENDC