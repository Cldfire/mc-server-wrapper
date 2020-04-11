/// Commands that can be sent over channels to be performed by the MC server.
///
/// Note that all commands will be ignored if they cannot be performed (i.e.,
/// telling the server to send a message )
#[derive(Debug)]
pub enum ServerCommand {
    /// Send a message to all players on the server
    ///
    /// Message should be JSON of the following format:
    /// https://minecraft.gamepedia.com/Raw_JSON_text_format
    TellRaw(String),
    /// Write the given string to the server's stdin as a command
    ///
    /// This means that the given string will have "\n" appended to it
    WriteCommandToStdin(String),
    /// Write the given string verbatim to stdin
    WriteToStdin(String),

    /// Start the Minecraft server
    StartServer,
    /// Stop the Minecraft server
    StopServer,

    /// Stop listening for commands and gracefully shut down everything related
    /// to a `McServer` instance.
    ///
    /// This will cause a `StopServer` command to be sent as well. Send this
    /// command when you have no further intentions of starting the Minecraft
    /// server back up.
    // TODO: is this a good name?
    EndInstance
}
