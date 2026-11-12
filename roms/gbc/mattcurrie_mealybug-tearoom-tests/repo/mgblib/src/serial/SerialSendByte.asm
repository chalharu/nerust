IF !DEF(INC_SerialSendByte)
INC_SerialSendByte = 1

; Send a byte over the serial connection as master. 
; Waits until the send has completed.
;
; @param a the value to send
; @destroys af
SerialSendByte::
    ldh [rSB], a
    ld a, $81
    ldh [rSC],a 
.loop:
    ldh a, [rSC]
    bit 7, a
    jr nz, .loop
    ret

ENDC
