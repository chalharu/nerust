IF !DEF(INC_ResetCursor)
INC_ResetCursor = 1

; Resets the cursor position, so the next print will start at $9800
;
; @destroys af
ResetCursor::
    xor a
    ldh [rSCY], a
    ld [wPrintScrolling], a
    ld [wPrintCursorAddress], a
    ld a, $98
    ld [wPrintCursorAddress + 1], a
    ret

ENDC