GAME_BG_OFF		equ $80	;word
GAME_CUR_DELAY	equ $82
GAME_CUR_TEXT	equ $83
GAME_CUR_FADE	equ $84
GAME_BG_PTR		equ $85
GAME_BG_PAGE	equ $86
GAME_SKIP		equ $87
GAME_CHANGE_CNT	equ $88	;word

GAME_CHANGE		equ 300

wellDone
	jsr waitNMI
	lda #0
	sta PPU_MASK

	ldx #LOW(wellDoneTable)
	ldy #HIGH(wellDoneTable)
	lda #$20
	sta PPU_ADDR
	lda #$00
	sta PPU_ADDR
	jsr unrle
	ldx #LOW(wellDoneTable)
	ldy #HIGH(wellDoneTable)
	lda #$24
	sta PPU_ADDR
	lda #$00
	sta PPU_ADDR
	jsr unrle

	lda #$31
	jsr wellDoneTextColor
	jsr resetPPUAdr

	lda #50
	sta <GAME_CUR_DELAY
	lda #0
	sta <GAME_CUR_OFF
	sta <GAME_CUR_TEXT
	sta <GAME_CUR_FADE
	sta <GAME_BG_PTR
	sta <GAME_BG_OFF
	lda #$01
	sta <GAME_BG_OFF+1
	lda #$21
	sta <GAME_BG_PAGE
	
	lda #LOW(GAME_CHANGE)
	sta <GAME_CHANGE_CNT
	lda #HIGH(GAME_CHANGE)
	sta <GAME_CHANGE_CNT+1
	
	jsr clearOAM
	
	lda #$3f			;put sprite 0 to the split position
	sta OAM_PAGE+0	;y
	lda #$ff			
	sta OAM_PAGE+1	;tile
	sty OAM_PAGE+2	;flags
	sty OAM_PAGE+3	;x

	lda #%00011110	;enable display, enable sprites
	sta PPU_MASK
	jsr waitNMI
	jsr updateOAM

	ldx #LOW(palDone)
	ldy #HIGH(palDone)
	jsr palFadeIn

	lda #BGM_DONE
	jsr bgmPlay

	jsr waitNMI
	jsr updateOAM

	lda #NMI_DONE
	jsr setNmiHandler

.wait
	jsr waitNMI
	jmp .wait


wellDoneTextColor
	ldx #$3f
	stx PPU_ADDR
	ldx #$0f
	stx PPU_ADDR
	sta PPU_DATA
	rts


nmiDone
	pha
	txa
	pha
	tya
	pha

	jsr updateOAM

	jsr ntscIsSkip
	lda #0
	bcc .1
	lda #1
.1
	sta <GAME_SKIP
	beq .draw
	jmp .noDraw
.draw

	lda #$20
	sta PPU_ADDR
	lda <GAME_CUR_OFF
	clc
	adc #$c2
	sta PPU_ADDR

	lda <GAME_CUR_DELAY
	beq .print
	lda <FRAME_CNT
	and #16
	beq .curBlink
	lda #$47
.curBlink
	sta PPU_DATA
	lda <GAME_CUR_DELAY
	cmp #$fe
	bcs .noDec
	dec <GAME_CUR_DELAY
.noDec
	jmp .noPrint

.print
	lda <GAME_CUR_FADE
	bne .noPrint
	lda <FRAME_CNT
	and #1
	beq .noPrint
	lda <GAME_CUR_DELAY
	bne .noPrint
	lda <GAME_CUR_OFF
	clc
	adc <GAME_CUR_TEXT
	tax
	lda wellDoneText,x
	bne .noBr
	lda #$ff
	sta <GAME_CUR_DELAY
	jmp .noPrint
	
.noBr
	sta PPU_DATA
	lda #$47
	sta PPU_DATA
	inc <GAME_CUR_OFF
.noPrint

	lda <GAME_CUR_DELAY
	cmp #$fe
	beq .skipH

	dec <GAME_CHANGE_CNT
	bne .skipH
	dec <GAME_CHANGE_CNT+1
	lda <GAME_CHANGE_CNT+1
	cmp #$ff
	bne .skipH
	
	lda <GAME_CUR_OFF
	beq .skipH
	lda <GAME_CUR_TEXT
	sec
	adc <GAME_CUR_OFF
	sta <GAME_CUR_TEXT
	tax
	lda wellDoneText,x
	bne .noStop
	lda #$fe
	sta <GAME_CUR_DELAY
	jmp .skipH
.noStop
	lda #20
	sta <GAME_CUR_FADE

	lda #LOW(GAME_CHANGE)
	sta <GAME_CHANGE_CNT
	lda #HIGH(GAME_CHANGE)
	sta <GAME_CHANGE_CNT+1
.skipH

	lda <GAME_CUR_FADE
	beq .noFade
	lsr a
	lsr a
	tax
	lda wellDoneFade,x
	jsr wellDoneTextColor
	dec <GAME_CUR_FADE
	lda <GAME_CUR_FADE
	bne .noFade
	lda #0
	sta <GAME_CUR_OFF
	lda #25
	sta <GAME_CUR_DELAY
	lda #$20
	sta PPU_ADDR
	lda #$c3
	sta PPU_ADDR
	lda #0
	ldx #28
.clearLine
	sta PPU_DATA
	dex
	bne .clearLine
	lda #$31
	jsr wellDoneTextColor
.noFade

.drawColumn
	lda <GAME_BG_OFF
	and #15
	beq .noSkip
	jmp .noDraw
.noSkip
	lda <GAME_BG_OFF
	clc
	adc #-16
	lsr a
	lsr a
	lsr a
	and #%00011110
	clc
	adc #$40
	sta <TEMP+1
	lda <GAME_BG_PAGE
	sta <TEMP+2
	lda #5
	sta <TEMP
	lda #1
	sta <TEMP+3
.drawCol0
	ldx <GAME_BG_PTR
	lda wellDoneBG,x
	and <TEMP+3
	bne .drawCol1
	ldx #$8d
	ldy #$a1
	jmp .drawCol2
.drawCol1
	ldx #$f8
	ldy #$f6
	lda <TEMP+1
	lsr a
	eor <TEMP
	and #1
	beq .drawCol2
	ldx #$f4
.drawCol2
	lda <TEMP+2
	sta PPU_ADDR
	lda <TEMP+1
	sta PPU_ADDR
	stx PPU_DATA
	inx
	stx PPU_DATA
	lda <TEMP+2
	sta PPU_ADDR
	lda <TEMP+1
	clc
	adc #32
	sta PPU_ADDR
	sty PPU_DATA
	iny
	sty PPU_DATA

	lda <TEMP+1
	clc
	adc #64
	sta <TEMP+1
	lda <TEMP+2
	adc #0
	sta <TEMP+2

	asl <TEMP+3
	dec <TEMP
	bne .drawCol0

	inc <GAME_BG_PTR
	ldx <GAME_BG_PTR
	lda wellDoneBG,x
	cmp #$ff
	bne .noDraw
	lda #0
	sta <GAME_BG_PTR
.noDraw

	jsr resetPPUAdr
	sta PPU_SCROLL	;A=0
	sta PPU_SCROLL

	jsr FamiToneUpdate

.sprite0hit0
	bit PPU_STATUS
	bvs .sprite0hit0
.sprite0hit1
	bit PPU_STATUS
	bvc .sprite0hit1

	ldx #50
.delay0
	dex
	bne .delay0

	lda #%00010100
	sta PPU_MASK
	lda #$01
	sta PPU_ADDR
	lda #$20
	sta PPU_ADDR

	inc <FRAME_CNT
	lda <FRAME_CNT
	and #%00010000
	ora #%10000000
	ora <GAME_BG_OFF+1
	sta PPU_CTRL

	lda <GAME_BG_OFF
	sta PPU_SCROLL
	lda #0
	sta PPU_SCROLL
	lda #%00011110
	sta PPU_MASK

	lda <GAME_SKIP
	bne .skip

	inc <GAME_BG_OFF
	lda <GAME_BG_OFF
	bne .noInc
	lda <GAME_BG_OFF+1
	eor #$01
	sta <GAME_BG_OFF+1
.noInc
	lda <GAME_BG_OFF
	cmp #16
	bne .noPageChange
	lda <GAME_BG_PAGE
	eor #$04
	sta <GAME_BG_PAGE
.noPageChange

.skip
	pla
	tay
	pla
	tax
	pla
	rti


wellDoneTable
	.incbin "welldone.rle"

wellDoneFade
	.db $0c,$0c,$1c,$11,$21,$2c

wellDoneText
	.db $ec,$84,$85,$f2,$e8,$ea,$87,$eb,$81,$ea,$87,$86,$84,$85,$e9,$f0,$fe,$00
	.db $f1,$84,$eb,$fe,$f3,$ea,$fa,$82,$fe,$ec,$84,$85,$85,$82,$ec,$87,$82,$ed,$fe,$00
	.db $ea,$81,$81,$fe,$87,$f3,$82,$fe,$ec,$84,$88,$fb,$eb,$87,$82,$e8,$e9,$fe,$00
	.db $85,$84,$fc,$fe,$f1,$84,$eb,$fe,$ea,$e8,$82,$fe,$87,$f3,$82,$fe,$81,$ea,$85,$fe,$88,$ea,$e9,$87,$82,$e8,$f0,$fe,$00
	.db $87,$f3,$ea,$85,$fd,$fe,$f1,$84,$eb,$fe,$d7,$84,$e8,$fe,$fb,$81,$ea,$f1,$86,$85,$f2,$f0,$fe,$00,$00

wellDoneBG
	.db %00011111
	.db %00001000
	.db %00011111
	.db %00000000
	.db %00011111
	.db %00010101
	.db %00010001
	.db %00000000
	.db %00011111
	.db %00010000
	.db %00010000
	.db %00000000
	.db %00011111
	.db %00010000
	.db %00010000
	.db %00000000
	.db %00000000
	.db %00000000
	.db %00011111
	.db %00010001
	.db %00001110
	.db %00000000
	.db %00011111
	.db %00010001
	.db %00011111
	.db %00000000
	.db %00011111
	.db %00000001
	.db %00011111
	.db %00000000
	.db %00011111
	.db %00010101
	.db %00010001
	.db %00000000
	.db %00010111
	.db %00000000
	.db %00000000
	.db %00000000
	.db %00000000
	.db %00000000
	.db %00000000
	.db $ff