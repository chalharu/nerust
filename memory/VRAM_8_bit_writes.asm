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
        mov     r6, MEM_GAMEPAK0
        mov     r0, 0
        str     r0, [r3, REG_SNDCNT]
        str     r0, [r3, REG_SNDCNTX]

        ; set waitstate
        mov     r4, r6
        adr     r4, .wait_data
        ldr     r4, [r4]
        str     r4, [r3, REG_WAITCNT]

        mov     r4, r6
        adr     r4, .read_data
        ldr     r4, [r4]

        mov     r5, r6
        adr     r5, .read_data_2
        ldr     r5, [r5]

        mov     r0, 0
        adr     r0, t001
        bx      r0

align 4
.wait_data:
        dw      0x00004014

.read_data:
        dw      0x00005555

.read_data_2:
        dw      0x55550000

t001:
        ; test palram 8 bit writes
        mov     r0, MEM_VRAM
        mov     r3, MEM_VRAM
        mov     r1, 0
        str     r1, [r0]

        mov     r1, 0x55
        strb    r1, [r3]
        ldr     r1, [r0]
        cmp     r1, r4
        bne     f001a

        mov     r1, 0
        str     r1, [r0]

        add     r3, 1
        mov     r1, 0x55
        strb    r1, [r3]
        ldr     r1, [r0]
        cmp     r1, r4
        bne     f001b

        mov     r1, 0
        str     r1, [r0]

        add     r3, 1
        mov     r1, 0x55
        strb    r1, [r3]
        ldr     r1, [r0]
        cmp     r1, r5
        bne     f001c

        mov     r1, 0
        str     r1, [r0]

        add     r3, 1
        mov     r1, 0x55
        strb    r1, [r3]
        ldr     r1, [r0]
        cmp     r1, r5
        bne     f001d

        mov     r12, 0
        b       test_end

f001a:
        mov     r12, 1
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
