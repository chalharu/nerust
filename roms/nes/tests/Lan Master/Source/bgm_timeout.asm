bgm_timeout_module
	.dw .chn0,.chn1,.chn2,.chn3,.chn4,.ins
	.db $06
.env_default
	.db $c0,$7f,$00
.env_vol0
	.db $cf,$04,$c0,$7f,$02
.env_vol1
	.db $c1,$c0,$c1,$03,$c0,$c1,$c0,$c1,$c0,$c1,$03,$c0,$c1,$04,$c0,$c1
	.db $04,$c0,$c1,$08,$c0,$c2,$c1,$c2,$03,$c1,$c2,$03,$c1,$c2,$c2,$c1
	.db $c2,$06,$c1,$c2,$06,$c3,$04,$c2,$c3,$05,$c2,$c3,$07,$c2,$c3,$0a
	.db $c2,$c3,$09,$c2,$05,$c1,$c2,$09,$c1,$c2,$06,$c1,$c2,$c2,$c1,$13
	.db $c0,$c1,$05,$c0,$c1,$c1,$c0,$c1,$c1,$c0,$c1,$c1,$c0,$c1,$c0,$7f
	.db $4e
.env_vol2
	.db $c3,$03,$c0,$c1,$c0,$03,$c2,$03,$c0,$c2,$c0,$03,$c2,$c2,$c0,$c0
	.db $c2,$c0,$03,$c2,$c2,$c0,$c0,$c2,$c0,$7f,$18
.env_vol3
	.db $c1,$c2,$c3,$c2,$c1,$7f,$04
.env_vol4
	.db $c2,$03,$c0,$7f,$02
.env_vol5
	.db $cf,$03,$c0,$7f,$02
.env_vol6
	.db $c3,$c4,$c3,$c2,$7f,$03
.env_vol7
	.db $c2,$04,$c1,$0b,$c0,$7f,$04
.env_vol8
	.db $c2,$c2,$c3,$03,$c4,$25,$c3,$c4,$c3,$c4,$c3,$c4,$c3,$c4,$c3,$c4
	.db $c3,$c4,$c3,$c4,$c3,$18,$c2,$c3,$c2,$c3,$c2,$c3,$c2,$c3,$c2,$c3
	.db $c2,$c3,$c2,$c3,$c2,$c3,$c2,$c3,$c2,$17,$c1,$c2,$c1,$c2,$c1,$c2
	.db $c1,$c2,$c1,$c2,$c1,$c2,$c1,$c2,$c1,$c2,$c1,$c2,$c1,$0b,$c0,$c1
	.db $07,$c0,$c1,$c1,$c0,$c1,$c1,$c0,$c1,$c0,$c1,$c0,$c1,$c0,$c1,$c0
	.db $03,$c1,$c0,$03,$c1,$c0,$04,$c1,$c0,$7f,$58
.env_vol9
	.db $c4,$c2,$c1,$05,$c0,$7f,$04
.env_vol10
	.db $cf,$06,$c0,$7f,$02
.env_vol11
	.db $cf,$7f,$00
.env_vol12
	.db $c2,$c3,$c2,$c2,$7f,$03
.env_vol13
	.db $cf,$cf,$c0,$7f,$02
.env_vol14
	.db $c3,$c4,$c5,$c5,$c4,$c3,$c1,$0e,$c0,$7f,$08
.env_vol15
	.db $c1,$c2,$c2,$c1,$04,$c0,$7f,$05
.env_vol16
	.db $c2,$30,$c1,$4f,$c0,$7f,$04
.env_vol17
	.db $c2,$7f,$00
.env_vol18
	.db $c3,$7f,$00
.env_vol19
	.db $c1,$03,$c0,$7f,$02
.env_arp0
	.db $c0,$bd,$ba,$7f,$02
.env_arp1
	.db $bd,$be,$bf,$c0,$7f,$03
.env_arp2
	.db $bc,$bc,$bd,$bd,$be,$be,$bf,$bf,$c0,$7f,$08
.env_arp3
	.db $cc,$c0,$7f,$01
.env_pitch0
	.db $ca,$7f,$00
.env_pitch1
	.db $c1,$c1,$c2,$c2,$c3,$c3,$c4,$c4,$c5,$c5,$c6,$c6,$c5,$c5,$c4,$c4
	.db $c3,$c3,$c2,$c2,$c1,$c1,$c0,$c0,$7f,$00
.env_pitch2
	.db $c2,$7f,$00
.env_pitch3
	.db $c0,$12,$c1,$04,$c2,$04,$c1,$04,$c0,$04,$7f,$02
.env_pitch4
	.db $c1,$c2,$c3,$c2,$c1,$c0,$7f,$00
.env_pitch5
	.db $ac,$a2,$9d,$9d,$7f,$03
.ins
	.dw .env_default,.env_default,.env_default
	.db $30,$00
	.dw .env_vol0,.env_default,.env_default
	.db $30,$00
	.dw .env_vol1,.env_default,.env_pitch1
	.db $30,$00
	.dw .env_vol1,.env_default,.env_pitch0
	.db $30,$00
	.dw .env_vol2,.env_default,.env_default
	.db $70,$00
	.dw .env_vol3,.env_default,.env_default
	.db $70,$00
	.dw .env_vol4,.env_default,.env_default
	.db $70,$00
	.dw .env_vol6,.env_default,.env_default
	.db $70,$00
	.dw .env_vol7,.env_default,.env_default
	.db $30,$00
	.dw .env_vol12,.env_default,.env_pitch2
	.db $70,$00
	.dw .env_vol7,.env_default,.env_pitch2
	.db $30,$00
	.dw .env_vol8,.env_default,.env_pitch3
	.db $70,$00
	.dw .env_vol9,.env_default,.env_default
	.db $30,$00
	.dw .env_vol10,.env_default,.env_pitch4
	.db $30,$00
	.dw .env_vol11,.env_default,.env_default
	.db $30,$00
	.dw .env_vol13,.env_default,.env_default
	.db $30,$00
	.dw .env_vol11,.env_arp1,.env_default
	.db $30,$00
	.dw .env_vol8,.env_arp2,.env_pitch3
	.db $70,$00
	.dw .env_vol16,.env_default,.env_pitch3
	.db $70,$00
	.dw .env_vol17,.env_default,.env_pitch4
	.db $70,$00
	.dw .env_vol18,.env_default,.env_pitch4
	.db $70,$00
	.dw .env_vol19,.env_default,.env_default
	.db $70,$00

.chn0
.chn0_0
	.db $47,$1c,$48,$1c,$47,$15,$48,$1c,$47,$18,$48,$15,$47,$15,$48,$18
	.db $47,$1a,$48,$15,$47,$13,$48,$1a,$47,$17,$48,$13,$47,$13,$48,$17
	.db $47,$18,$48,$13,$47,$11,$48,$18,$47,$15,$48,$11,$47,$11,$48,$15
	.db $4b,$0e,$a6
.chn0_loop
.chn0_1
	.db $bf
	.db $fe
	.dw .chn0_loop

.chn1
.chn1_0
	.db $49,$1c,$4a,$1c,$49,$15,$4a,$1c,$49,$18,$4a,$15,$49,$15,$4a,$18
	.db $49,$1a,$4a,$15,$49,$13,$4a,$1a,$49,$17,$4a,$13,$49,$13,$4a,$17
	.db $49,$18,$4a,$13,$49,$11,$4a,$18,$49,$15,$4a,$11,$49,$11,$4a,$15
	.db $42,$0e,$a6
.chn1_loop
.chn1_1
	.db $bf
	.db $fe
	.dw .chn1_loop

.chn2
.chn2_0
	.db $41,$15,$81,$15,$81,$15,$80,$13,$81,$13,$81,$13,$80,$11,$81,$11
	.db $81,$11,$80,$4e,$02,$8a,$3f,$9a
.chn2_loop
.chn2_1
	.db $bf
	.db $fe
	.dw .chn2_loop

.chn3
.chn3_0
	.db $46,$0d,$82,$0b,$82,$0d,$82,$0b,$82,$0d,$82,$0b,$82,$44,$0b,$82
	.db $0d,$a2
.chn3_loop
.chn3_1
	.db $bf
	.db $fe
	.dw .chn3_loop

.chn4
.chn4_0
	.db $00,$81,$00,$04,$80,$00,$80,$00,$81,$00,$04,$80,$00,$80,$00,$81
	.db $00,$04,$80,$00,$80,$00,$a6
.chn4_loop
.chn4_1
	.db $bf
	.db $fe
	.dw .chn4_loop
