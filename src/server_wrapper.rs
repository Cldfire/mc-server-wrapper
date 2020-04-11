use tokio::prelude::*;
use tokio::io::BufReader;
use tokio::sync::mpsc;
use tokio::stream::StreamExt;
use tokio::process;
use tokio::sync::Mutex;

use std::process::{Stdio, ExitStatus};
use std::sync::Arc;
use std::path::PathBuf;

use crate::parse::ConsoleMsgSpecific;
use crate::command::ServerCommand;

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
// TODO: should we embed `ConsoleMsgSpecific` or hide that?
// TODO: move to different file
#[derive(Debug)]
pub enum ServerEvent {
    /// An event parsed from the server's console output (stderr or stdout)
    ConsoleEvent(ConsoleMsgSpecific),
    /// An unknown line received from the server's stdout
    StdoutLine(String),
    /// An unknown line received from the server's stderr
    StderrLine(String),

    /// The Minecraft server process exited with the given exit status and, if
    /// known, a reason for exiting
    ServerStopped(ExitStatus, Option<ShutdownReason>)
}

/// Reasons that a Minecraft server stopped running
// TODO: add variant indicating user requested server be stopped
#[derive(Debug)]
pub enum ShutdownReason {
    /// The server stopped because the EULA has not been accepted
    EulaNotAccepted
}

/// Represents a single wrapped Minecraft server that may be running or stopped.
pub struct McServer {
    /// Configuration for this server instance
    // TODO: support editing this config while server is running
    config: McServerConfig,
    /// So we can send ourself commands if need be
    cmd_sender: mpsc::Sender<ServerCommand>,
    /// Channel via which we send events
    event_sender: mpsc::Sender<ServerEvent>,
    /// Handle to the server's stdin if it's running
    mc_stdin: Arc<Mutex<Option<process::ChildStdin>>>
}

impl McServer {
    /// Create a new `McServer` with the given `McServerConfig`.
    ///
    /// Returns a tuple containing the `McServer` instance, a `Sender` with
    /// which to send the server commands, and a `Receiver` with which to
    /// receive events from the server.
    ///
    /// Note that the Minecraft server is not launched until you send a command
    /// to cause that to happen.
    pub async fn new(
        config: McServerConfig
    ) -> (Arc<Self>, mpsc::Sender<ServerCommand>, mpsc::Receiver<ServerEvent>) {
        let (cmd_sender, mut cmd_receiver) = mpsc::channel::<ServerCommand>(64);
        let (event_sender, event_receiver) = mpsc::channel::<ServerEvent>(64);
        let mc_server = Arc::new(McServer {
            config,
            cmd_sender: cmd_sender.clone(),
            event_sender: event_sender.clone(),
            mc_stdin: Arc::new(Mutex::new(None))
        });

        let event_sender_clone = event_sender.clone();
        let mc_server_clone = mc_server.clone();
        // Start a task to receive server commands and handle them appropriately
        tokio::spawn(async move {
            let event_sender = event_sender_clone;
            let mc_server = mc_server_clone;
            let cmd_sender = mc_server.cmd_sender.clone();

            while let Some(cmd) = cmd_receiver.next().await {
                use ServerCommand::*;
                let mc_server = mc_server.clone();
                let mut cmd_sender = cmd_sender.clone();

                match cmd {
                    TellRaw(json) => {
                        let mut mc_stdin = mc_server.mc_stdin.lock().await;
                        if let Some(mc_stdin) = &mut *mc_stdin {
                            // TODO: handle error?
                            let _ = mc_stdin.write_all(
                                ("tellraw @a ".to_string() + &json + "\n")
                                .as_bytes()
                            ).await;
                        }
                    },
                    WriteCommandToStdin(text) => {
                        let mut mc_stdin = mc_server.mc_stdin.lock().await;
                        if let Some(mc_stdin) = &mut *mc_stdin {
                            // TODO: handle error?
                            let _ = mc_stdin.write_all((text + "\n").as_bytes()).await;
                        }
                    },
                    WriteToStdin(text) => {
                        let mut mc_stdin = mc_server.mc_stdin.lock().await;
                        if let Some(mc_stdin) = &mut *mc_stdin {
                            // TODO: handle error?
                            let _ = mc_stdin.write_all(text.as_bytes()).await;
                        }
                    },

                    StartServer => {
                        // Make sure the server is not already running
                        {
                            let mc_stdin = mc_server.mc_stdin.lock().await;
                            if mc_stdin.is_some() { continue }
                        }

                        let mut event_sender_clone = event_sender.clone();
                        // Spawn a task to drive the server process to completion
                        // and send an event when it exits
                        tokio::spawn(async move {
                            let ret = mc_server.run_server(mc_server.event_sender.clone()).await;
                            event_sender_clone.send(ServerEvent::ServerStopped(ret.0, ret.1)).await.unwrap();
                        });
                    },
                    StopServer => {
                        let mut mc_stdin = mc_server.mc_stdin.lock().await;
                        if let Some(mc_stdin) = &mut *mc_stdin {
                            // TODO: handle error?
                            let _ = mc_stdin.write_all(("stop".to_string() + "\n").as_bytes()).await;
                        }
                    },

                    EndInstance => {
                        cmd_sender.send(StopServer).await.unwrap();
                        break;
                    }
                }
            }
        });

        (mc_server, cmd_sender, event_receiver)
    }

    /// Run a minecraft server.
    // TODO: write better docs
    async fn run_server(
        &self,
        mut event_sender: mpsc::Sender<ServerEvent>
    ) -> (ExitStatus, Option<ShutdownReason>) {
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
            // TODO: verify that this cannot, in fact, be reached
            if mc_stdin.is_some() { unreachable!() };
            *mc_stdin = Some(process.stdin.take().unwrap());
        }

        let mut stdout = BufReader::new(process.stdout.take().unwrap()).lines();
        let mut stderr = BufReader::new(process.stderr.take().unwrap()).lines();

        let status_handle = tokio::spawn(async {
            process.await.expect("child process encountered an error")
        });

        let event_sender_clone = event_sender.clone();
        let stderr_handle = tokio::spawn(async move {
            use ServerEvent::*;
            let mut event_sender = event_sender_clone;

            while let Some(line) = stderr.next_line().await.unwrap() {
                event_sender.send(StderrLine(line)).await.unwrap();
            }
        });

        let stdout_handle = tokio::spawn(async move {
            use ServerEvent::*;
            // We have this return value so we can keep track of things (such
            // as a EULA that needs agreed to) and send that along with the
            // server shutdown event
            //
            // This makes things much easier on the library user as they don't
            // need to come up with a separate mechanism for doing that
            let mut shutdown_reason = None;

            while let Some(line) = stdout.next_line().await.unwrap() {
                let parsed = match ConsoleMsgSpecific::try_parse_from(&line) {
                    Some(msg) => msg,
                    None => {
                        // spigot servers print lines that reach this branch ("\n",
                        // "Loading libraries, please wait...")
                        event_sender.send(StdoutLine(line)).await.unwrap();
                        continue;
                    }
                };

                match &parsed {
                    ConsoleMsgSpecific::MustAcceptEula(_) => {
                        shutdown_reason = Some(ShutdownReason::EulaNotAccepted);
                    },
                    _ => {}
                }

                event_sender.send(ConsoleEvent(parsed)).await.unwrap();
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
}
