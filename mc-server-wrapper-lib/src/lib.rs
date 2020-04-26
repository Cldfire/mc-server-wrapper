use tokio::fs::File;
use tokio::io::BufReader;
use tokio::prelude::*;
use tokio::process;
use tokio::stream::StreamExt;
use tokio::sync::mpsc;
use tokio::sync::Mutex;

use thiserror::Error;

use once_cell::sync::OnceCell;

use std::io;
use std::path::PathBuf;
use std::process::{ExitStatus, Stdio};
use std::{ffi::OsStr, sync::Arc};

use crate::communication::*;
use crate::parse::{ConsoleMsg, ConsoleMsgSpecific};

pub mod communication;
pub mod parse;
#[cfg(test)]
mod test;

/// The value that `ConsoleMsg.log()` will use for `log!`'s target parameter
///
/// Will be set to a default of `mc` if not set elsewhere.
pub static CONSOLE_MSG_LOG_TARGET: OnceCell<&str> = OnceCell::new();

/// Configuration provided to setup an `McServer` instance.
#[derive(Debug)]
pub struct McServerConfig {
    /// The path to the server jarfile
    server_path: PathBuf,
    /// The amount of memory in megabytes to allocate for the server
    memory: u16,
    /// Custom flags to pass to the JVM
    jvm_flags: Option<String>,
}

/// Errors regarding an `McServerConfig`
#[derive(Error, Debug)]
pub enum McServerConfigError {
    #[error("the provided server path \"{0}\" was not an accessible file")]
    ServerPathFileNotPresent(PathBuf),
}

impl McServerConfig {
    /// Create a new `McServerConfig`
    pub fn new<P: Into<PathBuf>>(server_path: P, memory: u16, jvm_flags: Option<String>) -> Self {
        let server_path = server_path.into();
        McServerConfig {
            server_path,
            memory,
            jvm_flags,
        }
    }

    /// Validates aspects of the config
    ///
    /// The validation ensures that the provided `server_path` is a path to a
    /// file present on the filesystem. The config will be returned to you
    /// if it is valid.
    pub fn validate(self) -> Result<Self, McServerConfigError> {
        use McServerConfigError::*;

        if !self.server_path.is_file() {
            Err(ServerPathFileNotPresent(self.server_path.clone()))
        } else {
            Ok(self)
        }
    }
}

/// Represents a single wrapped Minecraft server that may be running or stopped
#[derive(Debug)]
pub struct McServer {
    /// Channel through which commands can be sent to the server
    pub cmd_sender: mpsc::Sender<ServerCommand>,
    /// Channel through which events are received from the server
    pub event_receiver: mpsc::Receiver<ServerEvent>,
}

impl McServer {
    /// Create a new `McServer` with the given `McServerConfig`.
    ///
    /// The server config will be validated before it is used.
    ///
    /// Note that the Minecraft server is not launched until you send a command
    /// to cause that to happen.
    // TODO: should this be called `manage`?
    pub fn new(config: McServerConfig) -> Result<Self, McServerConfigError> {
        let config = config.validate()?;

        let (cmd_sender, mut cmd_receiver) = mpsc::channel::<ServerCommand>(64);
        let (event_sender, event_receiver) = mpsc::channel::<ServerEvent>(64);
        let mc_server_internal = Arc::new(McServerInternal {
            config,
            event_sender,
            mc_stdin: Arc::new(Mutex::new(None)),
        });

        // Start a task to receive server commands and handle them appropriately
        // TODO: move this out of this function
        tokio::spawn(async move {
            while let Some(cmd) = cmd_receiver.next().await {
                use ServerCommand::*;
                use ServerEvent::*;
                let mc_server_internal = mc_server_internal.clone();

                match cmd {
                    TellRaw(json) => {
                        // TODO: handle error
                        let _ = mc_server_internal
                            .write_to_stdin(format!("tellraw @a {}\n", json))
                            .await;
                    }
                    WriteCommandToStdin(text) => {
                        // TODO: handle error
                        let _ = mc_server_internal.write_to_stdin(text + "\n").await;
                    }
                    WriteToStdin(text) => {
                        // TODO: handle error
                        let _ = mc_server_internal.write_to_stdin(text).await;
                    }

                    AgreeToEula => {
                        tokio::spawn(async move {
                            mc_server_internal
                                .event_sender
                                .clone()
                                .send(AgreeToEulaResult(mc_server_internal.agree_to_eula().await))
                                .await
                                .unwrap();
                        });
                    }
                    StartServer => {
                        if mc_server_internal.server_running().await {
                            continue;
                        }

                        // Spawn a task to drive the server process to completion
                        // and send an event when it exits
                        tokio::spawn(async move {
                            let ret = mc_server_internal.run_server().await;
                            mc_server_internal
                                .event_sender
                                .clone()
                                .send(ServerStopped(ret.0, ret.1))
                                .await
                                .unwrap();
                        });
                    }
                    StopServer { forever } => {
                        // TODO: handle error
                        let _ = mc_server_internal.write_to_stdin("stop\n").await;

                        if forever {
                            break;
                        }
                    }
                }
            }
        });

        Ok(McServer {
            cmd_sender,
            event_receiver,
        })
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
    mc_stdin: Arc<Mutex<Option<process::ChildStdin>>>,
}

impl McServerInternal {
    /// Run a minecraft server.
    // TODO: write better docs
    // TODO: maybe split into functions that start the server and interface
    // with it
    // TODO: audit unwrapping
    async fn run_server(&self) -> (tokio::io::Result<ExitStatus>, Option<ShutdownReason>) {
        let folder = self
            .config
            .server_path
            .as_path()
            .parent()
            .map(|p| p.as_os_str())
            .unwrap_or_else(|| OsStr::new("."));
        let file = self.config.server_path.file_name().unwrap();

        let mut process = process::Command::new("sh")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .args(&[
                "-c",
                &format!(
                    "cd {:?} && exec java -Xms{}M -Xmx{}M {} -jar {:?} nogui",
                    folder,
                    self.config.memory,
                    self.config.memory,
                    self.config.jvm_flags.as_deref().unwrap_or(""),
                    file
                ),
            ])
            .spawn()
            .unwrap();

        // Update the stored handle to the server's stdin
        {
            let mut mc_stdin = self.mc_stdin.lock().await;
            if mc_stdin.is_some() {
                unreachable!()
            };
            *mc_stdin = Some(process.stdin.take().unwrap());
        }

        let mut stdout = BufReader::new(process.stdout.take().unwrap()).lines();
        let mut stderr = BufReader::new(process.stderr.take().unwrap()).lines();

        let status_handle = tokio::spawn(async { process.await });

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

                    event_sender
                        .send(ConsoleEvent(console_msg, specific_msg))
                        .await
                        .unwrap();
                } else {
                    // spigot servers print lines that reach this branch ("\n",
                    // "Loading libraries, please wait...")
                    event_sender.send(StdoutLine(line)).await.unwrap();
                }
            }

            shutdown_reason
        });

        let (status, stdout_val, _) = tokio::join!(status_handle, stdout_handle, stderr_handle,);

        // Update the stored handle to the server's stdin
        {
            let mut mc_stdin = self.mc_stdin.lock().await;
            if mc_stdin.is_none() {
                unreachable!()
            };
            *mc_stdin = None;
        }

        (status.unwrap(), stdout_val.unwrap())
    }

    /// Returns true if the server is currently running
    async fn server_running(&self) -> bool {
        let mc_stdin = self.mc_stdin.lock().await;
        mc_stdin.is_some()
    }

    /// Writes the given bytes to the server's stdin if the server is running
    async fn write_to_stdin<B: AsRef<[u8]>>(&self, bytes: B) -> io::Result<()> {
        let mut stdin = self.mc_stdin.lock().await;
        if let Some(stdin) = &mut *stdin {
            stdin.write_all(bytes.as_ref()).await
        } else {
            Ok(())
        }
    }

    /// Overwrites the `eula.txt` file with the contents `eula=true`.
    async fn agree_to_eula(&self) -> io::Result<()> {
        let mut file = File::create(self.config.server_path.with_file_name("eula.txt")).await?;

        file.write_all(b"eula=true").await
    }
}
