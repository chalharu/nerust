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
        ldr     r4, [r4]
        str     r4, [r3, REG_WAITCNT]

        add     r3, REG_TIM0CNT
        mov     r1, MEM_EWRAM

        mov     r4, r6
        adr     r4, .cnt_tmr

        mov     r5, MEM_IO
        add     r5, 4

        mov     r0, 0
        adr     r0, t001 + 1
        bx      r0

align 4
.wait_data:
        dw      0x00004014
align 4
.cnt_tmr:
        dw      0x00800000

code16
align 2

t001:
        ; loading a register from EWRAM should add one instruction
        ; to the prefetrcher for each load
        ; tests what happens when branching to nearby addresses
        mov     r7, 0x1
        mov     r0, 0
        str     r0, [r3]
        ldr     r0, [r4]
        str     r0, [r3]
vb1:
        ldrh    r0, [r5]
        and     r0, r7
        cmp     r0, 1
        bne     vb1

        mov     r7, 0xFF
        ldr     r0, [r3]
        mov     r1, r0
        and     r0, r7
        lsr     r1, 8
        and     r1, r7
        cmp     r0, 0x4E
        bne     f001a
        cmp     r1, 0x9E
        bne     f001b

        b       test_end

f001a:
        mov     r12, r0
        bl      test_end

f001b:
        mov     r12, r1
        bl      test_end

test_end:
        mov     r0, 0
        adr     r0, eval
        bx      r0

code32
align 4
eval:
        m_vsync
        m_test_eval r12

idle:
        b       idle

include '../lib/text.asm'

main_end:
