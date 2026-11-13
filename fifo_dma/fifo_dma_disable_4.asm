format binary as 'gba'

include '../lib/constants.inc'
include '../lib/macros.inc'

macro m_exit test {
        mov     r12, test
        b       eval
}

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

        ; set up dma 1 for auto refill of FIFO A
        
        str     r0, [r3, REG_DMA1SAD]
        str     r0, [r3, REG_DMA1DAD]
        str     r0, [r3, REG_DMA1CNT]

        mov     r4, r6
        mov     r5, r6
        mov     r0, MEM_EWRAM
        adr     r4, .fifo_addr
        adr     r5, .fifo_dma
        ldr     r1, [r4]
        ldr     r2, [r5]

        str     r0, [r3, REG_DMA1SAD]
        str     r1, [r3, REG_DMA1DAD]
        str     r2, [r3, REG_DMA1CNT]

        ; set waitstate
        mov     r4, r6
        adr     r4, .wait_data
        ldr     r0, [r4]
        str     r0, [r3, REG_WAITCNT]

        b       t001

.wait_data:
        dw      0x00004014

.fifo_addr:
        dw      0x040000A0

.fifo_dma:
        dw      0xB7400004

t001:
        ; test timing of DMA with prefetch enabled
        mov     r10, MEM_IWRAM
        mov     r0, 0
        str     r0, [r3, REG_SNDCNT]
        str     r0, [r3, REG_SNDCNTX]

        ; start sound and timer, dma should be triggered on first overflow
        ; but we will disable DMA at this time, should crash

        str     r0, [r3, REG_TIM0CNT]
        str     r0, [r3, REG_TIM1CNT]
        mov     r4, r6
        adr     r4, .sound_on
        ldr     r0, [r4]
        str     r0, [r3, REG_SNDCNTX]
        mov     r4, r6
        mov     r5, r6
        adr     r4, .fifo_tmr
        adr     r5, .cnt_tmr
        ldr     r0, [r4]
        ldr     r1, [r5]
        str     r1, [r3, REG_TIM1CNT]
        str     r0, [r3, REG_TIM0CNT]
        mov     r0, 0
        mov     r1, MEM_EWRAM
        ldr     r0, [r1]
        ldr     r0, [r1]
        ldr     r0, [r1]
        ldr     r0, [r1]
        ldr     r0, [r1]
        ldr     r0, [r1]
        mov     r0, 0
        str     r0, [r3, REG_DMA1CNT]
        mov     r0, r0
        ldr     r0, [r3, REG_TIM1CNT]
        and     r0, 0xFF
        cmp     r0, 0x4A
        bne     f001a
        b       eval

.sound_on:
        dw      0x00000080

.fifo_tmr:
        dw      0x00C0FFC6

.cnt_tmr:
        dw      0x00800000


f001a:
        mov     r12, r0
        b       eval

eval:
        mov     r0, 0
        str     r0, [r3, REG_SNDCNTX]
        m_vsync
        m_test_eval r12

idle:
        b       idle

include '../lib/text.asm'

main_end:
