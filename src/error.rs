#[derive(Debug)]
pub enum ServerError {
    /// The server failed to start because the EULA has not been accepted
    EulaNotAccepted,
    /// Something was received on stderr
    ///
    /// It is unlikely that it will be possible to restart the server after this
    StdErr(Vec<String>)
}
