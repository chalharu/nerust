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
        adr     r4, test_2
        add     r4, 1

        mov     r2, 0
        adr     r2, branch_1 + 1
        bx      r2
        mov     r0, r0
        mov     r0, r0
code16
align 2
branch_1:
        mov     r0, r0
        ldr     r0, [r1]
        ldr     r0, [r1]
        bx      r15
code32
align 4
        mov     r0, r0
        adr     r5, test_end + 1
        adr     r6, eval
        sub     r6, 2
        mov     r7, 0xFF
        ldr     r0, [r3]
        and     r0, r7
        cmp     r0, 0x4C
        bne     fail_go
        bx      r4
        dw      0x00000000

fail_go:
        adr     r7, f001a + 1
        bx      r7

code16
test_2:
        mov     r0, r0
        ldr     r0, [r3]
        and     r0, r7
        cmp     r0, 0x6A
        bne     f001b

        bx      r5

f001a:
        mov     r12, r0
        bx      r5

f001b:
        mov     r12, r0
        bx      r5

align 2
code16
test_end:
        db      0x00
        db      0x00
        bx      r6
        db      0x00
        db      0x00
        db      0x00
        db      0x00
        db      0x00
        db      0x00
align 4
code32
eval:
        m_vsync
        m_test_eval r12

idle:
        b       idle

include '../lib/text.asm'

main_end:
