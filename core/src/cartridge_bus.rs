use crate::OpenBusReadResult;
use crate::cart_device::Cartridge as MapperCartridge;
use crate::interrupt::Interrupt;
use crate::mapper::Mapper;
use crate::ppu_memory_access::{PpuBusEvent, PpuReadAccess};

pub(crate) trait PpuCartridgeBus {
    fn read_ppu_pattern(
        &mut self,
        address: usize,
        access: PpuReadAccess,
        interrupt: &mut Interrupt,
    ) -> OpenBusReadResult;
    fn write_ppu_pattern(&mut self, address: usize, value: u8, interrupt: &mut Interrupt);
    fn read_ppu_nametable(
        &mut self,
        address: usize,
        access: PpuReadAccess,
        ciram: &mut [u8],
    ) -> OpenBusReadResult;
    fn write_ppu_nametable(
        &mut self,
        address: usize,
        value: u8,
        ciram: &mut [u8],
        interrupt: &mut Interrupt,
    );
    fn peek_ppu_nametable(&self, address: usize, ciram: &[u8]) -> Option<u8>;
    fn notify_ppu_status_read(&mut self, value: u8, interrupt: &mut Interrupt);
    fn notify_ppu_ctrl(&mut self, value: u8);
    fn notify_ppu_mask(&mut self, value: u8);
    fn notify_ppu_bus_event(&mut self, event: PpuBusEvent, interrupt: &mut Interrupt);
}

impl<T: MapperCartridge + ?Sized> PpuCartridgeBus for T {
    fn read_ppu_pattern(
        &mut self,
        address: usize,
        access: PpuReadAccess,
        interrupt: &mut Interrupt,
    ) -> OpenBusReadResult {
        MapperCartridge::read_ppu_pattern(self, address, access, interrupt)
    }

    fn write_ppu_pattern(&mut self, address: usize, value: u8, interrupt: &mut Interrupt) {
        MapperCartridge::write_ppu_pattern(self, address, value, interrupt);
    }

    fn read_ppu_nametable(
        &mut self,
        address: usize,
        access: PpuReadAccess,
        ciram: &mut [u8],
    ) -> OpenBusReadResult {
        MapperCartridge::read_ppu_nametable(self, address, access, ciram)
    }

    fn write_ppu_nametable(
        &mut self,
        address: usize,
        value: u8,
        ciram: &mut [u8],
        interrupt: &mut Interrupt,
    ) {
        MapperCartridge::write_ppu_nametable(self, address, value, ciram, interrupt);
    }

    fn peek_ppu_nametable(&self, address: usize, ciram: &[u8]) -> Option<u8> {
        MapperCartridge::peek_ppu_nametable(self, address, ciram)
    }

    fn notify_ppu_status_read(&mut self, value: u8, interrupt: &mut Interrupt) {
        MapperCartridge::notify_ppu_status_read(self, value, interrupt);
    }

    fn notify_ppu_ctrl(&mut self, value: u8) {
        MapperCartridge::notify_ppu_ctrl(self, value);
    }

    fn notify_ppu_mask(&mut self, value: u8) {
        MapperCartridge::notify_ppu_mask(self, value);
    }

    fn notify_ppu_bus_event(&mut self, event: PpuBusEvent, interrupt: &mut Interrupt) {
        Mapper::notify_ppu_bus_event(self, event, interrupt);
    }
}

pub(crate) trait CpuCartridgeBus: PpuCartridgeBus {
    fn read(&self, address: usize) -> OpenBusReadResult;
    fn write(&mut self, address: usize, value: u8, interrupt: &mut Interrupt);
    fn notify_cpu_read(&mut self, address: usize, value: u8, interrupt: &mut Interrupt);
    fn notify_oam_dma(&mut self, interrupt: &mut Interrupt);
}

impl<T: MapperCartridge + ?Sized> CpuCartridgeBus for T {
    fn read(&self, address: usize) -> OpenBusReadResult {
        MapperCartridge::read(self, address)
    }

    fn write(&mut self, address: usize, value: u8, interrupt: &mut Interrupt) {
        MapperCartridge::write(self, address, value, interrupt);
    }

    fn notify_cpu_read(&mut self, address: usize, value: u8, interrupt: &mut Interrupt) {
        MapperCartridge::notify_cpu_read(self, address, value, interrupt);
    }

    fn notify_oam_dma(&mut self, interrupt: &mut Interrupt) {
        MapperCartridge::notify_oam_dma(self, interrupt);
    }
}

pub(crate) struct MapperCartridgeBus<'a>(&'a mut dyn MapperCartridge);

impl PpuCartridgeBus for MapperCartridgeBus<'_> {
    fn read_ppu_pattern(
        &mut self,
        address: usize,
        access: PpuReadAccess,
        interrupt: &mut Interrupt,
    ) -> OpenBusReadResult {
        MapperCartridge::read_ppu_pattern(self.0, address, access, interrupt)
    }

    fn write_ppu_pattern(&mut self, address: usize, value: u8, interrupt: &mut Interrupt) {
        MapperCartridge::write_ppu_pattern(self.0, address, value, interrupt);
    }

    fn read_ppu_nametable(
        &mut self,
        address: usize,
        access: PpuReadAccess,
        ciram: &mut [u8],
    ) -> OpenBusReadResult {
        MapperCartridge::read_ppu_nametable(self.0, address, access, ciram)
    }

    fn write_ppu_nametable(
        &mut self,
        address: usize,
        value: u8,
        ciram: &mut [u8],
        interrupt: &mut Interrupt,
    ) {
        MapperCartridge::write_ppu_nametable(self.0, address, value, ciram, interrupt);
    }

    fn peek_ppu_nametable(&self, address: usize, ciram: &[u8]) -> Option<u8> {
        MapperCartridge::peek_ppu_nametable(self.0, address, ciram)
    }

    fn notify_ppu_status_read(&mut self, value: u8, interrupt: &mut Interrupt) {
        MapperCartridge::notify_ppu_status_read(self.0, value, interrupt);
    }

    fn notify_ppu_ctrl(&mut self, value: u8) {
        MapperCartridge::notify_ppu_ctrl(self.0, value);
    }

    fn notify_ppu_mask(&mut self, value: u8) {
        MapperCartridge::notify_ppu_mask(self.0, value);
    }

    fn notify_ppu_bus_event(&mut self, event: PpuBusEvent, interrupt: &mut Interrupt) {
        Mapper::notify_ppu_bus_event(self.0, event, interrupt);
    }
}

impl CpuCartridgeBus for MapperCartridgeBus<'_> {
    fn read(&self, address: usize) -> OpenBusReadResult {
        MapperCartridge::read(self.0, address)
    }

    fn write(&mut self, address: usize, value: u8, interrupt: &mut Interrupt) {
        MapperCartridge::write(self.0, address, value, interrupt);
    }

    fn notify_cpu_read(&mut self, address: usize, value: u8, interrupt: &mut Interrupt) {
        MapperCartridge::notify_cpu_read(self.0, address, value, interrupt);
    }

    fn notify_oam_dma(&mut self, interrupt: &mut Interrupt) {
        MapperCartridge::notify_oam_dma(self.0, interrupt);
    }
}

pub(crate) fn mapper_cartridge_bus(cartridge: &mut dyn MapperCartridge) -> MapperCartridgeBus<'_> {
    MapperCartridgeBus(cartridge)
}

pub(crate) struct CpuPpuCartridgeBus<'a>(&'a mut dyn CpuCartridgeBus);

impl PpuCartridgeBus for CpuPpuCartridgeBus<'_> {
    fn read_ppu_pattern(
        &mut self,
        address: usize,
        access: PpuReadAccess,
        interrupt: &mut Interrupt,
    ) -> OpenBusReadResult {
        PpuCartridgeBus::read_ppu_pattern(self.0, address, access, interrupt)
    }

    fn write_ppu_pattern(&mut self, address: usize, value: u8, interrupt: &mut Interrupt) {
        PpuCartridgeBus::write_ppu_pattern(self.0, address, value, interrupt);
    }

    fn read_ppu_nametable(
        &mut self,
        address: usize,
        access: PpuReadAccess,
        ciram: &mut [u8],
    ) -> OpenBusReadResult {
        PpuCartridgeBus::read_ppu_nametable(self.0, address, access, ciram)
    }

    fn write_ppu_nametable(
        &mut self,
        address: usize,
        value: u8,
        ciram: &mut [u8],
        interrupt: &mut Interrupt,
    ) {
        PpuCartridgeBus::write_ppu_nametable(self.0, address, value, ciram, interrupt);
    }

    fn peek_ppu_nametable(&self, address: usize, ciram: &[u8]) -> Option<u8> {
        PpuCartridgeBus::peek_ppu_nametable(self.0, address, ciram)
    }

    fn notify_ppu_status_read(&mut self, value: u8, interrupt: &mut Interrupt) {
        PpuCartridgeBus::notify_ppu_status_read(self.0, value, interrupt);
    }

    fn notify_ppu_ctrl(&mut self, value: u8) {
        PpuCartridgeBus::notify_ppu_ctrl(self.0, value);
    }

    fn notify_ppu_mask(&mut self, value: u8) {
        PpuCartridgeBus::notify_ppu_mask(self.0, value);
    }

    fn notify_ppu_bus_event(&mut self, event: PpuBusEvent, interrupt: &mut Interrupt) {
        PpuCartridgeBus::notify_ppu_bus_event(self.0, event, interrupt);
    }
}

pub(crate) fn cpu_ppu_cartridge_bus(cartridge: &mut dyn CpuCartridgeBus) -> CpuPpuCartridgeBus<'_> {
    CpuPpuCartridgeBus(cartridge)
}
