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
        mov     r7, MEM_IO
        add     r7, 6
        add     r5, 0x200
        add     r5, 2

        mov     r6, MEM_GAMEPAK0
        mov     r0, 0
        str     r0, [r3, REG_SNDCNT]
        str     r0, [r3, REG_SNDCNTX]

        ; turn on vbl irq and timer 0 irq
        mov     r0, 1
        str     r0, [r3, REG_IME]

        mov     r0, 9
        str     r0, [r3, REG_IE]

        mov     r0, 8
        str     r0, [r3, REG_DISPSTAT]

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

        add     r3, REG_TIM0CNT

        mov     r4, r6
        adr     r4, .cnt_tmr
        ldr     r4, [r4]

        mov     r0, 0
        adr     r0, t001 + 1
        bx      r0

align 4
.wait_data:
        dw      0x00004014

.cnt_tmr:
        dw      0x00C0FFE0

.irq_hand:
        dw      0x03007FFC

irq_rt:
        ; clear vbl interrupt
        ldrh    r0, [r5]
        mov     r1, 1
        and     r0, r1
        cmp     r0, r1
        bne     timer_only
        mov     r0, 1
        strh    r0, [r5]
        ; enable timer
        ; and wait for interrupt
        str     r4, [r3]
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

        ; return, which should trigger another interrupt
        mov     r0, 0x138
        bx      r0

timer_only:
        mov     r0, 0
        mov     r5, MEM_IO
        str     r0, [r5, REG_IME]

        mov     r12, MEM_GAMEPAK0
        adr     r12, .irq_ret2
        ldr     r12, [r12]

        mov     r0, 1
        add     r12, r0
        bx      r12


.irq_ret2:
        dw      0x08000228

code16
align 2

t001:
        ; tests what happens when sub triggers int
        mov     r1, 160
wait_vbl:
        ldrh    r0, [r7]
        cmp     r0, r1
        bne     wait_vbl


        ; check timer value
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0

        ldr     r0, [r3]
        mov     r7, 0xFF
        and     r0, r7
        mov     r1, 0xF8

        cmp     r0, r1
        bne     f001a

        mov     r0, 0
        mov     r12, r0
        b       test_end

f001a:
        mov     r12, r0
        bl      test_end


test_end:
        adr     r0, eval
        bx      r0

code32
align 4
eval:
        m_vsync
        m_test_eval r12

idle:
        b       idle

include '../lib/text.asm'

main_end:
