format binary as 'gba'

include '../lib/constants.inc'
include '../lib/macros.inc'

header:
        include '../lib/header.asm'

main:
        m_test_init
        ; Reset test register
        mov     r12, 0

        ; turn off IRQ
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

        ; set waitstate
        mov     r4, r6
        adr     r4, .wait_data
        ldr     r0, [r4]
        str     r0, [r3, REG_WAITCNT]

        mov     r4, r6
        adr     r4, .cnt_tmr

        b       t001

align 4
.wait_data:
        dw      0x00004014

align 4
.cnt_tmr:
        dw      0x00800000

.irq_hand:
        dw      0x03007FFC

irq_rt:
        mov     r0, 0
        mov     r3, MEM_IO
        str     r0, [r3, REG_IME]
        mov     r0, 1
        add     r3, 0x200
        add     r3, 0x2
        strh    r0, [r3]
        mov     r7, 1

        mov     r0, 0
        mov     r3, MEM_IO
        str     r0, [r3, REG_IME]
        add     r3, REG_TIM0CNT
        ldr     r5, [r3]

        mov     r15, r14

t001:
        ; tests timing of first vbl irq
        mov      r7, 0

        ; set vbl interrupt
        mov     r0, 1
        str     r0, [r3, REG_IE]
        str     r0, [r3, REG_IME]

        mov     r0, 8
        str     r0, [r3, 4]

        ; turn on timer
        mov     r3, MEM_IO
        add     r3, REG_TIM0CNT
        mov     r0, 0
        str     r0, [r3]
        ldr     r0, [r4]
        str     r0, [r3]
        mov     r3, MEM_IO

        ; wait for next vbl
vloop2:
        ldrh     r0, [r3, 6]
        and     r0, 0xFF
        cmp     r0, 0xA0
        bne     vloop2

        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0

        ; interrupt should have occured
        cmp     r7, 1
        bne     f001a

        mov     r0, r5
        mov     r1, r5
        mov     r7, 0xFF

        and     r0, r7
        lsr     r1, 8
        and     r1, r7
        cmp     r0, 0x35
        bne     f001b
        cmp     r1, 0x9E
        bne     f001c

        mov     r7, 0
        b       eval

f001a:
        mov     r7, 1
        b       eval

f001b:
        mov     r7, r0
        b       eval

f001c:
        mov     r7, r1
        b       eval

eval:

        ; disable interrupts
        mov     r0, 0
        str     r0, [r3, REG_IE]
        str     r0, [r3, REG_IME]

        m_vsync
        m_test_eval r7

idle:
        b       idle

include '../lib/text.asm'

main_end:
