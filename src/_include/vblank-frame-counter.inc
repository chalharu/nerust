IF !DEF(INC_VBLANK_FRAME_COUNTER)
DEF INC_VBLANK_FRAME_COUNTER EQU 1


PUSHS
SECTION "frame-counter", WRAM0
FRAME_COUNTER: DS 1

SECTION "vblank-interrupt-handler", ROM0[$40]
    jp on_vblank_interrupt
POPS


activate_vblank_frame_counter:
    ; indicate finished test after 5 frames
    ld a, 5
    ld [FRAME_COUNTER], a
    ; enable v-blank interrupt
    SET_IE_BIT IEF_VBLANK
    ei
    ret


on_vblank_interrupt:
    push af
    ; frame limit already reached?
    ld a, [FRAME_COUNTER]
    or a, a
    jr z, .continue
    ; count this frame
    dec a
    ld [FRAME_COUNTER], a
    ; indicate finished test,
    ; if frame limit reached on this frame
    jr nz, .continue
    INDICATE_FINISHED_TEST
.continue:
    pop af
    reti


ENDC ; !DEF(INC_VBLANK_FRAME_COUNTER)
