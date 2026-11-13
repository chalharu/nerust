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

        mov     r5, MEM_EWRAM

        mov     r4, r6
        adr     r4, .cnt_tmr
        ldr     r0, [r4]
        str     r0, [r3]

        mov     r4, r6
        adr     r4, .bus_read
        ldr     r1, [r4]

        mov     r4, r6
        adr     r4, .bus_read_2
        ldr     r2, [r4]

        mov     r0, 0
        adr     r0, t001
        bx      r0

align 4
.wait_data:
        dw      0x00004014

.cnt_tmr:
        dw      0x0080FF00

.irq_hand:
        dw      0x03007FFC

.bus_read:
        dw      0xE20330FF

.bus_read_2:
        dw      0x1A000008

t001:
        ; test coprocessor instructions on cp14
        ; all others besides MRC / MCR are undef
        ; MCR / MRC only work with cp14
        ; but the debugger is disabled so they have no effect

        ; add an instruction to prefetcher
        ldr     r0, [r5]
        ldr     r0, [r5]
        ldr     r0, [r5]
        ldr     r0, [r5]
        ldr     r0, [r5]
        ldr     r0, [r5]

        MRC     p14, 0, r0, c0, c0, 0

        ; check timing
        ldr     r3, [r3]
        and     r3, 0xFF
        cmp     r3, 0x75
        bne     f001a

        cmp     r0, r1
        bne     f001b

        MRC     p14, 0, r0, c1, c0, 0
        cmp     r0, r2
        bne     f001c


        ; destination r15 updates flags (in this case check N flag
        MRC     p14, 0, r15, c1, c0, 0
        bpl     f001d

        mov     r12, 0

        ; shouldn't crash, not otherwise tested
        MCR     p14, 0, r1, c1, c0, 0

        b       test_end

f001a:
        mov     r12, r3
        bl      test_end

f001b:
        mov     r12, 2
        bl      test_end

f001c:
        mov     r12, 3
        bl      test_end

f001d:
        mov     r12, 4
        bl      test_end

test_end:
        mov     r0, 0
        str     r0, [r3]
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
