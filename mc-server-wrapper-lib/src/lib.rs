use tokio::{
    fs::File,
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    process,
    sync::{mpsc, oneshot, Mutex},
};

use thiserror::Error;

use once_cell::sync::OnceCell;

use std::{
    ffi::OsStr,
    io,
    path::{Path, PathBuf},
    process::{ExitStatus, Stdio},
    sync::Arc,
};

use crate::{
    communication::*,
    parse::{ConsoleMsg, ConsoleMsgSpecific},
};
use process::Child;

pub mod communication;
pub mod parse;
#[cfg(test)]
mod test;

/// The value that `ConsoleMsg.log()` will use for `log!`'s target parameter
///
/// Will be set to a default of `mc` if not set elsewhere.
pub static CONSOLE_MSG_LOG_TARGET: OnceCell<&str> = OnceCell::new();

/// Configuration to run a Minecraft server instance with
// TODO: make a builder for this
#[derive(Debug, Clone)]
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
    /// An server constructed with `inherit_stdin` set to true will ignore
    /// any commands it receives to write to the server's stdin.
    ///
    /// Set this to true if you want simple hands-free passthrough of whatever
    /// you enter on the console to the Minecraft server. Set this to false
    /// if you'd rather manually handle stdin and send data to the Minecraft
    /// server yourself (more work, but more flexible).
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
    /// file present on the filesystem.
    pub fn validate(&self) -> Result<(), McServerConfigError> {
        use McServerConfigError::*;

        if !self.server_path.is_file() {
            return Err(ServerPathFileNotPresent(self.server_path.clone()));
        }

        Ok(())
    }
}

/// Errors that can occur when starting up a server
#[derive(Error, Debug)]
pub enum McServerStartError {
    #[error("config error: {0}")]
    ConfigError(#[from] McServerConfigError),
    #[error("io error: {0}")]
    IoError(#[from] io::Error),
    #[error(
        "no config provided with the request to start the server and no previous \
        config existed"
    )]
    NoPreviousConfig,
}

/// Manages a single Minecraft server, running or stopped
#[derive(Debug)]
pub struct McServerManager {
    /// Handle to server internals (present if server is running)
    internal: Arc<Mutex<Option<McServerInternal>>>,
}

impl McServerManager {
    /// Create a new `McServerManager`
    ///
    /// The returned channel halves can be used to send commands to the server
    /// and receive events from the server.
    ///
    /// Commands that require the server to be running to do anything will be
    /// ignored.
    pub fn new() -> (
        Arc<Self>,
        mpsc::Sender<ServerCommand>,
        mpsc::Receiver<ServerEvent>,
    ) {
        let (cmd_sender, cmd_receiver) = mpsc::channel::<ServerCommand>(64);
        let (event_sender, event_receiver) = mpsc::channel::<ServerEvent>(64);

        let server = Arc::new(McServerManager {
            internal: Arc::new(Mutex::new(None)),
        });

        let self_clone = server.clone();
        self_clone.spawn_listener(event_sender, cmd_receiver);

        (server, cmd_sender, event_receiver)
    }

    fn spawn_listener(
        self: Arc<Self>,
        event_sender: mpsc::Sender<ServerEvent>,
        mut cmd_receiver: mpsc::Receiver<ServerCommand>,
    ) {
        tokio::spawn(async move {
            let mut current_config: Option<McServerConfig> = None;

            while let Some(cmd) = cmd_receiver.recv().await {
                use ServerCommand::*;
                use ServerEvent::*;

                match cmd {
                    TellRawAll(json) => {
                        let _ = self.write_to_stdin(format!("tellraw @a {}\n", json)).await;
                    }
                    WriteCommandToStdin(text) => {
                        let _ = self.write_to_stdin(text + "\n").await;
                    }
                    WriteToStdin(text) => {
                        let _ = self.write_to_stdin(text).await;
                    }

                    AgreeToEula => {
                        let event_sender_clone = event_sender.clone();

                        if let Some(config) = &current_config {
                            let server_path = config.server_path.clone();
                            tokio::spawn(async move {
                                event_sender_clone
                                    .send(AgreeToEulaResult(
                                        McServerManager::agree_to_eula(server_path).await,
                                    ))
                                    .await
                                    .unwrap();
                            });
                        }
                    }
                    StartServer { config } => {
                        if self.running().await {
                            continue;
                        }

                        let config = if let Some(config) = config {
                            current_config = Some(config);
                            current_config.as_ref().unwrap()
                        } else if let Some(current_config) = &current_config {
                            current_config
                        } else {
                            event_sender
                                .send(ServerEvent::StartServerResult(Err(
                                    McServerStartError::NoPreviousConfig,
                                )))
                                .await
                                .unwrap();
                            continue;
                        };

                        let (child, rx) = match McServerInternal::setup_server(config) {
                            Ok((internal, child, rx)) => {
                                *self.internal.lock().await = Some(internal);
                                (child, rx)
                            }
                            Err(e) => {
                                event_sender
                                    .send(ServerEvent::StartServerResult(Err(e)))
                                    .await
                                    .unwrap();
                                continue;
                            }
                        };

                        let event_sender_clone = event_sender.clone();
                        let internal_clone = self.internal.clone();

                        // Spawn a task to drive the server process to completion
                        // and send an event when it exits
                        tokio::spawn(async move {
                            let event_sender = event_sender_clone;
                            let ret =
                                McServerInternal::run_server(child, rx, event_sender.clone()).await;
                            let _ = internal_clone.lock().await.take();

                            event_sender
                                .send(ServerStopped(ret.0, ret.1))
                                .await
                                .unwrap();
                        });
                    }
                    StopServer { forever } => {
                        // TODO: handle error
                        let _ = self.write_to_stdin("stop\n").await;

                        if forever {
                            break;
                        }
                    }
                }
            }
        });
    }

    /// Writes the given bytes to the server's stdin if the server is running
    async fn write_to_stdin<B: AsRef<[u8]>>(&self, bytes: B) {
        let bytes = bytes.as_ref();
        let mut internal = self.internal.lock().await;

        if let Some(internal) = &mut *internal {
            if bytes == b"stop\n" {
                if let Some(tx) = internal.shutdown_reason_oneshot.take() {
                    let _ = tx.send(ShutdownReason::RequestedToStop);
                }
            }

            if let Some(stdin) = &mut internal.stdin {
                if let Err(e) = stdin.write_all(bytes).await {
                    log::warn!("Failed to write to Minecraft server stdin: {}", e);
                }
            }
        }
    }

    /// Returns true if the server is currently running
    pub async fn running(&self) -> bool {
        let running = self.internal.lock().await;
        running.is_some()
    }

    /// Overwrites the `eula.txt` file with the contents `eula=true`.
    async fn agree_to_eula<P: AsRef<Path>>(server_path: P) -> io::Result<()> {
        let mut file = File::create(server_path.as_ref().with_file_name("eula.txt")).await?;

        file.write_all(b"eula=true").await
    }
}

/// Groups together stuff needed internally by the library
///
/// Anything inside of here needs to both be accessed by the manager and have
/// its lifetime tied to the Minecraft server process it was created for.
///
/// Stuff that needs to be tied to the lifetime of the server process but does
/// not need to be accessed by the manager should be passed directly into
/// `run_server`.
///
/// Stuff that needs to outlive the Minecraft server process belongs at the
/// manager level (either in the struct, if it needs to be accessed by the
/// library consumer, or in `spawn_listener` if not).
#[derive(Debug)]
struct McServerInternal {
    /// Handle to the server's stdin (if captured)
    stdin: Option<process::ChildStdin>,
    /// Provides a way for the manager to set a shutdown reason
    shutdown_reason_oneshot: Option<oneshot::Sender<ShutdownReason>>,
}

impl McServerInternal {
    /// Set up the server process with the given config
    ///
    /// The config will be validated before it is used.
    fn setup_server(
        config: &McServerConfig,
    ) -> Result<(Self, Child, oneshot::Receiver<ShutdownReason>), McServerStartError> {
        config.validate()?;

        let folder = config
            .server_path
            .as_path()
            .parent()
            .map(|p| p.as_os_str())
            .unwrap_or_else(|| OsStr::new("."));
        let file = config.server_path.file_name().unwrap();

        let java_args = format!(
            "-Xms{}M -Xmx{}M {} -jar {:?} nogui",
            config.memory,
            config.memory,
            config.jvm_flags.as_deref().unwrap_or(""),
            file
        );

        // I don't know much about powershell but this works so ¯\_(ツ)_/¯
        let args = if cfg!(windows) {
            vec![
                "Start-Process",
                "-NoNewWindow",
                "-FilePath",
                "java.exe",
                "-WorkingDirectory",
                &folder.to_string_lossy(),
                "-ArgumentList",
                &format!("'{}'", &java_args),
            ]
            .into_iter()
            .map(|s| s.into())
            .collect()
        } else {
            vec![
                "-c".into(),
                format!(
                    "cd {} && exec java {}",
                    folder.to_string_lossy(),
                    &java_args
                ),
            ]
        };

        let mut process = process::Command::new(if cfg!(windows) { "PowerShell" } else { "sh" })
            .stdin(if config.inherit_stdin {
                Stdio::inherit()
            } else {
                Stdio::piped()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .args(&args)
            .spawn()?;

        let stdin = if !config.inherit_stdin {
            Some(process.stdin.take().unwrap())
        } else {
            None
        };

        let (tx, rx) = oneshot::channel();

        Ok((
            Self {
                stdin,
                shutdown_reason_oneshot: Some(tx),
            },
            process,
            rx,
        ))
    }

    /// Drive the given server process to completion, sending any events over the
    /// `event_sender`
    async fn run_server(
        mut process: Child,
        mut shutdown_reason_oneshot: oneshot::Receiver<ShutdownReason>,
        event_sender: mpsc::Sender<ServerEvent>,
    ) -> (io::Result<ExitStatus>, Option<ShutdownReason>) {
        let mut stdout = BufReader::new(process.stdout.take().unwrap()).lines();
        let mut stderr = BufReader::new(process.stderr.take().unwrap()).lines();

        let status_handle = tokio::spawn(async move { process.wait().await });

        let event_sender_clone = event_sender.clone();
        let stderr_handle = tokio::spawn(async move {
            use ServerEvent::*;
            let event_sender = event_sender_clone;

            while let Some(line) = stderr.next_line().await.unwrap() {
                event_sender.send(StderrLine(line)).await.unwrap();
            }
        });

        let stdout_handle = tokio::spawn(async move {
            use ServerEvent::*;
            let event_sender = event_sender;
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

        let (status, shutdown_reason, _) =
            tokio::join!(status_handle, stdout_handle, stderr_handle,);
        let mut shutdown_reason = shutdown_reason.unwrap();

        // Shutdown reason from the manager gets preference
        if let Ok(reason) = shutdown_reason_oneshot.try_recv() {
            shutdown_reason = Some(reason);
        }

        (status.unwrap(), shutdown_reason)
    }
}
