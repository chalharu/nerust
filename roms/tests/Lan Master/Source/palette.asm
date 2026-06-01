palReset
	lda #$3f
	sta PPU_ADDR
	lda #$00
	sta PPU_ADDR
	lda #$0f
	ldx #32
.1
	sta PPU_DATA
	dex
	bne .1
	rts


palFadeIn
	stx <PAL_LOW
	sty <PAL_HIGH

	lda #LOW(palBrightTable+3*64)
	sta <PAL_BR_LOW
	lda #HIGH(palBrightTable+3*64)
	sta <PAL_BR_HIGH

	lda #16
	sta <PAL_CNT
.1
	jsr waitNMI50

	lda <PAL_CNT
	and #$03
	bne .3

	lda #$3f
	sta PPU_ADDR
	lda #$00
	sta PPU_ADDR

	lda #0
.2
	pha
	tay
	lda [PAL_LOW],y
	tay
	lda [PAL_BR_LOW],y
	sta PPU_DATA
	pla
	adc #1
	cmp #16+4
	bne .2

	jsr resetPPUAdr

	clc
	lda <PAL_BR_LOW
	adc #-64
	sta <PAL_BR_LOW
	lda <PAL_BR_HIGH
	adc #-1
	sta <PAL_BR_HIGH

.3
	jsr FamiToneUpdate
	dec <PAL_CNT
	bne .1

	rts



palFadeOut
	stx <PAL_LOW
	sty <PAL_HIGH

	lda #LOW(palBrightTable)
	sta <PAL_BR_LOW
	lda #HIGH(palBrightTable)
	sta <PAL_BR_HIGH

	lda #16
	sta <PAL_CNT
.1
	jsr waitNMI50

	lda <PAL_CNT
	and #$03
	bne .3

	lda #$3f
	sta PPU_ADDR
	lda #$00
	sta PPU_ADDR

	lda #0
.2
	pha
	tay
	lda [PAL_LOW],y
	tay
	lda [PAL_BR_LOW],y
	sta PPU_DATA
	pla
	adc #1
	cmp #16+4
	bne .2

	jsr resetPPUAdr

	clc
	lda <PAL_BR_LOW
	adc #64
	sta <PAL_BR_LOW
	lda <PAL_BR_HIGH
	adc #0
	sta <PAL_BR_HIGH

.3
	jsr FamiToneUpdate
	dec <PAL_CNT
	bne .1

	jsr waitNMI50
	jsr palReset
	jsr resetPPUAdr
	jsr FamiToneUpdate
	jsr waitNMI50
	jsr FamiToneUpdate

	rts


palBrightTable
	.db $00,$01,$02,$03,$04,$05,$06,$07,$08,$09,$0a,$0b,$0c,$0f,$0e,$0f
	.db $10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$1a,$1b,$1c,$1f,$1e,$1f
	.db $20,$21,$22,$23,$24,$25,$26,$27,$28,$29,$2a,$2b,$2c,$2d,$2e,$2f
	.db $30,$31,$32,$33,$34,$35,$36,$37,$38,$39,$3a,$3b,$3c,$3d,$3e,$3f
	.db $0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f
	.db $00,$01,$02,$03,$04,$05,$06,$07,$08,$09,$0a,$0b,$0c,$0f,$0e,$0f
	.db $10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$1a,$1b,$1c,$1f,$1e,$1f
	.db $20,$21,$22,$23,$24,$25,$26,$27,$28,$29,$2a,$2b,$2c,$2d,$2e,$2f
	.db $0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f
	.db $0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f
	.db $00,$01,$02,$03,$04,$05,$06,$07,$08,$09,$0a,$0b,$0c,$0f,$0e,$0f
	.db $10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$1a,$1b,$1c,$1f,$1e,$1f
	.db $0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f
	.db $0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f
	.db $0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f,$0f
	.db $00,$01,$02,$03,$04,$05,$06,$07,$08,$09,$0a,$0b,$0c,$0f,$0e,$0f

palTitle
	.db $0c,$31,$11,$21,$0c,$32,$11,$30,$0c,$30,$1f,$31,$0c,$21,$11,$21,$0c,$00,$00,$00
palGame
	.db $0c,$30,$1c,$31,$0c,$31,$1f,$1c,$0c,$31,$1c,$30,$0c,$21,$21,$21,$0c,$10,$0c,$30
palDone
	.db $0c,$30,$1c,$31,$0c,$31,$1f,$1c,$0c,$0c,$1c,$31,$0c,$21,$21,$31,$0c,$0c,$1c,$31