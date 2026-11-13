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
        ; loading a register from EWRAM should add one instruction
        ; to the prefetrcher for each load
        ; tests what happens when branching to nearby addresses
        mov     r7, 0xFF
        mov     r0, 0
        str     r0, [r3]
        ldr     r0, [r4]
        str     r0, [r3]

        mov     r2, 0
        adr     r2, branch_1 + 1

        ldr     r0, [r4]
        ldr     r0, [r1]
        ldr     r0, [r1]
        ldr     r0, [r1]
        bx      r2
        mov     r0, r0
        mov     r0, r0

code16
align 2
branch_1:
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        ldr     r0, [r3]
        and     r0, r7

        mov     r6, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, 0

        str     r0, [r3]
        ldr     r0, [r4]
        str     r0, [r3]
        mov     r2, 0

        adr     r2, branch_2
        ;mov     r0, r0
        ldr     r0, [r4]
        ldr     r0, [r1]
        ldr     r0, [r1]
        ldr     r0, [r1]
        ;mov     r0, r0
        bx      r2
        mov     r0, r0
        mov     r0, r0

code32
align 4
branch_2:
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        ldr     r0, [r3]
        and     r0, r7
        mov     r5, 0x3A
        cmp     r6, r5
        bne     f001a
        mov     r5, 0x53
        cmp     r0, r5
        bne     f001b
        b       test_end

f001a:
        mov     r12, r6
        b       eval

f001b:
        mov     r12, r0
        b       eval

test_end:
        b       eval

eval:
        m_vsync
        m_test_eval r12

idle:
        b       idle

include '../lib/text.asm'

main_end:
