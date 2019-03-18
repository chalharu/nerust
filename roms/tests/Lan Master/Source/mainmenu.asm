mainMenu
	jsr waitNMI
	lda #0
	sta PPU_MASK

	jsr mainMenuSetSoundItems

	ldx #LOW(titleNameTable)
	ldy #HIGH(titleNameTable)
	lda #$20
	sta PPU_ADDR
	lda #$00
	sta PPU_ADDR
	jsr unrle

	jsr clearOAM

	lda #%00001110	;enable display, disable sprites
	sta PPU_MASK

	ldx #LOW(palTitle)
	ldy #HIGH(palTitle)
	jsr palFadeIn

	lda #0
	sta <MENU_CUR
	sta <MENU_CNT
	sta <MENU_CODECUR
	sta <MENU_ANIM
	jsr rand
	and #%00010000
	ora #%10000000
	sta <MENU_SET

	lda #BGM_MENU
	jsr bgmPlay

mainMenuLoop
	jsr waitNMI50
	lda <MENU_SET
	sta PPU_CTRL
	jsr mainMenuShowCur
	jsr mainMenuShowCurAttr
	jsr mainMenuAnim
	jsr FamiToneUpdate

	jsr padPoll
	ldx PAD_STATET
	txa
	and #PAD_UP
	bne mainMenuCurUp
	txa
	and #(PAD_DOWN|PAD_SELECT)
	bne mainMenuCurDown
	txa
	and #(PAD_A|PAD_B|PAD_START)
	bne mainMenuCurSelect

	inc <MENU_CNT

    jmp mainMenuLoop


mainMenuCurUp
	ldx <MENU_CUR
	dex
	cpx #255
	bne .1
	ldx #3
.1
	stx <MENU_CUR
	lda #0
	sta <MENU_CNT
	lda #SFX_CURSOR
	jsr sfxPlay
	jmp mainMenuLoop


mainMenuCurDown
	ldx <MENU_CUR
	inx
	cpx #4
	bne .1
	ldx #0
.1
	stx <MENU_CUR
	lda #0
	sta <MENU_CNT
	lda #SFX_CURSOR
	jsr sfxPlay
	jmp mainMenuLoop


mainMenuCurSelect
	lda <MENU_CUR
	cmp #0
	bne .1
	lda #0
	sta <GAME_LEVEL
	ldy #5
	jmp mainMenuStart
.1
	cmp #1
	bne .2
	lda #SFX_SELECT
	jsr sfxPlay
	jmp mainMenuEnterCode
.2
	cmp #2
	bne .3
	inc <GAME_SFX
	jsr waitNMI
	jsr mainMenuSetSoundItems
	jsr FamiToneUpdate
	lda #SFX_SELECT
	jsr sfxPlay
	jmp mainMenuLoop
.3
	cmp #3
	bne .5
	lda #0
	sta <MENU_ANIM
	inc <GAME_BGM
	lda <GAME_BGM
	and #1
	beq .4
	lda #BGM_MENU
.4
	jsr bgmPlay
	jsr waitNMI
	jsr mainMenuSetSoundItems
	jsr FamiToneUpdate
	lda #SFX_SELECT
	jsr sfxPlay
	jmp mainMenuLoop
.5
	jmp mainMenuLoop


mainMenuStart
	tya
	pha

	lda #BGM_NONE
	jsr bgmPlay

	lda #SFX_START
	jsr sfxPlay

	lda #0
	sta <MENU_CNT
	jsr waitNMI
	jsr mainMenuShowCur
	jsr FamiToneUpdate
	pla
	tay

	ldx #5
.1
	txa
	pha
	tya
	pha
	sta <MENU_CUR
	jsr waitNMI50
	jsr FamiToneUpdate
	jsr waitNMI50
	jsr mainMenuShowCurAttr
	jsr FamiToneUpdate
	lda #4
	sta <MENU_CUR
	jsr waitNMI50
	jsr FamiToneUpdate
	jsr waitNMI50
	jsr mainMenuShowCurAttr
	jsr FamiToneUpdate
	pla
	tay
	pla
	tax
	dex
	bne .1

	ldx #LOW(palTitle)
	ldy #HIGH(palTitle)
	jsr palFadeOut

	rts		;from mainmenu


mainMenuShowCur
	lda #$22
	sta <MENU_CURADRH
	lda #$55
	sta <MENU_CURADRL
	ldx #4
	ldy #0
.1
	lda <MENU_CURADRH
	sta PPU_ADDR
	lda <MENU_CURADRL
	sta PPU_ADDR
	lda <MENU_CNT
	and #$10
	eor #$10
	beq .2
	lda #$00
	cpy <MENU_CUR
	bne .2
	lda #$47
.2
	sta PPU_DATA

	clc
	lda <MENU_CURADRL
	adc #64
	sta <MENU_CURADRL
	lda <MENU_CURADRH
	adc #0
	sta <MENU_CURADRH

	iny
	dex
	bne .1

	jmp resetPPUAdr


mainMenuShowCurAttr
	clc
	lda <MENU_CUR
	adc <MENU_CUR
	adc <MENU_CUR
	asl a
	tax

	lda #$23
	sta PPU_ADDR
	lda #$e5
	sta PPU_ADDR
	lda menuColors,x
	sta PPU_DATA
	inx
	lda menuColors,x
	sta PPU_DATA
	inx
	lda #$23
	sta PPU_ADDR
	lda #$ed
	sta PPU_ADDR
	lda menuColors,x
	sta PPU_DATA
	inx
	lda menuColors,x
	sta PPU_DATA
	inx
	lda #$23
	sta PPU_ADDR
	lda #$f5
	sta PPU_ADDR
	lda menuColors,x
	sta PPU_DATA
	inx
	lda menuColors,x
	sta PPU_DATA

	jmp resetPPUAdr


mainMenuSetSoundItems
	lda #$22
	sta PPU_ADDR
	lda #$da
	sta PPU_ADDR
	lda <GAME_SFX
	and #$01
	beq .1
	lda #$53
.1
	sta PPU_DATA
	lda #$23
	sta PPU_ADDR
	lda #$1a
	sta PPU_ADDR
	lda <GAME_BGM
	and #$01
	beq .2
	lda #$53
.2
	sta PPU_DATA

	jmp resetPPUAdr


mainMenuAnim
	lda <GAME_BGM
	and #1
	bne .bgm
	lda <MENU_ANIM
	beq .1
	dec <MENU_ANIM
	rts
.1
	jsr rand
	and #15
	sta <MENU_ANIM
	lda <MENU_SET
	eor #$10
	sta <MENU_SET
	rts

.bgm
	lda <MENU_ANIM
	bne .2
	lda FT_MR_NOISE_V
	and #15
	bne .3
	lda #1
	sta <MENU_ANIM
	rts
.2
	lda FT_MR_NOISE_F
	and #$80
	beq .3
	lda FT_MR_NOISE_V
	and #15
	beq .3
	lda #0
	sta <MENU_ANIM
	lda <MENU_SET
	eor #$10
	sta <MENU_SET
	rts
.3
	lda <FRAME_CNT
	and #$1f
	bne .4
	lda <MENU_SET
	eor #$10
	sta <MENU_SET
.4
	rts

mainMenuEnterCode
	jsr waitNMI50
	lda <MENU_SET
	sta PPU_CTRL

	lda #$22
	sta PPU_ADDR
	lda #$95
	sta PPU_ADDR

	ldx #0
	stx PPU_DATA
	stx PPU_DATA
.1
	lda <GAME_CODE,x
	clc
	adc #$57
	sta PPU_DATA
	inx
	cpx #4
	bne .1

	jsr mainMenuEnterCodeCur
	jsr mainMenuAnim
	jsr FamiToneUpdate

	jsr padPoll
	ldx PAD_STATET
	txa
	and #PAD_UP
	bne mainMenuCodeUp
	txa
	and #PAD_DOWN
	bne mainMenuCodeDown
	txa
	and #PAD_LEFT
	bne mainMenuCodeLeft
	txa
	and #PAD_RIGHT
	bne mainMenuCodeRight
	txa
	and #(PAD_START|PAD_A|PAD_B)
	bne mainMenuCodeSelect
	txa
	and #PAD_SELECT
	beq .2
	jmp mainMenuCodeReturn
.2

	inc <MENU_CNT

	jmp mainMenuEnterCode


mainMenuCodeUp
	ldx <MENU_CODECUR
	inc <GAME_CODE,x
	lda <GAME_CODE,x
	cmp #10
	bne .1
	lda #0
.1
	sta <GAME_CODE,x
	lda #SFX_TEXT
	jsr sfxPlay
	jmp mainMenuEnterCode


mainMenuCodeDown
	ldx <MENU_CODECUR
	dec <GAME_CODE,x
	lda <GAME_CODE,x
	cmp #255
	bne .1
	lda #9
.1
	sta <GAME_CODE,x
	lda #SFX_TEXT
	jsr sfxPlay
	jmp mainMenuEnterCode


mainMenuCodeLeft
	dec <MENU_CODECUR
	lda <MENU_CODECUR
	cmp #255
	bne .1
	lda #3
.1
	sta <MENU_CODECUR
	lda #0
	sta <MENU_CNT
	lda #SFX_TEXT
	jsr sfxPlay
	jmp mainMenuEnterCode


mainMenuCodeRight
	inc <MENU_CODECUR
	lda <MENU_CODECUR
	cmp #4
	bne .1
	lda #0
.1
	sta <MENU_CODECUR
	lda #0
	sta <MENU_CNT
	lda #SFX_TEXT
	jsr sfxPlay
	jmp mainMenuEnterCode


mainMenuCodeSelect
	jsr waitNMI50
	lda #16
	sta <MENU_CNT
	jsr mainMenuEnterCodeCur
	jsr FamiToneUpdate

	lda <GAME_CODE
	cmp #6
	bne .noSfxTest
	lda <GAME_CODE+1
	cmp #8
	beq mainMenuSfxTest
.noSfxTest

	ldy #LEVELS_COUNT
	ldx #0
.1
	lda passwords,x
	and #$0f
	cmp <GAME_CODE+3
	bne .2
	lda passwords,x
	ror a
	ror a
	ror a
	ror a
	and #$0f
	cmp <GAME_CODE+2
	bne .2
	lda passwords+1,x
	and #$0f
	cmp <GAME_CODE+1
	bne .2
	lda passwords+1,x
	ror a
	ror a
	ror a
	ror a
	and #$0f
	cmp <GAME_CODE
	bne .2

	txa
	clc
	ror a
	clc
	adc #1
	sta <GAME_LEVEL

	ldy #6
	jmp mainMenuStart

.2
	inx
	inx
	dey
	bne .1


mainMenuCodeReturn
	jsr waitNMI50
	lda #16
	sta <MENU_CNT
	jsr mainMenuEnterCodeCur

	lda #$22
	sta PPU_ADDR
	lda #$97
	sta PPU_ADDR
	lda #$4c
	sta PPU_DATA
	lda #$4d
	sta PPU_DATA
	lda #$4e
	sta PPU_DATA
	lda #$4f
	sta PPU_DATA

	jsr resetPPUAdr
	sta <MENU_CNT	;A=0

	jsr FamiToneUpdate

	lda #SFX_ERROR
	jsr sfxPlay
	jmp mainMenuLoop


mainMenuSfxTest
	lda #0
	ldx <GAME_CODE+2
	beq .2
.1
	clc
	adc #10
	dex
	bne .1
.2
	clc
	adc <GAME_CODE+3
	cmp #SFX_COUNT
	bcs .3
	jsr sfxPlay
.3
	jmp mainMenuEnterCode

mainMenuEnterCodeCur
	lda #$22
	sta PPU_ADDR
	lda #$77
	sta PPU_ADDR
	ldx #0
	ldy #4
.1
	lda <MENU_CNT
	and #$10
	eor #$10
	beq .2
	lda #$00
	cpx <MENU_CODECUR
	bne .2
	lda #$61
.2
	sta PPU_DATA
	inx
	dey
	bne .1

	lda #$22
	sta PPU_ADDR
	lda #$b7
	sta PPU_ADDR
	ldx #0
	ldy #4
.3
	lda <MENU_CNT
	and #$10
	eor #$10
	beq .4
	lda #$00
	cpx <MENU_CODECUR
	bne .4
	lda #$62
.4
	sta PPU_DATA
	inx
	dey
	bne .3

	jmp resetPPUAdr


menuColors
	.db $33,$0f,$33,$ff,$33,$ff	;start
	.db $33,$ff,$33,$f0,$33,$ff	;code
	.db $33,$ff,$33,$0f,$33,$ff	;sfx
	.db $33,$ff,$33,$ff,$33,$f0	;bgm
	.db $33,$ff,$33,$ff,$33,$ff ;none
	.db $b0,$a0,$33,$ff,$33,$ff	;start bright
	.db $30,$f0,$3b,$fa,$33,$ff ;code bright


passwords
	.dw $0471	;2
	.dw $6725
	.dw $3241
	.dw $6758
	.dw $2981
	.dw $4597
	.dw $4783
	.dw $8913
	.dw $2056	;10
	.dw $2068
	.dw $5684
	.dw $8316
	.dw $9481
	.dw $8736
	.dw $1928
	.dw $8953
	.dw $6704
	.dw $5163
	.dw $9024	;20
	.dw $0641
	.dw $6281
	.dw $8306
	.dw $5210
	.dw $7490
	.dw $1538
	.dw $6102
	.dw $1604
	.dw $9341
	.dw $1738	;30
	.dw $4370
	.dw $1569
	.dw $1860
	.dw $7098
	.dw $4216
	.dw $6752
	.dw $3562
	.dw $6429
	.dw $6791
	.dw $2896	;40
	.dw $0521
	.dw $6753
	.dw $2856
	.dw $8163
	.dw $4126
	.dw $1926
	.dw $6305
	.dw $3695
	.dw $1857
	.dw $4752	;50
	.dw $6502