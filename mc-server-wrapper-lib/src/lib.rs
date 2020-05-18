use tokio::{
    fs::File,
    io::BufReader,
    prelude::*,
    process,
    process::ChildStdin,
    stream::StreamExt,
    sync::{mpsc, Mutex},
};

use thiserror::Error;

use once_cell::sync::OnceCell;

use std::{
    ffi::OsStr,
    io,
    path::PathBuf,
    process::{ExitStatus, Stdio},
    sync::Arc,
};

use crate::{
    communication::*,
    parse::{ConsoleMsg, ConsoleMsgSpecific},
};

pub mod communication;
pub mod parse;
#[cfg(test)]
mod test;

/// The value that `ConsoleMsg.log()` will use for `log!`'s target parameter
///
/// Will be set to a default of `mc` if not set elsewhere.
pub static CONSOLE_MSG_LOG_TARGET: OnceCell<&str> = OnceCell::new();

/// Configuration provided to setup an `McServer` instance.
// TODO: make a builder for this
#[derive(Debug)]
pub struct McServerConfig {
    /// The path to the server jarfile
    server_path: PathBuf,
    /// The amount of memory in megabytes to allocate for the server
    memory: u16,
    /// Custom flags to pass to the JVM
    jvm_flags: Option<String>,
    /// Whether or not the server's `stdin` should be inherited from the parent
    /// process's `stdin`.
    ///
    /// An `McServer` constructed with `inherit_stdin` set to true will ignore
    /// any commands it receives to write to the server's stdin.
    ///
    /// Set this to true if you want simple hands-free passthrough of whatever
    /// you enter on the console to the Minecraft server. Set this to false
    /// if you'd rather manually handle stdin and send data to the Minecraft
    /// server.
    inherit_stdin: bool,
}

/// Errors regarding an `McServerConfig`
#[derive(Error, Debug)]
pub enum McServerConfigError {
    #[error("the provided server path \"{0}\" was not an accessible file")]
    ServerPathFileNotPresent(PathBuf),
}

impl McServerConfig {
    /// Create a new `McServerConfig`
    pub fn new<P: Into<PathBuf>>(
        server_path: P,
        memory: u16,
        jvm_flags: Option<String>,
        inherit_stdin: bool,
    ) -> Self {
        let server_path = server_path.into();
        McServerConfig {
            server_path,
            memory,
            jvm_flags,
            inherit_stdin,
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
            Err(ServerPathFileNotPresent(self.server_path))
        } else {
            Ok(self)
        }
    }
}

/// Represents a single wrapped Minecraft server that may be running or stopped
// TODO: the design here is dumb. `McServer` should not hold a reference to
// `McServerInternal`.
//
// The conceptual problem here is that we have 3 lifetimes to be concerned about.
// The first is the lifetime of the channels that the user of the library uses
// to communicate with the possibly-running Minecraft server. The second is the
// lifetime of whatever handles that communication on the library side (the
// command listener). The third are the lifetimes of the individual Minecraft
// server processes themselves.
//
// The current design is mixing these lifetimes together into a tangled mess.
// Things need to be broken apart so that each lifetime is clearly defined in
// the way things are structured.
#[derive(Debug)]
pub struct McServer {
    /// Channel through which commands can be sent to the server
    pub cmd_sender: mpsc::Sender<ServerCommand>,
    /// Channel through which events are received from the server
    pub event_receiver: mpsc::Receiver<ServerEvent>,
    // A reference to internal stuff so we can provide the `running` method
    internal: Arc<McServerInternal>,
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

        let (cmd_sender, cmd_receiver) = mpsc::channel::<ServerCommand>(64);
        let (event_sender, event_receiver) = mpsc::channel::<ServerEvent>(64);

        let internal = Arc::new(McServerInternal {
            config,
            event_sender,
            mc_stdin: Mutex::new(None),
            shutdown_reason: Mutex::new(None),
            running: Mutex::new(false),
        });
        internal.clone().spawn_listener(cmd_receiver);

        Ok(McServer {
            cmd_sender,
            event_receiver,
            internal,
        })
    }

    /// Returns true if the Minecraft server is currently running
    pub async fn running(&self) -> bool {
        self.internal.running().await
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
    /// Handle to the server's stdin if it's running and stdin is being piped
    mc_stdin: Mutex<Option<process::ChildStdin>>,
    /// Keeps track of a reason why the server had to shut down
    ///
    /// An example is the EULA needing to be accepted. This allows us to provide
    /// the library user with some context when a shutdown occurs.
    shutdown_reason: Mutex<Option<ShutdownReason>>,
    /// Whether or not the server is currently running
    running: Mutex<bool>,
}

impl McServerInternal {
    /// Spawn a task to listen for and handle incoming `ServerCommand`s
    // TODO: if we're smarter about method boundaries we could get rid of the
    // `Arc<Self>` and have the `cmd_receiver` as a field of the struct
    fn spawn_listener(self: Arc<Self>, mut cmd_receiver: mpsc::Receiver<ServerCommand>) {
        tokio::spawn(async move {
            while let Some(cmd) = cmd_receiver.next().await {
                use ServerCommand::*;
                use ServerEvent::*;
                let mc_server_internal = self.clone();

                match cmd {
                    TellRawAll(json) => {
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
                        if mc_server_internal.running().await {
                            continue;
                        }

                        // Spawn a task to drive the server process to completion
                        // and send an event when it exits
                        tokio::spawn(async move {
                            let ret = mc_server_internal.clone().run_server().await;
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
    }

    /// Run a minecraft server.
    // TODO: write better docs
    // TODO: maybe split into functions that start the server and interface
    // with it
    // TODO: audit unwrapping
    async fn run_server(self: Arc<Self>) -> (io::Result<ExitStatus>, Option<ShutdownReason>) {
        self.clear_shutdown_reason().await;

        let folder = self
            .config
            .server_path
            .as_path()
            .parent()
            .map(|p| p.as_os_str())
            .unwrap_or_else(|| OsStr::new("."));
        let file = self.config.server_path.file_name().unwrap();

        let mut process = process::Command::new("sh")
            .stdin(if self.config.inherit_stdin {
                Stdio::inherit()
            } else {
                Stdio::piped()
            })
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

        if !self.config.inherit_stdin {
            self.set_stdin(Some(process.stdin.take().unwrap())).await;
        }
        self.set_running(true).await;

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
        let self_clone = self.clone();
        let stdout_handle = tokio::spawn(async move {
            use ServerEvent::*;
            let mut event_sender = event_sender_clone;
            let self_clone = self_clone;

            while let Some(line) = stdout.next_line().await.unwrap() {
                if let Some(console_msg) = ConsoleMsg::try_parse_from(&line) {
                    let specific_msg = ConsoleMsgSpecific::try_parse_from(&console_msg);

                    if specific_msg == Some(ConsoleMsgSpecific::MustAcceptEula) {
                        self_clone
                            .set_shutdown_reason(Some(ShutdownReason::EulaNotAccepted))
                            .await;
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
        });

        let (status, _, _) = tokio::join!(status_handle, stdout_handle, stderr_handle,);

        if !self.config.inherit_stdin {
            self.set_stdin(None).await;
        }
        self.set_running(false).await;

        (status.unwrap(), self.shutdown_reason().await)
    }

    /// Set the stdin handle
    async fn set_stdin(&self, to: Option<ChildStdin>) {
        let mut mc_stdin = self.mc_stdin.lock().await;
        *mc_stdin = to;
    }

    /// Set the shutdown reason
    async fn set_shutdown_reason(&self, to: Option<ShutdownReason>) {
        let mut shutdown_reason = self.shutdown_reason.lock().await;
        *shutdown_reason = to;
    }

    /// Clear the currently stored shutdown reason
    async fn clear_shutdown_reason(&self) {
        self.set_shutdown_reason(None).await;
    }

    /// Get the value of `shutdown_reason`
    async fn shutdown_reason(&self) -> Option<ShutdownReason> {
        let shutdown_reason = self.shutdown_reason.lock().await;
        shutdown_reason.clone()
    }

    /// Set the value of `running`
    async fn set_running(&self, to: bool) {
        let mut running = self.running.lock().await;
        *running = to;
    }

    /// Returns true if the server is currently running
    async fn running(&self) -> bool {
        let running = self.running.lock().await;
        *running
    }

    /// Writes the given bytes to the server's stdin if the server is running
    async fn write_to_stdin<B: AsRef<[u8]>>(&self, bytes: B) -> io::Result<()> {
        let bytes = bytes.as_ref();

        if bytes == b"stop\n" {
            self.set_shutdown_reason(Some(ShutdownReason::RequestedToStop))
                .await;
        }

        let mut stdin = self.mc_stdin.lock().await;
        if let Some(stdin) = &mut *stdin {
            stdin.write_all(bytes).await
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
