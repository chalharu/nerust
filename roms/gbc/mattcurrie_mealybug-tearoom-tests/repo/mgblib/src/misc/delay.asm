IF !DEF(INC_delay)
INC_delay = 1 

; Delay for a specified number of M-cycles
; @param \1 Number of M-cycles to wait for
delay: MACRO
DELAY = (\1)

IF DELAY >= 100000
    REPT DELAY / 100000
    call Delay100000MCycles
    ENDR
DELAY = DELAY % 100000
ENDC

IF DELAY >= 10000
    call Delay10000MCycles - (3 * ((DELAY / 10000) - 1))
DELAY = DELAY % 10000
ENDC

IF DELAY >= 1000
    call Delay1000MCycles - (3 * ((DELAY / 1000) - 1))
DELAY = DELAY % 1000
ENDC

IF DELAY >= 100
    call Delay100MCycles - (3 * ((DELAY / 100) - 1))
DELAY = DELAY % 100
ENDC

IF DELAY >= 10
    call Delay10MCycles - (3 * ((DELAY / 10) - 1))
DELAY = DELAY % 10
ENDC

IF DELAY > 0
    nops DELAY
ENDC
    ENDM


Delay100000MCycles::
    call Delay10000MCycles
Delay90000MCycles::
    call Delay10000MCycles
Delay80000MCycles::
    call Delay10000MCycles
Delay70000MCycles::
    call Delay10000MCycles
Delay60000MCycles::
    call Delay10000MCycles
Delay50000MCycles::
    call Delay10000MCycles
Delay40000MCycles::
    call Delay10000MCycles
Delay30000MCycles::
    call Delay10000MCycles
Delay20000MCycles::
    call Delay10000MCycles

Delay10000MCycles::
    call Delay1000MCycles
Delay9000MCycles::
    call Delay1000MCycles
Delay8000MCycles::
    call Delay1000MCycles
Delay7000MCycles::
    call Delay1000MCycles
Delay6000MCycles::
    call Delay1000MCycles
Delay5000MCycles::
    call Delay1000MCycles
Delay4000MCycles::
    call Delay1000MCycles
Delay3000MCycles::
    call Delay1000MCycles
Delay2000MCycles::
    call Delay1000MCycles
    
Delay1000MCycles::
    call Delay100MCycles
Delay900MCycles::
    call Delay100MCycles
Delay800MCycles::
    call Delay100MCycles
Delay700MCycles::
    call Delay100MCycles
Delay600MCycles::
    call Delay100MCycles
Delay500MCycles::
    call Delay100MCycles
Delay400MCycles::
    call Delay100MCycles
Delay300MCycles::
    call Delay100MCycles
Delay200MCycles::
    call Delay100MCycles

Delay100MCycles::
    call Delay10MCycles
Delay90MCycles::
    call Delay10MCycles
Delay80MCycles::
    call Delay10MCycles
Delay70MCycles::
    call Delay10MCycles
Delay60MCycles::
    call Delay10MCycles
Delay50MCycles::
    call Delay10MCycles
Delay40MCycles::
    call Delay10MCycles
Delay30MCycles::
    call Delay10MCycles
Delay20MCycles::
    call Delay10MCycles

Delay10MCycles::
    ret


ENDC