Notes: In ROM region, non-sequential accesses are decided based on if the current internal addresses is adjacent to a 0x20000 boundary.
In this case, the NEXT access will use non-sequential timing, even if in decrement or fixed mode.
