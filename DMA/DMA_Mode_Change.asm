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
        mov     r4, r6
        adr     r4, .wait_data
        ldr     r0, [r4]
        str     r0, [r3, REG_WAITCNT]


        mov     r0, MEM_IWRAM
        str     r0, [r3, REG_DMA1SAD]
        add     r0, 0x40
        str     r0, [r3, REG_DMA0DAD]

        ; start DMA 0 with hbl timing
        mov     r5, r6
        adr     r5, .cnt_dma_0
        ldr     r1, [r5]
        str     r1, [r3,REG_DMA0CNT]

        mov     r5, r6
        adr     r5, .cnt_dma_change
        ldr     r2, [r5]

        ; set up timer 0 to trigger IRQ during
        mov     r5, r6
        adr     r5, .cnt_tmr
        ldr     r5, [r5]
        mov     r4, r3
        add     r4, REG_TIM0CNT
        str     r5, [r4]

        b       t001

align 4
.wait_data:
        dw      0x00000014

.cnt_tmr:
        dw      0x00800000

.cnt_dma_0:
        dw      0xA4000001

.cnt_dma_change:
        dw      0x84000001

t001:
        ; tests how changing mode while DMA enabled works
        mov     r7, 0xFF

        str     r2, [r3, REG_DMA0CNT]
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0

        ; check timer
        ldr     r0, [r4]
        and     r0, r7
        cmp     r0, 0x2A

        bne     f001a

        ; check written value
        ldr     r0, [r3, REG_DMA0CNT]
        lsr     r0, 16
        lsr     r2, 16
        cmp     r0, r2
        bne     f001b

        ; wait until a DMA should occur on HBL


        mov     r2, 0

        b       test_end

f001a:
        mov     r2, r0
        bl      test_end

f001b:
        mov     r2, 2
        bl      test_end

test_end:
        mov     r0, 0
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
