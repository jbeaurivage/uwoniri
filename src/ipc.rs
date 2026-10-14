use niri_ipc::{Action, Request, Response, socket::Socket};

use crate::error::{Error, Result};

/// Send a request and flatten the two failure layers (transport error and
/// niri-reported error) into [`Error`]. `what` names the request in messages.
pub fn send(socket: &mut Socket, what: &'static str, request: Request) -> Result<Response> {
    socket
        .send(request)
        .map_err(|source| Error::Send { what, source })?
        .map_err(|message| Error::Niri { what, message })
}

/// Send an action and check that niri acknowledged it.
pub fn action(socket: &mut Socket, what: &'static str, action: Action) -> Result<()> {
    match send(socket, what, Request::Action(action))? {
        Response::Handled => Ok(()),
        other => Err(unexpected(what, other)),
    }
}

pub fn unexpected(what: &'static str, response: Response) -> Error {
    Error::UnexpectedResponse {
        what,
        response: format!("{response:?}"),
    }
}
