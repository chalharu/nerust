IF !DEF(INC_WaitVBlank)
INC_WaitVBlank = 1

; Wait for Vblank (i.e. rLY = $90).
;
; @destroys af
WaitVBlank::
    wait_vblank
    ret

ENDC