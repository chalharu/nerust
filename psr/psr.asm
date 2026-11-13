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

        ; set up IRQ handling
        mov     r4, r6
        adr     r4, .irq_hand
        ldr     r0, [r4]
        mov     r4, r6
        adr     r4, irq_rt
        str     r4, [r0]
        mov     r0, 8
        str     r0, [r3, REG_IE]
        mov     r0, 1
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
        b       t001

align 4
.wait_data:
        dw      0x00004014

.cnt_tmr:
        dw      0x00C0FFD8

.irq_hand:
        dw      0x03007FFC

irq_rt:
        mov     r0, 0
        mov     r3, MEM_IO
        str     r0, [r3, REG_IME]
        add     r3, REG_TIM0CNT
        ldr     r8, [r3]
        and     r8, 0xFF
        str     r0, [r3]
        mov     r7, r6
        mov     r3, MEM_IO
        adr     r7, .irq_flag
        ldr     r7, [r7]
        str     r7, [r3, REG_IE]

        mov     r0, 1
        str     r0, [r3, REG_IME]
        add     r3, REG_TIM0CNT

        mov     r15, r14

.irq_flag:
        dw      0x00080000

t001:
        ; tests reading / writing MSR in different modes
        ; currently in system mode
        ; change to abort mode and back
        mov     r5, r6
        adr     r5, .mode_data
        ldr     r2, [r5]
        ;msr     spsr_c, r2
        dw      0xE161F002
        mrs     r1, spsr
        and     r1, 0xFF
        cmp     r1, 0x1F
        bne     f001a

        msr     cpsr_c, r2
        ;msr     spsr_c, r2
        dw      0xE161F002
        mrs     r1, spsr
        and     r1, 0xFF
        cmp     r1, 0x17
        bne     f001b

        mov     r5, r6
        adr     r5, .mode_data_2
        ldr     r2, [r5]
        msr     cpsr_c, r2
        mrs     r1, spsr
        and     r1, 0xFF
        cmp     r1, 0x1F
        bne     f001c

        ; try to change to an invalid mode
        ; with upper bit clear
        ; documentation says this is unrecoverable
        ; but works fine on hardware
        ; (keeps upper bit set)
        mov     r5, r6
        adr     r5, .mode_data
        ldr     r2, [r5]
        msr     cpsr_c, r2
        and     r2, 0x03
        msr     cpsr_c, r2
        mrs     r1, cpsr
        and     r1, 0xFF
        cmp     r1, 0x13
        bne     f001d

        mov     r1, 0x17
        msr     cpsr_c, r1
        mov     r1, MEM_EWRAM

        mov     r2, 0x9F
        ;msr     spsr_c, r2
        dw      0xE161F002

        ; check timing of changing interrupt
        ; disable flag
        mov     r9, 0x8
        mov     r8, 0x8
        mov     r0, 0
        ldr     r0, [r4]
        str     r0, [r3]
        ldr     r0, [r4]
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        ldr     r0, [r1]
        ldr     r0, [r1]
        mov     r0, r0
        ;mov     r0, r0
        ; add r7 to r15
        dw      0xE13FF009
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0
        mov     r0, r0

        mov     r1, 0x1F
        msr     cpsr_c, r1

        cmp     r8, 0xE4
        bne     f001e

        bl      eval

.mode_data:
        dw      0xF0000017

.mode_data_2:
        dw      0xF000001F

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
        mov     r1, 0x1F
        msr     cpsr_c, r1
        mov     r12, 4
        bl      eval

f001e:
        mov     r12, r8
        bl      eval

eval:
        m_vsync
        m_test_eval r12

idle:
        b       idle

include '../lib/text.asm'

main_end:
