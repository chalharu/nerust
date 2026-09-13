Tests prefetcher behaviour when branching to nearby addresses and what happens when the prefetcher is full.
Interesting behaviour is noted when prefetcher is full. It appears that the prefetch unit pauses until the buffer is empty and then starts up again with non-sequential timing.

When the prefetch buffer is empty, and a brnach is done to the next instruction that would be read by the prefetcher, the instruction fetch is done with non-sequential timing (even though the prefetcher starts at the same time with sequential timing.)

When the prefetcher encounters a 0x20000 boundary, it stops and behaves as though it is full.
