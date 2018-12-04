#[derive(Debug)]
pub enum ServerError {
    /// The server failed to start because the EULA has not been accepted
    EulaNotAccepted
}
