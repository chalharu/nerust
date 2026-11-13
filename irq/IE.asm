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
        mov     r5, MEM_IO
        add     r5, 0x200
        add     r5, 0x2

        mov     r6, MEM_GAMEPAK0
        mov     r0, 0
        str     r0, [r3, REG_SNDCNT]
        str     r0, [r3, REG_SNDCNTX]
        str     r0, [r3, REG_IE]
        str     r0, [r3, REG_IME]

        ; set up IRQ handling
        mov     r4, r6
        adr     r4, .irq_hand
        ldr     r0, [r4]
        mov     r4, r6
        adr     r4, irq_rt
        str     r4, [r0]

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

.cnt_tmr:
        dw      0x00C0FFE0

.irq_hand:
        dw      0x03007FFC

irq_rt:
        mov     r0, 0
        mov     r3, MEM_IO
        str     r0, [r3, REG_IME]
        add     r3, REG_TIM0CNT
        str     r0, [r3]
        strh    r7, [r5]

        mov     r0, 1
        mov     r3, MEM_IO
        str     r0, [r3, REG_IME]
        add     r3, REG_TIM0CNT

        mov     r8, 1
        mov     r15, r14


code16
align 2

t001:
        ; tests what happens when IE is writen
        ; around the time IF is set
        mov     r7, 0x8
        mov     r0, 0x08
        strh    r0, [r5]
        mov     r0, 1
        mov     r2, 9
        lsl     r0, r2
        mov     r3, 1
        mov     r2, 26
        lsl     r3, r2
        add     r3, r0
        mov     r0, 8
        str     r0, [r3]
        add     r3, r0
        mov     r0, 1
        str     r0, [r3]
        mov     r0, 1
        mov     r2, 8
        lsl     r0, r2
        mov     r3, 1
        mov     r2, 26
        lsl     r3, r2
        add     r3, r0

        mov     r6, r5
        sub     r6, 2
        mov     r2, 8
        mov     r0, 0
        mov     r8, r0
        str     r0, [r3]
        strh    r7, [r5]
        str     r2, [r6]
        mov     r2, 0
        ldr     r0, [r4]
        str     r0, [r3]
        ldr     r0, [r4]
        mov     r0, r0
        mov     r0, r0
        ldr     r0, [r1]
        ldr     r0, [r1]
        str     r2, [r6]
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        str     r0, [r3]
        mov     r0, r0
        mov     r0, r0
        mov     r0, 0
        cmp     r8, r0
        bne     f001a

        mov     r0, 0
        mov     r2, 8
        mov     r8, r0
        strh    r7, [r5]
        str     r2, [r6]
        mov     r2, 0
        ldr     r0, [r4]
        str     r0, [r3]
        ldr     r0, [r4]
        mov     r0, r0
        mov     r0, r0
        ldr     r0, [r1]
        ldr     r0, [r1]
        mov     r0, r0
        str     r2, [r6]
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        str     r0, [r3]
        mov     r0, r0
        mov     r0, r0
        mov     r0, 0
        cmp     r8, r0
        bne     f001b

        mov     r0, 0
        mov     r2, 8
        mov     r8, r0
        strh    r7, [r5]
        str     r2, [r6]
        mov     r2, 0
        ldr     r0, [r4]
        str     r0, [r3]
        ldr     r0, [r4]
        mov     r0, r0
        mov     r0, r0
        ldr     r0, [r1]
        ldr     r0, [r1]
        mov     r0, r0
        mov     r0, r0
        str     r2, [r6]
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        str     r0, [r3]
        mov     r0, r0
        mov     r0, r0
        mov     r0, 1
        cmp     r8, r0
        bne     f001c

        mov     r0, 0
        mov     r2, 8
        mov     r8, r0
        strh    r7, [r5]
        str     r2, [r6]
        mov     r2, 0
        ldr     r0, [r4]
        str     r0, [r3]
        ldr     r0, [r4]
        mov     r0, r0
        mov     r0, r0
        ldr     r0, [r1]
        ldr     r0, [r1]
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        str     r2, [r6]
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        str     r0, [r3]
        mov     r0, r0
        mov     r0, r0
        mov     r0, 1
        cmp     r8, r0
        bne     f001d

        mov     r2, 0
        b       test_end

f001a:
        mov     r2, 0x1
        bl      test_end

f001b:
        mov     r2, 0x2
        bl      test_end

f001c:
        mov     r2, 0x3
        bl      test_end

f001d:
        mov     r2, 0x4
        bl      test_end

test_end:
        mov     r0, 0
        str     r0, [r3]
        adr     r0, eval
        bx      r0

code32
align 4
eval:
        mov     r12, r2
        m_vsync
        m_test_eval r12

idle:
        b       idle

include '../lib/text.asm'

main_end:
