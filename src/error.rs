/// General application errors
#[derive(Debug)]
pub enum Error {
    DiscordErr(twilight::http::Error),
}

impl From<twilight::http::Error> for Error {
    fn from(err: twilight::http::Error) -> Self {
        Self::DiscordErr(err)
    }
}

/// Errors originating from a Minecraft server process.
#[derive(Debug)]
pub enum ServerError {
    /// The server failed to start because the EULA has not been accepted
    EulaNotAccepted,
    /// Something was received on stderr
    ///
    /// It is unlikely that it will be possible to restart the server after this
    StdErrMsg(Vec<String>)
}
