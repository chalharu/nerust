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

        str     r0, [r3, REG_DMA1SAD]
        str     r0, [r3, REG_DMA1DAD]
        str     r0, [r3, REG_DMA1CNT]

        ; set up dma 1 to start timer 1
        ; source address is IWRAM
        ; where we store value to enable timer
        mov     r0, MEM_IWRAM
        mov     r4, r6
        adr     r4, .tmr_data
        ldr     r1, [r4]
        str     r1, [r0]
        str     r0, [r3, REG_DMA1SAD]

        ; destination is timer register
        mov     r4, r6
        adr     r4, .tmr_addr
        ldr     r1, [r4]
        str     r1, [r3, REG_DMA1DAD]

        ; set DMA control to r1
        ; 32 bit write, 1 length
        mov     r4, r6
        adr     r4, .dma_data
        ldr     r1, [r4]

        ; set waitstate
        mov     r4, r6
        adr     r4, .wait_data
        ldr     r0, [r4]
        str     r0, [r3, REG_WAITCNT]

        b       t001

.wait_data:
        dw      0x00000014

.tmr_addr:
        dw      0x04000104

.tmr_data:
        dw      0x00800000

.dma_data:
        dw      0x84000001

t001:
        ; enable DMA and read timer
        mov     r7, MEM_EWRAM
        mov     r0, 0
        str     r0, [r3, REG_TIM1CNT]
        str     r1, [r3, REG_DMA1CNT]
        mov     r0, r0
        mov     r0, r0
        str     r0, [r3, REG_TIM1CNT]
        ldr     r0, [r3, REG_TIM1CNT]

        and     r0, 0xFF
        cmp     r0, 0x0B
        bne     f001a

        ; enable again, but this time 16 bit writes
        mov     r4, 1
        strh    r4, [r3, REG_DMA1CNT]
        mov     r5, 0xC6
        mov     r7, r1
        lsr     r1, 16

        mov     r0, 0
        str     r0, [r3, REG_TIM1CNT]
        strh    r1, [r3, r5]
        mov     r0, r0
        mov     r0, r0
        str     r0, [r3, REG_TIM1CNT]
        ldr     r0, [r3, REG_TIM1CNT]

        and     r0, 0xFF
        cmp     r0, 0x0B
        bne     f001b

        ; now try from bios
        mov     r0, MEM_IWRAM
        add     r0, 4
        str     r7, [r0]
        add     r3, 0xC4
        mov     r1, r3
        mov     r3, MEM_IO
        mov     r2, 2

        mov     r5, 0
        str     r5, [r3, REG_TIM0CNT]
        str     r5, [r3, REG_TIM1CNT]

        mov     r4, r6
        adr     r4, .tmr_data_2
        ldr     r5, [r4]
        str     r5, [r3, REG_TIM0CNT]

        swi     0x0B0000

        mov     r3, MEM_IO
        ldr     r0,[r3, REG_TIM0CNT]
        ldr     r1,[r3, REG_TIM1CNT]

        mov     r2, 0
        str     r2, [r3, REG_TIM0CNT]
        str     r2, [r3, REG_TIM1CNT]

        and     r0, 0xFF
        and     r1, 0xFF

        cmp     r0, 0x92
        bne     f001c

        cmp     r1, 0x3A
        bne     f001d

        b       eval

.tmr_data_2:
        dw      0x00800000

f001a:
        mov     r12, r0
        b       eval

f001b:
        mov     r12, r0
        b       eval

f001c:
        mov     r12, r0
        b       eval

f001d:
        mov     r12, r1
        b       eval

eval:
        m_vsync
        m_test_eval r12

idle:
        b       idle

include '../lib/text.asm'

main_end:
