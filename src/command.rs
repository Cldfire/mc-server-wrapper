/// Commands that can be sent over channels to be performed by the MC server.
#[derive(Debug)]
pub enum ServerCommand {
    /// Send a chat message to all players
    SendChatMsg(String)
}
