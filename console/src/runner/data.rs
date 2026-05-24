use crate::console::{AuxiliaryInput, ConsoleRequestResult, ControllerInputs, ControllerPort};
use nerust_contract::CoreOptions;
use nerust_core::cartridge_data::CartridgeData;
use std::sync::mpsc::Sender;

pub(crate) enum ConsoleData {
    Load {
        cartridge_data: CartridgeData,
        options: CoreOptions,
        reply: Sender<ConsoleRequestResult>,
    },
    Resume,
    Pause,
    Reset(Sender<ConsoleRequestResult>),
    PortInputs {
        port: ControllerPort,
        inputs: ControllerInputs,
    },
    AuxiliaryInput {
        input: AuxiliaryInput,
        active: bool,
    },
    Unload(Sender<ConsoleRequestResult>),
    ExportMapperSave(Sender<ConsoleRequestResult>),
    ImportMapperSave {
        bytes: Vec<u8>,
        reply: Sender<ConsoleRequestResult>,
    },
    PersistenceTarget(Sender<ConsoleRequestResult>),
    ExportState(Sender<ConsoleRequestResult>),
    ImportState {
        bytes: Vec<u8>,
        reply: Sender<ConsoleRequestResult>,
    },
}
