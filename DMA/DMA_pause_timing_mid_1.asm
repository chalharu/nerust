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

        mov     r0, 0

        str     r0, [r3, REG_DMA0SAD]
        str     r0, [r3, REG_DMA0DAD]
        str     r0, [r3, REG_DMA0CNT]

        str     r0, [r3, REG_DMA1SAD]
        str     r0, [r3, REG_DMA1DAD]
        str     r0, [r3, REG_DMA1CNT]


        mov     r0, MEM_IWRAM
        str     r0, [r3, REG_DMA1DAD]
        add     r0, 0x40
        str     r0, [r3, REG_DMA0DAD]

        mov     r0, r3
        add     r0, REG_TIM0CNT
        str     r0, [r3, REG_DMA0SAD]
        str     r0, [r3, REG_DMA1SAD]

        ; start timer 0
        mov     r5, r6
        adr     r5, .cnt_tmr
        ldr     r2, [r5]
        str     r2, [r3,REG_TIM0CNT]

        ; start DMA 1 with immediate timing
        ; start dma 0 with hbl timing repeating
        mov     r5, r6
        adr     r5, .cnt_dma_0
        ldr     r2, [r5]
        str     r2, [r3,REG_DMA0CNT]

        mov     r5, r6
        adr     r5, .cnt_dma_1
        ldr     r2, [r5]

        add     r3, REG_DMA1CNT
        mov     r4, MEM_IWRAM
        add     r4, 0x40

        mov     r0, 0
        adr     r0, t001 + 1
        bx      r0

align 4
.wait_data:
        dw      0x00000014

.cnt_tmr:
        dw      0x00800009

.cnt_dma_0:
        dw      0xA1000001

.cnt_dma_1:
        dw      0x81000020

code16
align 2

t001:
        ; tests if dma can pause in between read
        ; and write cycles
        mov     r7, 0x30
        mov     r0, 1
loop1:
        sub     r7, r0
        cmp     r7, r0
        bne     loop1
        mov     r7, 0xFF

        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        str     r2, [r3]
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0



        ldr     r2, [r4]
        and     r2, r7
        cmp     r2, 0x09

        bne     f001a

        mov     r2, 0

        b       test_end

f001a:
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
