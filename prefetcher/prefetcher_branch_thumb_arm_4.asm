format binary as 'gba'

include '../lib/constants.inc'
include '../lib/macros.inc'

header:
        include '../lib/header.asm'

main:
        m_test_init
        ; Reset test register
        mov     r12, 0

        ; turn off sound
        mov     r3, MEM_IO
        mov     r6, MEM_GAMEPAK0
        mov     r0, 0
        str     r0, [r3, REG_SNDCNT]
        str     r0, [r3, REG_SNDCNTX]

        ; set waitstate
        mov     r4, r6
        adr     r4, .wait_data
        ldr     r0, [r4]
        str     r0, [r3, REG_WAITCNT]

        add     r3, REG_TIM0CNT

        mov     r1, MEM_EWRAM

        mov     r4, r6
        adr     r4, .cnt_tmr

        b       t001

align 4
.wait_data:
        dw      0x00004014
align 4
.cnt_tmr:
        dw      0x00800000



t001:
        ; tests branching back and forth between thumb and arm
        ; turn on the timer
        mov     r7, 0xFF
        mov     r0, 0
        str     r0, [r3]
        ldr     r0, [r4]
        str     r0, [r3]
        adr     r4, test_1
        sub     r4, 3
        adr     r6, eval
        mov     r0, r0
        ldr     r0, [r1]
        bx      r4
        dw      0x00000000
code16
test_1:
        mov     r0, r0
        ldr     r0, [r3]
        and     r0, r7
        cmp     r0, 0x28
        bne     f001a
        b       test_end

f001a:
        mov     r12, r0
        bx      r6

test_end:
        mov     r0, r0
        bx      r6

align 4
code32
eval:
        m_vsync
        m_test_eval r12

idle:
        b       idle

include '../lib/text.asm'

main_end:
