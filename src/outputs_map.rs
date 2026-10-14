//! Create an outputs map, used later for window/output size comparison,
//! used as a workaround for some limitations of the niri IPC
//! This can be dropped once https://github.com/Antiz96/oniri/issues/3 is solved

use log::info;
use niri_ipc::{Output, Request, Response};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::error::Result;
use crate::ipc::{self, ActionSocket};

pub fn init_outputs_map(
    action_socket: Arc<Mutex<ActionSocket>>,
    config: &crate::Config,
) -> Result<HashMap<String, Output>> {
    let mut action_socket = action_socket.lock().unwrap();
    let outputs = match action_socket.request(Request::Outputs)? {
        Response::Outputs(outputs) => outputs,
        other => return Err(ipc::unexpected(other)),
    };

    for name in outputs.keys() {
        let uw_applies = config.applies_to(Some(name), &outputs);
        if uw_applies {
            info!("Registered output: {name}");
        } else {
            info!("Ignored output: {name}");
        }
    }

    Ok(outputs)
}
