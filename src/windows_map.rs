//! Tracks which tiled windows live on which workspace.

use niri_ipc::{Request, Response};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::error::Result;
use crate::ipc::{self, ActionSocket};

#[derive(Default)]
pub struct WindowMap(HashMap<u64, Vec<u64>>);

impl WindowMap {
    /// Build the map from the windows niri currently reports.
    pub fn init(socket: Arc<Mutex<ActionSocket>>) -> Result<Self> {
        let response = socket.lock().unwrap().request(Request::Windows)?;
        let windows = match response {
            Response::Windows(windows) => windows,
            other => return Err(ipc::unexpected(other)),
        };

        let mut map = Self::default();
        // Floating windows cannot/should not be width-managed.
        for window in windows.iter().filter(|w| !w.is_floating) {
            if let Some(ws) = window.workspace_id {
                map.0.entry(ws).or_default().push(window.id);
            }
        }
        Ok(map)
    }

    pub fn workspace_of(&self, id: u64) -> Option<u64> {
        self.0
            .iter()
            .find_map(|(&ws, ids)| ids.contains(&id).then_some(ws))
    }

    /// Track `id` on `ws`; returns the workspace it was on before, if any.
    pub fn place(&mut self, id: u64, ws: u64) -> Option<u64> {
        let previous = self.remove(id);
        self.0.entry(ws).or_default().push(id);
        previous
    }

    /// Stop tracking `id`; returns the workspace it was on, if any.
    pub fn remove(&mut self, id: u64) -> Option<u64> {
        let ws = self.workspace_of(id)?;
        if let Some(ids) = self.0.get_mut(&ws) {
            ids.retain(|&other| other != id);
        }
        Some(ws)
    }

    pub fn windows(&self, ws: u64) -> &[u64] {
        self.0.get(&ws).map(Vec::as_slice).unwrap_or_default()
    }
}
