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

        ; enable timer 0 IRQ
        mov     r0, 8
        str     r0, [r3, REG_IE]
        mov     r0, 1
        str     r0, [r3, REG_IME]

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
        dw      0x00C0FFDD

.irq_hand:
        dw      0x03007FFC

irq_rt:
        mov     r0, 0
        mov     r3, MEM_IO
        str     r0, [r3, REG_IME]
        add     r3, REG_TIM0CNT
        ldr     r8, [r3]
        and     r8, 0xFF
        str     r0, [r3]
        mov     r6, r14
        dw      0xE8BD500F
        ;ldm     r13,{r0, r1, r2, r3, r12, r14}
        mov     r5, r14
        dw      0xE92D500F
        ;stm     r13,{r0, r1, r2, r3, r12, r14}
        mov     r14, r6
        mov     r3, MEM_IO
        add     r3, REG_TIM0CNT

        mov     r15, r14


code16
align 2

t001:
        ; tests what value of pc is written to the stack
        ldr     r0, [r4]
        str     r0, [r3]
        swi     2

        mov     r7, 0xFF

        mov     r6, r8

        cmp     r6, 0xF0

        bne     f001a

        and     r5, r7
        cmp     r5, 0xB8

        bne     f001b

        mov     r2, 0
        b       test_end

f001a:
        mov     r2, r6
        bl      test_end

f001b:
        mov     r2, r5
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
