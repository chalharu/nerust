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

        ; enable serial port IRQ
        mov     r0, 0x80
        str     r0, [r3, REG_IE]
        mov     r0, 1
        str     r0, [r3, REG_IME]

        ; set waitstate
        mov     r4, r6
        adr     r4, .wait_data
        ldr     r0, [r4]
        str     r0, [r3, REG_WAITCNT]

        ; start timer 0
        mov     r4, r6
        adr     r4, .cnt_tmr
        ldr     r0, [r4]
        str     r0, [r3, REG_TIM0CNT]

        ; move necessary data to serial regs to start
        mov     r2, 0
        mov     r4, 0
        add     r4, 0x134
        strh    r0, [r3, r4]

        mov     r2, r6
        adr     r2, .cnt_ser
        ldr     r2, [r2]
        mov     r4, r3
        add     r3, REG_SIOCNT
        add     r4, REG_TIM0CNT

        mov     r1, MEM_EWRAM

        mov     r0, 0
        adr     r0, t001 + 1
        bx      r0

align 4
.wait_data:
        dw      0x00004014

.cnt_tmr:
        dw      0x00800000

.cnt_ser:
        dw      0x00004083

.irq_hand:
        dw      0x03007FFC

irq_rt:
        mov     r0, 0
        mov     r3, MEM_IO
        str     r0, [r3, REG_IME]
        ldr     r7, [r4]
        mov     r15, r14


code16
align 2

t001:
        ; tests timing of serial IRQ and timing source

        ; add some instructions to the prefetcher
        ldr     r0, [r1]
        ldr     r0, [r1]
        ldr     r0, [r1]
        mov     r0, r0
        mov     r0, r0
        str     r2, [r3]
        swi     2

        mov     r6, 0xFF
        mov     r5, r7
        and     r7, r6
        mov     r4, 8
        lsr     r5, r4
        and     r5, r6

        cmp     r7, 0xDA
        bne     f001a

        cmp     r5, 0x0
        bne     f001b

        mov     r2, 0
        b       test_end

f001a:
        mov     r2, r7
        bl      test_end

f001b:
        mov     r2, 2
        bl      test_end

test_end:
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
