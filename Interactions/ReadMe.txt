Tests various edge case interactions in the GBA.

When a DMA occurs, the IRQ pipeline is stalled whenever the cpu is stalled. So, the IRQ pipeline will be stalled if the cpu is waiting to do a read or write, but not if it is doing internal cycles such as multiply. Halt counts as an internal cycle for this situation.
