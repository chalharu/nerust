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
        mov     r0, MEM_IWRAM
        adr     r4, .fifo_addr

        ldr     r1, [r4]
        ldr     r2, [r5]

        str     r0, [r3, REG_DMA1SAD]
        str     r1, [r3, REG_DMA1DAD]


        ; set waitstate
        mov     r4, r6
        adr     r4, .wait_data
        ldr     r0, [r4]
        str     r0, [r3, REG_WAITCNT]

        b       t001

.wait_data:
        dw      0x00004010

.fifo_addr:
        dw      0x040000A0

t001:
        ; start sound and timer, dma should be triggered on first overflow
        str     r0, [r3, REG_TIM0CNT]

        mov     r2, MEM_EWRAM
        mov     r4, r6
        mov     r5, r6
        adr     r4, .cnt_tmr
        adr     r5, .dma_cnt
        ldr     r0, [r4]
        ldr     r1, [r5]
        str     r0, [r3, REG_TIM0CNT]
        ldr     r0, [r2]
        ldr     r0, [r2]
        ldr     r0, [r2]
        str     r1, [r3, REG_DMA1CNT]
        str     r1, [r5]
        mov     r0, r0
        ldr     r0, [r3, REG_TIM0CNT]
        and     r0, 0xFF
        cmp     r0, 0x32
        bne     f001a

        mov     r0, 0
        str     r0, [r3, REG_TIM0CNT]
        str     r6, [r3, REG_DMA1SAD]
        mov     r4, r6
        adr     r4, .wait_data_2
        ldr     r0, [r4]
        ;str     r0, [r3, REG_WAITCNT]
        mov     r4, r6
        mov     r5, r6
        adr     r4, .cnt_tmr
        adr     r5, .dma_cnt
        ldr     r0, [r4]
        ldr     r1, [r5]
        str     r0, [r3, REG_TIM0CNT]
        ldr     r0, [r2]
        ldr     r0, [r2]
        ldr     r0, [r2]
        mov     r0, r0
        str     r1, [r3, REG_DMA1CNT]
        mov     r0, r0
        mov     r0, r0
        str     r1, [r2]
        mov     r0, r0
        ldr     r0, [r3, REG_TIM0CNT]
        and     r0, 0xFF
        cmp     r0, 0x38
        bne     f001b


        b       eval

.sound_on:
        dw      0x00000080

.cnt_tmr:
        dw      0x00800000

.dma_cnt:
        dw      0x86400001

.wait_data_2:
        dw      0x00004014

f001a:
        mov     r12, r0
        b       eval
        m_exit  1

f001b:
        mov     r12, r0
        b       eval
        m_exit  1

eval:
        m_vsync
        m_test_eval r12

idle:
        b       idle

include '../lib/text.asm'

main_end:
