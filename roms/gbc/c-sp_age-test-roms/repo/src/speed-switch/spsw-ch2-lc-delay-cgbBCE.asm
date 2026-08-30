; Tests how CGB speed switching affects APU length counter timing
;
; Verified:
;   2022-02-23 pass: CPU CGB E - CPU-CGB-06
;   2022-03-16 pass: CPU CGB C - CPU-CGB-04
;   2022-02-23 pass: CPU CGB B - CPU-CGB-02
;
INCLUDE "hardware.inc"
DEF CART_COMPATIBILITY EQU CART_COMPATIBLE_GBC
INCLUDE "test-setup.inc"



EXPECTED_TEST_RESULTS:
    ; number of test result rows
    DB 8
    ; ch2 init -> double speed
    DB $F2, $F0,  $F2, $F0,  $F2, $F0,  $F2, $F0
    ; ch2 init -> double speed -> ch2 init
    DB $F2, $F0,  $F2, $F0,  $00, $00,  $00, $00
    ; ch2 init -> double speed -> div reset
    DB $F2, $F0,  $F2, $F0,  $00, $00,  $00, $00
    ; sound off -> double speed -> sound on, ch2 init
    DB $F2, $F0,  $F2, $F0,  $00, $00,  $00, $00
    ; sound on -> double speed -> sound off, sound on, ch2 init
    DB $F2, $F0,  $F2, $F0,  $F2, $F0,  $F2, $F0
    ; ch2 init -> double speed -> normal speed -> double speed
    DB $F2, $F0,  $F2, $F0,  $F2, $F0,  $F2, $F0
    ; ch2 init -> double speed -> normal speed -> double speed -> normal speed -> double speed
    DB $F2, $F0,  $F2, $F0,  $F2, $F0,  $F2, $F0
    ; immediate length counter decrement (NS -> DS)
    DB $F2, $F0, $F2, $F0, $F2, $F0, $F2, $F0
    ; immediate length counter decrement (DS -> NS)
    DB $F2, $F0, $F2, $F0, $F2, $F0, $F2, $F0



MACRO SAVE_NR52
    ldh a, [rNR52]
    ld [hl+], a
ENDM

MACRO ADD_4_BYTE_PADDING
    inc hl
    inc hl
    inc hl
    inc hl
ENDM

MACRO SOUND_ON
    ld a, $80
    ldh [rNR52], a
ENDM

MACRO INIT_TEST
    SOUND_OFF
    ldh [rDIV], a ; reset DIV
ENDM

MACRO INIT_CH2_LC
    ld a, \1
    ldh [rNR21], a ; channel 2 length counter = \1
    ld a, $F0
    ldh [rNR22], a
    ld a, $00
    ldh [rNR23], a
    ld a, $C7
    ldh [rNR24], a ; initialize channel 2 with length counter
ENDM



; Length counter ticks are delayed by 1 m-cycle after
; switching to double speed no matter on what 1MHz edge
; sound was activated and speeds were switched.
MACRO TEST_DS
    INIT_TEST
    DELAY \1
    SOUND_ON
    INIT_CH2_LC $3B ; channel 2 length counter = 5
    DELAY \2
    SWITCH_SPEED    ; switch to double speed (length counter ticks delayed by 1 m-cycle)
    DELAY \3
    SAVE_NR52       ; read channel-2-on flag
    SWITCH_SPEED    ; switch to normal speed
ENDM

; Initialising channel 2 while running at double speed does
; not neutralise the current 1-m-cycle length counter
; delay no matter on what 2MHz edge channel 2 was initialised.
MACRO TEST_DS_CH2_INIT
    INIT_TEST
    SOUND_ON
    SWITCH_SPEED    ; switch to double speed (length counter ticks delayed by 1 m-cycle)
    DELAY \1
    INIT_CH2_LC $3F ; channel 2 length counter = 1
    DELAY \2
    SAVE_NR52       ; read channel-2-on flag
    SWITCH_SPEED    ; switch to normal speed
ENDM

; Resetting the DIV while running at double speed does
; not neutralise the current 1-m-cycle length counter
; delay no matter on what 2MHz edge the DIV was reset.
MACRO TEST_DS_DIV_RESET
    INIT_TEST
    SOUND_ON
    INIT_CH2_LC $3B ; channel 2 length counter = 5
    SWITCH_SPEED    ; switch to double speed (length counter ticks delayed by 1 m-cycle)
    DELAY \1
    ldh [rDIV], a    ; reset DIV
    DELAY \2
    SAVE_NR52       ; read channel-2-on flag
    SWITCH_SPEED    ; switch to normal speed
ENDM

; TODO describe
MACRO TEST_DS_ON
    INIT_TEST
    SWITCH_SPEED    ; switch to double speed (sound is off)
    DELAY \1
    SOUND_ON
    INIT_CH2_LC $3F ; channel 2 length counter = 1
    DELAY \2
    SAVE_NR52       ; read channel-2-on flag
    SWITCH_SPEED    ; switch to normal speed
ENDM

; TODO describe
MACRO TEST_DS_OFF_ON
    INIT_TEST
    SOUND_ON
    SWITCH_SPEED    ; switch to double speed (length counter ticks delayed by 1 m-cycle)
    DELAY \1
    SOUND_OFF
    DELAY \2
    SOUND_ON
    INIT_CH2_LC $3F ; channel 2 length counter = 1
    DELAY \3
    SAVE_NR52       ; read channel-2-on flag
    SWITCH_SPEED    ; switch to normal speed
ENDM

; The second switch to double speed does not introduce
; a 1 m-cycle length counter delay no matter on what 2MHz
; edge the switch to normal speed occurred and on what
; 1MHz edge double speed was activated.
MACRO TEST_DS_NS_DS
    INIT_TEST
    SOUND_ON
    INIT_CH2_LC $2F ; channel 2 length counter = 17
    SWITCH_SPEED    ; switch to double speed (length counter ticks delayed by 1 m-cycle)
    DELAY \1
    SWITCH_SPEED    ; switch to normal speed
    DELAY \2
    SWITCH_SPEED    ; switch to double speed (length counter ticks not delayed)
    DELAY \3
    SAVE_NR52       ; read channel-2-on flag
    SWITCH_SPEED    ; switch to normal speed
ENDM

; Length counter ticks are delayed by 1 m-cycle after the
; third switch to double speed no matter on what 2MHz
; edges normal speed was activated.
MACRO TEST_DS_NS_DS_NS_DS
    INIT_TEST
    SOUND_ON
    INIT_CH2_LC $23 ; channel 2 length counter = 29
    SWITCH_SPEED    ; switch to double speed (length counter ticks delayed by 1 m-cycle)
    DELAY \1
    SWITCH_SPEED    ; switch to normal speed
    SWITCH_SPEED    ; switch to double speed (length counter ticks not delayed)
    DELAY \2
    SWITCH_SPEED    ; switch to normal speed
    SWITCH_SPEED    ; switch to double speed (length counter ticks delayed by 1 m-cycle)
    DELAY \3
    SAVE_NR52       ; read channel-2-on flag
    SWITCH_SPEED    ; switch to normal speed
ENDM

; The DIV reset automatically triggered when switching
; speeds may cause immediate length counter decrements.
; However, the edge for immediate decrements is 1 m-cycle late.
; This 1 m-cycle delay only affects the 1->0 edge
; and not the 0->1 edge though.
MACRO TEST_RESET_EDGE_DS
    INIT_TEST
    SOUND_ON
    INIT_CH2_LC $3A ; channel 2 length counter = 6
    DELAY \1
    SWITCH_SPEED    ; switch to double speed
    DELAY \2
    SAVE_NR52       ; read channel-2-on flag
    SWITCH_SPEED    ; switch to normal speed
ENDM

MACRO TEST_RESET_EDGE_NS
    SWITCH_SPEED    ; switch to double speed
    INIT_TEST
    SOUND_ON
    INIT_CH2_LC $36 ; channel 2 length counter = 3
    DELAY \1
    SWITCH_SPEED    ; switch to normal speed
    DELAY \2
    SAVE_NR52       ; read channel-2-on flag
ENDM



run_test:
    call lcd_off
    ld hl, TEST_RESULTS

    TEST_DS 0, 0, 4093
    TEST_DS 0, 0, 4094
    TEST_DS 0, 1, 4093
    TEST_DS 0, 1, 4094
    TEST_DS 1, 0, 4093
    TEST_DS 1, 0, 4094
    TEST_DS 1, 1, 4093
    TEST_DS 1, 1, 4094

    TEST_DS_CH2_INIT 0, 4073
    TEST_DS_CH2_INIT 0, 4074
    TEST_DS_CH2_INIT 1, 4072
    TEST_DS_CH2_INIT 1, 4073
    ADD_4_BYTE_PADDING

    TEST_DS_DIV_RESET 0, 4093
    TEST_DS_DIV_RESET 0, 4094
    TEST_DS_DIV_RESET 1, 4093
    TEST_DS_DIV_RESET 1, 4094
    ADD_4_BYTE_PADDING

    TEST_DS_ON 0, 4067
    TEST_DS_ON 0, 4068
    TEST_DS_ON 1, 4066
    TEST_DS_ON 1, 4067
    ADD_4_BYTE_PADDING

    TEST_DS_OFF_ON 0, 0, 4063
    TEST_DS_OFF_ON 0, 0, 4064
    TEST_DS_OFF_ON 0, 1, 4062
    TEST_DS_OFF_ON 0, 1, 4063
    TEST_DS_OFF_ON 1, 0, 4062
    TEST_DS_OFF_ON 1, 0, 4063
    TEST_DS_OFF_ON 1, 1, 4061
    TEST_DS_OFF_ON 1, 1, 4062

    TEST_DS_NS_DS 0, 0, 4092
    TEST_DS_NS_DS 0, 0, 4093
    TEST_DS_NS_DS 0, 1, 4092
    TEST_DS_NS_DS 0, 1, 4093
    TEST_DS_NS_DS 1, 0, 4092
    TEST_DS_NS_DS 1, 0, 4093
    TEST_DS_NS_DS 1, 1, 4092
    TEST_DS_NS_DS 1, 1, 4093

    TEST_DS_NS_DS_NS_DS 0, 0, 4093
    TEST_DS_NS_DS_NS_DS 0, 0, 4094
    TEST_DS_NS_DS_NS_DS 0, 1, 4093
    TEST_DS_NS_DS_NS_DS 0, 1, 4094
    TEST_DS_NS_DS_NS_DS 1, 0, 4093
    TEST_DS_NS_DS_NS_DS 1, 0, 4094
    TEST_DS_NS_DS_NS_DS 1, 1, 4093
    TEST_DS_NS_DS_NS_DS 1, 1, 4094

    TEST_RESET_EDGE_DS 986, 12285
    TEST_RESET_EDGE_DS 986, 12286
    TEST_RESET_EDGE_DS 987, 8189
    TEST_RESET_EDGE_DS 987, 8190
    TEST_RESET_EDGE_DS 2009, 8189
    TEST_RESET_EDGE_DS 2009, 8190
    TEST_RESET_EDGE_DS 2010, 8189
    TEST_RESET_EDGE_DS 2010, 8190

    TEST_RESET_EDGE_NS 2010, 6140
    TEST_RESET_EDGE_NS 2010, 6141
    TEST_RESET_EDGE_NS 2011, 4092
    TEST_RESET_EDGE_NS 2011, 4093
    TEST_RESET_EDGE_NS 4057, 4092
    TEST_RESET_EDGE_NS 4057, 4093
    TEST_RESET_EDGE_NS 4058, 4092
    TEST_RESET_EDGE_NS 4058, 4093

    SOUND_OFF
    ld hl, EXPECTED_TEST_RESULTS
    ret
