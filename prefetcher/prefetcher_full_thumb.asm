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
        mov     r7, 0xFF
        mov     r0, 0
        str     r0, [r3]
        ldr     r0, [r4]
        str     r0, [r3]
        ldr     r0, [r4]
        ldr     r0, [r1]
        ldr     r0, [r1]
        ldr     r0, [r1]
        mov     r0, r0
        b       branch_1
branch_1:
        mov     r0, r0
        mov     r0, r0
        ldr     r0, [r3]
        and     r0, r7
        mov     r5, 0x30
        cmp     r0, r5
        bne     f001a

        mov     r0, 0
        str     r0, [r3]
        ldr     r0, [r4]
        str     r0, [r3]
        ldr     r0, [r4]
        ldr     r0, [r1]
        ldr     r0, [r1]
        ldr     r0, [r1]
        mov     r0, r0
        b       branch_2
        mov     r0, r0
branch_2:
        mov     r0, r0
        ldr     r0, [r3]
        and     r0, r7
        mov     r5, 0x2E
        cmp     r0, r5
        bne     f001b

        mov     r0, 0
        str     r0, [r3]
        ldr     r0, [r4]
        str     r0, [r3]
        ldr     r0, [r4]
        ldr     r0, [r1]
        ldr     r0, [r1]
        ldr     r0, [r1]
        mov     r0, r0
        b       branch_3
        mov     r0, r0
        mov     r0, r0
branch_3:
        ldr     r0, [r3]
        and     r0, r7
        mov     r5,0x27
        cmp     r0, r5
        bne     f001c
        b       t002

f001a:
        mov     r12, r0
        bl      test_end

f001b:
        mov     r12, r0
        bl      test_end

f001c:
        mov     r12, r0
        bl      test_end


t002:
        ; loading a register from EWRAM should add one instruction
        ; to the prefetrcher for each load
        ; tests what happens when trying to add more than 8 samples
        mov     r0, 0
        str     r0, [r3]
        ldr     r0, [r4]
        str     r0, [r3]
        ldr     r0, [r4]
        ldr     r0, [r1]
        ldr     r0, [r1]
        mov     r0, r0
        mov     r0, r0
        ldr     r0, [r3]
        and     r0, r7
        mov     r5, 0x1D
        cmp     r0, r5
        bne     f002a

        mov     r0, 0
        str     r0, [r3]
        ldr     r0, [r4]
        str     r0, [r3]
        ldr     r0, [r4]
        ldr     r0, [r1]
        ldr     r0, [r1]
        ldr     r0, [r1]
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        ldr     r0, [r3]
        and     r0, r7
        mov     r5, 0x26
        cmp     r0, r5
        bne     f002b

        mov     r0, 0
        str     r0, [r3]
        ldr     r0, [r4]
        str     r0, [r3]
        ldr     r0, [r4]
        ldr     r0, [r1]
        ldr     r0, [r1]
        ldr     r0, [r1]
        ldr     r0, [r1]
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        ldr     r0, [r3]
        and     r0, r7
        mov     r5, 0x2F
        cmp     r0, r5
        bne     f002c

        mov     r0, 0
        str     r0, [r3]
        ldr     r0, [r4]
        str     r0, [r3]
        ldr     r0, [r4]
        ldr     r0, [r1]
        ldr     r0, [r1]
        ldr     r0, [r1]
        ldr     r0, [r1]
        ldr     r0, [r1]
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        ldr     r0, [r3]
        and     r0, r7
        mov     r5, 0x38
        cmp     r0, r5
        bne     f002d
        bl      test_end

f002a:
        mov     r12, r0
        bl      test_end

f002b:
        mov     r12, r0
        bl      test_end

f002c:
        mov     r12, r0
        bl      test_end

f002d:
        mov     r12, r0
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
