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
        str     r0, [r3, REG_TIM0CNT]
        str     r0, [r3, REG_SNDCNT]
        str     r0, [r3, REG_SNDCNTX]
        str     r0, [r3, REG_IE]
        str     r0, [r3, REG_IME]

        ; set waitstate
        mov     r0, 0x14
        str     r0, [r3, REG_WAITCNT]

        ; wait for ly 158
loop_ly_158:
        ldrb    r0, [r3, 6]
        cmp     r0, 158
        bne     loop_ly_158

        ; set up IRQ handling
        mov     r4, r6
        adr     r4, .irq_hand
        ldr     r0, [r4]
        mov     r4, r6
        adr     r4, irq_rt
        str     r4, [r0]

        mov     r0, 1
        str     r0, [r3, REG_IME]
        mov     r0, 1
        str     r0, [r3, REG_IE]

        ; turn on vbl irqs in the ppu
        mov    r0, 8
        str     r0, [r3, REG_DISPSTAT]

        mov     r0, MEM_EWRAM
        str     r0, [r3, REG_DMA0SAD]
        mov     r0, MEM_IWRAM
        str     r0, [r3, REG_DMA0DAD]

        ; enable timer 0
        mov     r5, r6
        adr     r5, .cnt_tmr
        ldr     r2, [r5]
        str     r2, [r3, REG_TIM0CNT]
        mov     r5, r6

        mov     r0, 0
        b       t001

align 4

.cnt_tmr:
        dw      0x00800000

.cnt_dma_0:
        dw      0xA340008C

.irq_hand:
        dw      0x03007FFC

irq_rt:
        mov     r0, 0
        mov     r3, MEM_IO
        str     r0, [r3, REG_IME]
        add     r3, REG_TIM0CNT
        ldr     r8, [r3]
        and     r8, 0xFF
;        str     r0, [r3]
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

t001:
        ; tests when IRQs are registered while DMA
        ; happens while halted

        ; halt, will return here when irq has processed
        swi     0x20000

        mov     r7, 0xFF

        ; PC moved to R14 is in R5
        mov     r0, 0xFF
        and     r5, r0
        cmp     r5, 0x74

        bne     f001a

        ; r8 has the timer
        cmp     r8, 0x55
        bne     f001b

        mov     r12, 0

        b       eval

f001a:
        mov     r12, r5
        b       eval

f001b:
        mov     r12, r8
        b       eval


eval:
        m_vsync
        m_test_eval r12

idle:
        b       idle

include '../lib/text.asm'

main_end:
