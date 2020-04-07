/// Commands that can be sent over channels to be performed by the MC server.
#[derive(Debug)]
pub enum ServerCommand {
    /// Send a message from a Discord user to all players on the server
    SendDiscordMsg {
        /// The name of the Discord user that sent this message
        username: String,
        /// The message the user sent
        msg: String
    },

    /// Signals that the server has been shut down and we should stop listening
    /// for messages
    ServerClosed
}
