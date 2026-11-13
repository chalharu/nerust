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

        mov     r0, r6
        mov     r4, r6
        mov     r5, r6
        adr     r0, .dma_data
        adr     r4, .fifo_addr
        adr     r5, .fifo_dma
        ldr     r1, [r4]
        ldr     r2, [r5]

        str     r0, [r3, REG_DMA1SAD]
        str     r1, [r3, REG_DMA1DAD]
        str     r2, [r3, REG_DMA1CNT]

        b       t001

.fifo_addr:
        dw      0x040000A0

.fifo_dma:
        dw      0xB7400004

.dma_data:
        dw      0x00000000

t001:
        ; Test readability of control bits while sound off
        mov     r4, r6
        mov     r5, r6
        adr     r4, .fifo_bits
        adr     r5, .fifo_read
        ldr     r0, [r4]
        ldr     r1, [r5]
        str     r0, [r3, REG_SNDCNT]
        ldr     r0, [r3, REG_SNDCNT]
        cmp     r0, r1
        beq     t002
        b       f001

.fifo_bits:
        dw      0xFFFF0000

.fifo_read:
        dw      0x770F0000

f001:
        m_exit  1

t002:
        ; Test if fifo can still be loaded while sound is off
        ; 24 samples should be loaded, but on console a DMA happens
        ; indicating no sampoles are loaded
        ; gives unexpected results on hardware
        mov     r0, 0
        str     r0, [r3, REG_SNDCNT]
        str     r0, [r3, REG_SNDFIFOA]
        str     r0, [r3, REG_SNDFIFOA]
        str     r0, [r3, REG_SNDFIFOA]
        str     r0, [r3, REG_SNDFIFOA]
        str     r0, [r3, REG_SNDFIFOA]
        str     r0, [r3, REG_SNDFIFOA]

        ; start sound and timer, dma should be triggered on first overflow
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
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        ldr     r0, [r3, REG_TIM1CNT]
        ldr     r7, [r3, REG_TIM1CNT]
        mov     r5, r6
        adr     r5, .tmr_read_a
        ldr     r1, [r5]
        cmp     r0, r1
        bne     f002a
        mov     r5, r6
        adr     r5, .tmr_read_b
        ldr     r1, [r5]
        cmp     r7, r1
        bne     f002b
        b       t003

.sound_on:
        dw      0x00000080

.fifo_tmr:
        dw      0x0080FFE0

.cnt_tmr:
        dw      0x00800000

.tmr_read_a:
        dw      0x00800068

.tmr_read_b:
        dw      0x0080008B

f002a:
        mov     r0, 0
        str     r0, [r3, REG_SNDCNTX]
        m_exit  2

f002b:
        mov     r0, 0
        str     r0, [r3, REG_SNDCNTX]
        m_exit  2

t003:
        ; Test if disabling clears the buffer
        ; Load 24 samples into the buffer then turn off sound
        mov     r0, 0
        str     r0, [r3, REG_TIM0CNT]
        str     r0, [r3, REG_TIM1CNT]
        str     r0, [r3, REG_SNDCNTX]
        mov     r4, r6
        adr     r4, .sound_on
        ldr     r0, [r4]
        str     r0, [r3, REG_SNDCNTX]
        mov     r0, 0
        str     r0, [r3, REG_SNDFIFOA]
        str     r0, [r3, REG_SNDFIFOA]
        str     r0, [r3, REG_SNDFIFOA]
        str     r0, [r3, REG_SNDFIFOA]
        str     r0, [r3, REG_SNDFIFOA]
        str     r0, [r3, REG_SNDFIFOA]
        str     r0, [r3, REG_SNDCNTX]

        ; start sound and timer, dma should be triggered on first overflow
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
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        ldr     r0, [r3, REG_TIM1CNT]
        mov     r5, r6
        adr     r5, .tmr_read
        ldr     r1, [r5]
        cmp     r0, r1
        bne     f003

        b       t004

.sound_on:
        dw      0x00000080

.fifo_tmr:
        dw      0x0080FFE0

.cnt_tmr:
        dw      0x00800000

.tmr_read:
        dw      0x00800068

f003:
        mov     r0, 0
        str     r0, [r3, REG_SNDCNTX]
        m_exit  3

t004:
        ; test DMA timing with different DMA settings
        ; unexpected results on hardware
        mov     r0, 0
        str     r0, [r3, REG_TIM0CNT]
        str     r0, [r3, REG_TIM1CNT]
        str     r0, [r3, REG_SNDCNT]
        str     r0, [r3, REG_SNDCNTX]
        mov     r4, r6
        adr     r4, .sound_on
        ldr     r0, [r4]
        str     r0, [r3, REG_SNDCNTX]
        mov     r4, r6
        adr     r4, .fifo_bits
        ldr     r0, [r4]
        str     r0, [r3, REG_SNDCNT]
        mov     r0, 0
        str     r0, [r3, REG_SNDCNT]
        str     r0, [r3, REG_SNDCNTX]


        ; set up dma 1 for auto refill of FIFO A
        str     r0, [r3, REG_DMA1SAD]
        str     r0, [r3, REG_DMA1DAD]
        str     r0, [r3, REG_DMA1CNT]

        mov     r4, r6
        mov     r5, r6
        adr     r4, .fifo_addr
        adr     r5, .fifo_dma
        ldr     r1, [r4]
        ldr     r2, [r5]
        mov     r4, r6
        adr     r4, .src_ofst
        ldr     r0, [r4]
        str     r0, [r3, REG_DMA1SAD]
        str     r1, [r3, REG_DMA1DAD]
        str     r2, [r3, REG_DMA1CNT]

        mov     r4, r6
        adr     r4, .sound_on
        ldr     r0, [r4]
        str     r0, [r3, REG_SNDCNTX]

        ; start sound and timer, dma should be triggered on first overflow
        mov     r4, r6
        mov     r5, r6
        adr     r4, .fifo_tmr
        adr     r5, .cnt_tmr
        ldr     r0, [r4]
        ldr     r1, [r5]
        str     r1, [r3, REG_TIM1CNT]
        str     r0, [r3, REG_TIM0CNT]
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        ldr     r0, [r3, REG_TIM1CNT]
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        ldr     r7, [r3, REG_TIM1CNT]
        mov     r5, r6
        adr     r5, .tmr_read_a
        ldr     r1, [r5]
        cmp     r0, r1
        bne     f004a
        mov     r5, r6
        adr     r5, .tmr_read_b
        ldr     r1, [r5]
        cmp     r7, r1
        bne     f004b

        b       eval

.fifo_bits:
        dw      0xFFFF0000

.src_ofst:
        dw      0x08000150

.sound_on:
        dw      0x00000080

.fifo_tmr:
        dw      0x0080FFE0

.cnt_tmr:
        dw      0x00800000

.tmr_read_a:
        dw      0x00800068

.tmr_read_b:
        dw      0x008000A5

.fifo_addr:
        dw      0x040000A0

.fifo_dma:
        dw      0xB7400004
        ;dw      0xB6C00004

f004a:
        and     r0, 0xFF
        mov     r12, r0
        mov     r0, 0
        str     r0, [r3, REG_SNDCNTX]
        b       eval
        m_exit  4

f004b:
        and     r7, 0xFF
        mov     r12, r7
        mov     r0, 0
        str     r0, [r3, REG_SNDCNTX]
        b       eval
        m_exit  4

eval:
        mov     r0, 0
        str     r0, [r3, REG_SNDCNTX]
        m_vsync
        m_test_eval r12

idle:
        b       idle

include '../lib/text.asm'

main_end:
