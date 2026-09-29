Tests various edge cases involving cpsrs / spsr including:

- upper bit of mode value is always set
- reading / writing spsr in modes without an spsr
- timing of disabling interrupts when diasbling via S bit set
