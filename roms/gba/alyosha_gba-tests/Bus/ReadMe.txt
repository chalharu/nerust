Various test for open bus emulation. Note that GBA Tek gives an essentially complete description of bus behavior, see there for details.

Sme minor detials that are tested here:

DMA updates the cpu bus. In particular, DMA (16 bit) from 0x04010000 updates cpu open bus (both 16 bit halves.) 

32 bit DMA from addresses it cannot access do not update the DMA bus, but do copy it to the cpu bus. 16 bit DMA from these addresses copy the read half of the DMA bus to both halves of the cpu bus.

However 16 bit cpu reads from unused memory (anything that normally returns cpu open bus) do not update the cpu bus.


