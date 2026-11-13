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
        str     r0, [r3, REG_IE]
        str     r0, [r3, REG_IME]

        ; set waitstate
        mov     r4, r6
        adr     r4, .wait_data
        ldr     r0, [r4]
        str     r0, [r3, REG_WAITCNT]

        ; put data pointer in reg 1
        mov     r1, r6
        adr     r1, .user_data

        ; move some data to RAM to be read out later
        mov    r2, MEM_IWRAM
        mov    r0, 1
        str    r0, [r2]
        add    r2, 4
        mov    r0, 2
        str    r0, [r2]
        add    r2, 4
        mov    r0, 3
        str    r0, [r2]
        add    r2, 4
        mov    r0, 0
        str    r0, [r2]


        mov     r0, 0
        b       t001

align 4
.wait_data:
        dw      0x00000014

.user_data:
        dw      0x00000004
        dw      0x00000004
        dw      0x00000004

t001:
        ; tests LDM^ with mul ul and sl
        ; currently in system mode, change to FIRQ
        mov     r5, r6
        adr     r5, .mode_data
        ldr     r2, [r5]
        msr     cpsr, r2
        mrs     r2, cpsr
        and     r2, 0xFF
        cmp     r2, 0x11
        bne     f001a

        mov     r5, r6
        adr     r5, .data_r8
        ldr     r8, [r5]

        mov     r9, r8
        mov     r10, 0

        ldm     r1, {r8}^
        ldr     r0, [r8]

        cmp     r0, 2
        bne     f001b

        ldm     r1, {r8,r9,r10}^
        ldr     r0, [r9, +r10]

        cmp     r0, 3
        bne     f001c

        ldm     r1, {r8, r9, r10}^
        ldm     r8, {r8, r9, r10}^
        ldr     r0, [r9, +r10]
        mov     r1, 0x100

        cmp     r0, r1
        bne     f001d

        mov     r12, 0
        bl      eval

.mode_data:
        dw      0xF0000011

.data_r8:
        dw      0x03000000


f001a:
        mov     r12, 1
        bl      eval

f001b:
        mov     r12, 2
        bl      eval

f001c:
        mov     r12, 3
        bl      eval

f001d:
        mov     r12, 4
        bl      eval

eval:
        mov     r0, r12
        mov     r1, 0x1F
        msr     cpsr, r1

        m_vsync
        m_test_eval r0

idle:
        b       idle

include '../lib/text.asm'

main_end:
