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

        ; set waitstate
        mov     r4, r6
        adr     r4, .wait_data
        ldr     r0, [r4]
        str     r0, [r3, REG_WAITCNT]

        add     r3, REG_TIM0CNT

        mov     r1, MEM_EWRAM

        mov     r4, r6
        adr     r4, .cnt_tmr

        mov     r0, 0
        adr     r0, t001 + 1
        bx      r0

align 4
.wait_data:
        dw      0x00004014

.cnt_tmr:
        dw      0x00C0FFE0


code16
align 2

t001:
        ; tests what happens when IF is read
        ; around the timer is disabled
        mov     r7, 0x8
        mov     r0, 0
        mov     r2, 0
        str     r0, [r3]
        strh    r7, [r5]
        ldr     r0, [r4]
        str     r0, [r3]
        ldr     r0, [r4]
        mov     r0, r0
        ldr     r0, [r1]
        ldr     r0, [r1]
        mov     r0, r0
        str     r2, [r3]
        mov     r0, r0
        mov     r0, r0
        ldrh    r0, [r5]
        and     r0, r7
        cmp     r0, 0x0
        bne     f001a

        mov     r0, 0
        mov     r2, 0
        str     r0, [r3]
        strh    r7, [r5]
        ldr     r0, [r4]
        str     r0, [r3]
        ldr     r0, [r4]
        mov     r0, r0
        ldr     r0, [r1]
        ldr     r0, [r1]
        mov     r0, r0
        mov     r0, r0
        str     r2, [r3]
        mov     r0, r0
        mov     r0, r0
        ldrh    r0, [r5]
        and     r0, r7
        cmp     r0, 0x8
        bne     f001b

        mov     r0, 0
        mov     r2, 0
        str     r0, [r3]
        strh    r7, [r5]
        ldr     r0, [r4]
        str     r0, [r3]
        ldr     r0, [r4]
        mov     r0, r0
        ldr     r0, [r1]
        ldr     r0, [r1]
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        str     r2, [r3]
        mov     r0, r0
        mov     r0, r0
        ldrh    r0, [r5]
        and     r0, r7
        cmp     r0, 0x8
        bne     f001c

        mov     r0, 0
        mov     r2, 0
        str     r0, [r3]
        strh    r7, [r5]
        ldr     r0, [r4]
        str     r0, [r3]
        ldr     r0, [r4]
        mov     r0, r0
        ldr     r0, [r1]
        ldr     r0, [r1]
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        str     r2, [r3]
        mov     r0, r0
        mov     r0, r0
        ldrh    r0, [r5]
        and     r0, r7
        cmp     r0, 0x8
        bne     f001d

        b       test_end

f001a:
        mov     r2, 1
        bl      test_end

f001b:
        mov     r2, 2
        bl      test_end

f001c:
        mov     r2, 3
        bl      test_end

f001d:
        mov     r2, 4
        bl      test_end

test_end:
        mov     r0, 0
        str     r0, [r3]
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
