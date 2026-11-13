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
        mov     r10, MEM_EWRAM

        ; start sound and timer, dma should be triggered on first overflow
        str     r0, [r3, REG_TIM0CNT]
        str     r0, [r3, REG_TIM1CNT]
        mov     r4, r6
        adr     r4, .cnt_tmr
        ldr     r0, [r4]
        str     r0, [r3, REG_TIM0CNT]
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        ldr     r0, [r3, REG_TIM0CNT]
        ldr     r1, [r3, REG_TIM0CNT]
        ldr     r2, [r3, REG_TIM0CNT]
        ldr     r4, [r3, REG_TIM0CNT]
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        ldr     r9, [r10]
        ldr     r9, [r10]
        ldr     r9, [r10]
        mov     r0, r0
        mov     r0, r0
        ldr     r5, [r3, REG_TIM0CNT]
        ldr     r6, [r3, REG_TIM0CNT]
        ldr     r7, [r3, REG_TIM0CNT]
        ldr     r8, [r3, REG_TIM0CNT]
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        ldr     r9, [r10]
        ldr     r9, [r10]
        ldr     r9, [r10]
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        ldr     r9, [r3, REG_TIM0CNT]
        ldr     r10, [r3, REG_TIM0CNT]
        and     r0, 0xFF
        and     r1, 0xFF
        and     r2, 0xFF
        and     r4, 0xFF
        and     r5, 0xFF
        and     r6, 0xFF
        and     r7, 0xFF
        and     r8, 0xFF
        and     r9, 0xFF
        and     r10, 0xFF
        cmp     r0, 0
        bne     f001a
        cmp     r1, 0
        bne     f001b
        cmp     r2, 1
        bne     f001c
        cmp     r4, 1
        bne     f001d
        cmp     r5, 1
        bne     f001e
        cmp     r6, 2
        bne     f001f
        cmp     r7, 2
        bne     f001g
        cmp     r8, 2
        bne     f001h
        cmp     r9, 2
        bne     f001i
        cmp     r10, 3
        bne     f001j
        b       eval

.sound_on:
        dw      0x00000080

.cnt_tmr:
        dw      0x00810000

.cnt_tmr_2:
        dw      0x00800000

.tmr_read_a:
        dw      0x00800051

f001a:
        m_exit  1

f001b:
        m_exit  2

f001c:
        m_exit  3

f001d:
        m_exit  4

f001e:
        m_exit  5

f001f:
        m_exit  6

f001g:
        m_exit  7

f001h:
        m_exit  8

f001i:
        m_exit  9

f001j:
        m_exit  10

eval:
        m_vsync
        m_test_eval r12

idle:
        b       idle

include '../lib/text.asm'

main_end:
