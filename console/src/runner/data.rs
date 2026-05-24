use crate::ConsoleRequestResult;
use nerust_contract_options::CoreOptions;
use nerust_core::cartridge_rom::CartridgeData;
use std::sync::mpsc::Sender;

pub(crate) enum ConsoleData {
    Load {
        cartridge_data: CartridgeData,
        options: CoreOptions,
        reply: Sender<ConsoleRequestResult>,
    },
    Resume,
    Pause,
    ApplyInputState {
        bytes: Vec<u8>,
    },
    Reset(Sender<ConsoleRequestResult>),
    ApplyControllerState {
        bytes: Vec<u8>,
        reply: Sender<ConsoleRequestResult>,
    },
    Unload(Sender<ConsoleRequestResult>),
    ExportMapperSave(Sender<ConsoleRequestResult>),
    ImportMapperSave {
        bytes: Vec<u8>,
        reply: Sender<ConsoleRequestResult>,
    },
    PersistenceTarget(Sender<ConsoleRequestResult>),
    ExportState(Sender<ConsoleRequestResult>),
    CurrentControllerState(Sender<ConsoleRequestResult>),
    CurrentInputState(Sender<ConsoleRequestResult>),
    ImportState {
        bytes: Vec<u8>,
        reply: Sender<ConsoleRequestResult>,
    },
}
