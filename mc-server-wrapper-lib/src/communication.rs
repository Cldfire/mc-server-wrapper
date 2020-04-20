use crate::parse::*;

use std::io;
use std::process::ExitStatus;

/// Events from a Minecraft server.
// TODO: derive serialize, deserialize
// TODO: move to different file
// TODO: restructure so there are two main variants: stuff you get directly
// from the server, and stuff more related to management
#[derive(Debug)]
pub enum ServerEvent {
    /// An event parsed from the server's console output (stderr or stdout)
    ///
    /// You are given a `ConsoleMsg` representing a generic form of the console
    /// output. This can be directly printed to your program's stdout in order
    /// to replicate (with slightly nicer formatting) the Minecraft server's
    /// output.
    ///
    /// You are also given an `Option<ConsoleMsgSpecific>`. Some `ConsoleMsg`s
    /// can be parsed into more specific representations, and in that case you
    /// will be given one. These are not for printing; they are useful for
    /// triggering actions based on events coming from the server.
    ConsoleEvent(ConsoleMsg, Option<ConsoleMsgSpecific>),
    /// An unknown line received from the server's stdout
    StdoutLine(String),
    /// An unknown line received from the server's stderr
    StderrLine(String),

    /// The Minecraft server process exited with the given exit status and, if
    /// known, a reason for exiting
    ServerStopped(ExitStatus, Option<ShutdownReason>),

    /// Response to `AgreeToEula`
    AgreeToEulaResult(io::Result<()>),
}

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

    /// Agree to the EULA (required to run the server)
    AgreeToEula,
    /// Start the Minecraft server (if it is stopped)
    StartServer,
    /// Stop the Minecraft server (if it is running)
    ///
    /// Setting `forever` to true will cause the `McServer` instance to stop
    /// listening for commands and gracefully shutdown everything related to
    /// it.
    StopServer { forever: bool },
}

/// Reasons that a Minecraft server stopped running
// TODO: add variant indicating user requested server be stopped
#[derive(Debug)]
pub enum ShutdownReason {
    /// The server stopped because the EULA has not been accepted
    EulaNotAccepted,
}
