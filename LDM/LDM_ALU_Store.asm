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
        mov     r7, MEM_IWRAM

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

        mov     r0, 0
        b       t001

align 4
.wait_data:
        dw      0x00000014

.user_data:
        dw      0x00002481
        dw      0x00004218
        dw      0x000000F0
        dw      0x00000001


t001:
        ; tests LDM^ in various ways using mul and add
        ; currentlky in system mode, change to FIRQ
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

        mov     r5, r6
        adr     r5, .data_r9
        ldr     r9, [r5]

        mov     r5, r6
        adr     r5, .data_r3
        ldr     r3, [r5]

        mov     r5, r6
        adr     r5, .data_r4
        ldr     r4, [r5]

        ldm     r1, {r8, r9, r10}^
        mul     r10, r8, r9
        stm     r7, {r10}^
        ldr     r0, [r7]
        cmp     r0, r10

        beq     f001b

        ldm     r1, {r8, r9, r10}^
        add     r10, r9, r8
        stm     r7, {r10}^
        ldr     r0, [r7]
        cmp     r0, r10

        beq     f001c

        mov     r11, 0
        ldm     r1, {r8, r9, r10, r11}^
        add     r10, r8, r9, LSR r11
        stm     r7, {r10}^
        ldr     r0, [r7]
        cmp     r0, r10

        beq     f001d

        ldm     r1, {r8, r9, r10}^
        mov     r10, r3
        stm     r7, {r10}^
        ldr     r0, [r7]
        cmp     r0, r10

        beq     f001e

        mov     r10, 0
        ldm     r1, {r8, r9, r10}^
        mov     r0, r10
        cmp     r0, 0

        beq     f001f

        mov     r10, 0
        ldm     r1, {r8, r9, r10}^
        mov     r10, 0
        stm     r7, {r10}^
        ldr     r0, [r7]
        cmp     r0, 0xF0

        bne     f001g

        bl      eval

.mode_data:
        dw      0xF0000011

.data_r8:
        dw      0x00001248

.data_r9:
        dw      0x00008421

.data_r3:
        dw      0x000036C9

.data_r4:
        dw      0x0000C639

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

f001e:
        mov     r12, 5
        bl      eval

f001f:
        mov     r12, 6
        bl      eval

f001g:
        mov     r12, 7
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
