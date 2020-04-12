use tokio::prelude::*;
use tokio::io::BufReader;
use tokio::sync::mpsc;
use tokio::stream::StreamExt;
use tokio::process;
use tokio::sync::Mutex;
use tokio::fs::File;

use std::process::{Stdio, ExitStatus};
use std::sync::Arc;
use std::path::PathBuf;
use std::io;

use crate::parse::{ConsoleMsg, ConsoleMsgSpecific};

#[cfg(test)]
mod test;
pub mod parse;

/// Configuration provided to setup an `McServer` instance.
#[derive(Debug)]
pub struct McServerConfig {
    /// The path to the server jarfile
    pub server_path: PathBuf,
    /// The amount of memory in megabytes to allocate for the server
    pub memory: u16
}

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
    AgreeToEulaResult(io::Result<()>)
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
    StopServer {
        forever: bool
    }
}

/// Reasons that a Minecraft server stopped running
// TODO: add variant indicating user requested server be stopped
#[derive(Debug)]
pub enum ShutdownReason {
    /// The server stopped because the EULA has not been accepted
    EulaNotAccepted
}

/// Represents a single wrapped Minecraft server that may be running or stopped
#[derive(Debug)]
pub struct McServer {
    /// Channel through which commands can be sent to the server
    pub cmd_sender: mpsc::Sender<ServerCommand>,
    /// Channel through which events are received from the server
    pub event_receiver: mpsc::Receiver<ServerEvent>
}

impl McServer {
    /// Create a new `McServer` with the given `McServerConfig`.
    ///
    /// Note that the Minecraft server is not launched until you send a command
    /// to cause that to happen.
    // TODO: should this be called `manage`?
    pub fn new(config: McServerConfig) -> Self {
        let (cmd_sender, mut cmd_receiver) = mpsc::channel::<ServerCommand>(64);
        let (event_sender, event_receiver) = mpsc::channel::<ServerEvent>(64);
        let mc_server_internal = Arc::new(McServerInternal {
            config,
            event_sender,
            mc_stdin: Arc::new(Mutex::new(None))
        });

        let mc_server_internal_clone = mc_server_internal.clone();
        // Start a task to receive server commands and handle them appropriately
        // TODO: move this out of this function
        tokio::spawn(async move {
            while let Some(cmd) = cmd_receiver.next().await {
                use ServerCommand::*;
                use ServerEvent::*;
                let mc_server_internal = mc_server_internal_clone.clone();

                match cmd {
                    TellRaw(json) => {
                        let mut mc_stdin = mc_server_internal.mc_stdin.lock().await;
                        if let Some(mc_stdin) = &mut *mc_stdin {
                            // TODO: handle error?
                            let _ = mc_stdin.write_all(
                                ("tellraw @a ".to_string() + &json + "\n")
                                .as_bytes()
                            ).await;
                        }
                    },
                    WriteCommandToStdin(text) => {
                        let mut mc_stdin = mc_server_internal.mc_stdin.lock().await;
                        if let Some(mc_stdin) = &mut *mc_stdin {
                            // TODO: handle error?
                            let _ = mc_stdin.write_all((text + "\n").as_bytes()).await;
                        }
                    },
                    WriteToStdin(text) => {
                        let mut mc_stdin = mc_server_internal.mc_stdin.lock().await;
                        if let Some(mc_stdin) = &mut *mc_stdin {
                            // TODO: handle error?
                            let _ = mc_stdin.write_all(text.as_bytes()).await;
                        }
                    },
                    
                    AgreeToEula => {
                        tokio::spawn(async move {
                            mc_server_internal.event_sender.clone().send(
                                AgreeToEulaResult(mc_server_internal.agree_to_eula().await)
                            ).await.unwrap();
                        });
                    },   
                    StartServer => {
                        // Make sure the server is not already running
                        {
                            let mc_stdin = mc_server_internal.mc_stdin.lock().await;
                            if mc_stdin.is_some() { continue }
                        }

                        // Spawn a task to drive the server process to completion
                        // and send an event when it exits
                        tokio::spawn(async move {
                            let ret = mc_server_internal.run_server().await;
                            mc_server_internal.event_sender.clone().send(ServerStopped(ret.0, ret.1)).await.unwrap();
                        });
                    },
                    StopServer { forever } => {
                        let mut mc_stdin = mc_server_internal.mc_stdin.lock().await;
                        if let Some(mc_stdin) = &mut *mc_stdin {
                            // TODO: handle error?
                            let _ = mc_stdin.write_all(("stop".to_string() + "\n").as_bytes()).await;
                        }

                        if forever {
                            break;
                        }
                    }
                }
            }
        });

        McServer {
            cmd_sender,
            event_receiver
        }
    }
}

// Groups together stuff needed internally by the library
#[derive(Debug)]
struct McServerInternal {
    /// Configuration for this server instance
    // TODO: support editing this config while server is running
    config: McServerConfig,
    /// Channel through which we send events
    event_sender: mpsc::Sender<ServerEvent>,
    /// Handle to the server's stdin if it's running
    mc_stdin: Arc<Mutex<Option<process::ChildStdin>>>
}

impl McServerInternal {
    /// Run a minecraft server.
    // TODO: write better docs
    // TODO: maybe split into functions that start the server and interface
    // with it
    async fn run_server(&self) -> (ExitStatus, Option<ShutdownReason>) {
        // TODO: don't unwrap / expect, all over this function
        let folder = self.config.server_path.as_path().parent().unwrap();
        let file = self.config.server_path.file_name().unwrap();

        // TODO: support running from inside folder containing server jar
        // (don't run cd)
        let mut process = process::Command::new("sh")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .args(&[
                "-c",
                &format!(
                    "cd {} && exec java -Xms{}M -Xmx{}M -jar {} nogui",
                    folder.to_str().unwrap(),
                    self.config.memory,
                    self.config.memory,
                    file.to_str().unwrap()
                )
            ]).spawn().unwrap();

        // Update the stored handle to the server's stdin
        {
            let mut mc_stdin = self.mc_stdin.lock().await;
            if mc_stdin.is_some() { unreachable!() };
            *mc_stdin = Some(process.stdin.take().unwrap());
        }

        let mut stdout = BufReader::new(process.stdout.take().unwrap()).lines();
        let mut stderr = BufReader::new(process.stderr.take().unwrap()).lines();

        let status_handle = tokio::spawn(async {
            process.await.expect("child process encountered an error")
        });

        let event_sender_clone = self.event_sender.clone();
        let stderr_handle = tokio::spawn(async move {
            use ServerEvent::*;
            let mut event_sender = event_sender_clone;

            while let Some(line) = stderr.next_line().await.unwrap() {
                event_sender.send(StderrLine(line)).await.unwrap();
            }
        });

        let event_sender_clone = self.event_sender.clone();
        let stdout_handle = tokio::spawn(async move {
            use ServerEvent::*;
            let mut event_sender = event_sender_clone;
            // We have this return value so we can keep track of things (such
            // as a EULA that needs agreed to) and send that along with the
            // server shutdown event
            //
            // This makes things much easier on the library user as they don't
            // need to come up with a separate mechanism for doing that
            let mut shutdown_reason = None;

            while let Some(line) = stdout.next_line().await.unwrap() {
                if let Some(console_msg) = ConsoleMsg::try_parse_from(&line) {
                    let specific_msg = ConsoleMsgSpecific::try_parse_from(&console_msg);

                    if specific_msg == Some(ConsoleMsgSpecific::MustAcceptEula) {
                        shutdown_reason = Some(ShutdownReason::EulaNotAccepted);
                    }

                    event_sender.send(ConsoleEvent(console_msg, specific_msg)).await.unwrap();
                } else {
                    // spigot servers print lines that reach this branch ("\n",
                    // "Loading libraries, please wait...")
                    event_sender.send(StdoutLine(line)).await.unwrap();
                }
            }

            shutdown_reason
        });

        let (status, stdout_val, _) = tokio::join!(
            status_handle,
            stdout_handle,
            stderr_handle,
        );

        // Update the stored handle to the server's stdin
        {
            let mut mc_stdin = self.mc_stdin.lock().await;
            if mc_stdin.is_none() { unreachable!() };
            *mc_stdin = None;
        }

        (status.unwrap(), stdout_val.unwrap())
    }

    /// Overwrites the `eula.txt` file with the contents `eula=true`.
    async fn agree_to_eula(&self) -> io::Result<()> {
        let mut file = File::create(
            self.config.server_path.parent().unwrap().join("eula.txt")
        ).await?;

        file.write_all(b"eula=true").await
    }
}
