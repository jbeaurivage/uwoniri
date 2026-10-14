//! Initialize niri IPC socket connections and event stream

use niri_ipc::Event;
use niri_ipc::{Action, Request, Response, socket::Socket};

use crate::error::{Error, Result};
use crate::ipc;

pub struct ActionSocket(Socket);

impl ActionSocket {
    /// Send an action and check that niri acknowledged it.
    pub fn send(&mut self, action: Action) -> Result<()> {
        match send(&mut self.0, Request::Action(action))? {
            Response::Handled => Ok(()),
            other => Err(unexpected(other)),
        }
    }

    pub fn request(&mut self, request: Request) -> Result<Response> {
        send(&mut self.0, request)
    }
}

pub struct EventSocket(Socket);

impl EventSocket {
    pub fn read_events(self) -> impl FnMut() -> std::io::Result<Event> {
        self.0.read_events()
    }
}

pub fn init_socket_connections() -> Result<(EventSocket, ActionSocket)> {
    // Connect to niri IPC socket
    let mut event_socket = EventSocket(Socket::connect().map_err(Error::Connect)?);

    // Start the event stream
    match send(&mut event_socket.0, Request::EventStream)? {
        Response::Handled => {}
        other => return Err(ipc::unexpected(other)),
    }

    // Create a separate socket connection to send actions
    let action_socket = ActionSocket(Socket::connect().map_err(Error::Connect)?);

    // Return both sockets connections so they can be called elsewhere
    Ok((event_socket, action_socket))
}

pub fn unexpected(response: Response) -> Error {
    Error::UnexpectedResponse(format!("{response:?}"))
}

/// Send a request and flatten the two failure layers (transport error and
/// niri-reported error) into [`Error`]. `what` names the request in
/// messages.
fn send(socket: &mut Socket, request: Request) -> Result<Response> {
    socket
        .send(request)
        .map_err(Error::Send)?
        .map_err(Error::Niri)
}
