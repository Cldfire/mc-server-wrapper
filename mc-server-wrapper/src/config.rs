use crate::Opt;
use anyhow::{anyhow, Context};
use notify_debouncer_mini::{new_debouncer, notify, DebouncedEvent};
use serde_derive::{Deserialize, Serialize};
use std::{
    num::NonZeroU64,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncWriteExt},
    sync::mpsc,
};

/// Represents the mc-server-wrapper config structure
#[derive(Serialize, Deserialize, Debug)]
pub struct Config {
    /// Minecraft-related config options
    pub minecraft: Minecraft,
    /// Discord-related config options
    pub discord: Option<Discord>,
    /// Logging-related config options
    pub logging: Logging,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            minecraft: Minecraft::default(),
            discord: Some(Discord::default()),
            logging: Logging::default(),
        }
    }
}

impl Config {
    /// Load a config file at `path`
    ///
    /// If the config does not exist at the path a default config will be created,
    /// returned, and also written to the path.
    ///
    /// This will not overwrite an existing file, however.
    pub async fn load(path: impl AsRef<Path>) -> Result<Self, anyhow::Error> {
        let path = path.as_ref();
        if !path.exists() {
            let default_config = Self::default();
            default_config
                .store(path)
                .await
                .with_context(|| "Failed to save default config file")?;

            Ok(default_config)
        } else {
            let mut file = File::open(path)
                .await
                .with_context(|| format!("Failed to open config file at {:?}", path))?;
            let mut buffer = String::new();
            file.read_to_string(&mut buffer)
                .await
                .with_context(|| format!("Failed to read config file at {:?}", path))?;

            Ok(toml::from_str(&buffer)
                .with_context(|| format!("Failed to parse config file at {:?}", path))?)
        }
    }

    /// Write the current config to `path`
    ///
    /// This will overwrite whatever file is currently at `path`.
    pub async fn store(&self, path: impl AsRef<Path>) -> Result<(), anyhow::Error> {
        let path = path.as_ref();
        let mut file = File::create(path)
            .await
            .with_context(|| format!("Failed to open config file at {:?}", path))?;

        file.write_all(toml::to_string(self)?.as_bytes())
            .await
            .with_context(|| format!("Failed to write config file to {:?}", path))
    }

    /// Merge args passed in via the CLI into this config
    pub fn merge_in_args(&mut self, args: Opt) -> Result<(), anyhow::Error> {
        if args.bridge_to_discord {
            if let Some(discord) = &mut self.discord {
                discord.enable_bridge = true;
            } else {
                return Err(anyhow!(
                    "Discord bridge cannot be enabled if the bot token and channel ID \
                    are not specified in the config"
                ));
            }
        }

        if let Some(path) = args.server_path {
            self.minecraft.server_path = path;
        }

        Ok(())
    }

    /// Setup a file watcher to be notified when the config file changes
    ///
    /// This spawns a separate thread to watch the config file because there aren't
    /// any file watcher libs that integrate with tokio right now.
    pub fn setup_watcher(
        &self,
        config_filepath: impl Into<PathBuf>,
    ) -> mpsc::Receiver<Result<Vec<DebouncedEvent>, notify::Error>> {
        let (notify_sender, notify_receiver) = mpsc::channel(8);
        let config_filepath = config_filepath.into();
        let handle = tokio::runtime::Handle::current();

        std::thread::spawn(move || {
            let (tx, rx) = std::sync::mpsc::channel();

            let mut debouncer = new_debouncer(Duration::from_millis(500), tx).unwrap();

            debouncer
                .watcher()
                .watch(&config_filepath, notify::RecursiveMode::NonRecursive)
                .unwrap();

            loop {
                // rx.recv() can only error if the sender was disconnected
                //
                // This should never occur, so it's safe to unwrap here
                let event = rx.recv().unwrap();
                let sender_clone = notify_sender.clone();
                handle.spawn(async move {
                    sender_clone.send(event).await.unwrap();
                });
            }
        });

        notify_receiver
    }
}

/// Minecraft-related config options
#[derive(Serialize, Deserialize, Debug)]
pub struct Minecraft {
    /// Path to the Minecraft server jar
    pub server_path: PathBuf,
    /// Amount of memory in megabytes to allocate for the server
    pub memory: u16,
    /// Custom flags to pass to the JVM
    pub jvm_flags: Option<String>,
}

impl Default for Minecraft {
    fn default() -> Self {
        Self {
            server_path: "./server.jar".into(),
            memory: 1024,
            jvm_flags: None,
        }
    }
}

/// Discord-related config options
#[derive(Serialize, Deserialize, Debug)]
pub struct Discord {
    pub enable_bridge: bool,
    pub token: String,
    pub channel_id: NonZeroU64,
    pub update_status: bool,
}

impl Default for Discord {
    fn default() -> Self {
        Self {
            enable_bridge: false,
            token: "".into(),
            channel_id: NonZeroU64::new(123).unwrap(),
            update_status: true,
        }
    }
}

/// Logging-related config options
#[derive(Serialize, Deserialize, Debug)]
pub struct Logging {
    /// Logging level for mc-server-wrapper dependencies
    ///
    /// This only affects file logging.
    #[serde(with = "LevelDef")]
    pub all: log::Level,
    /// Logging level for mc-server-wrapper dependencies
    ///
    /// This only affects file logging.
    #[serde(rename = "self")]
    #[serde(with = "LevelDef")]
    pub self_level: log::Level,
    #[serde(with = "LevelDef")]
    /// Logging level for mc-server-wrapper dependencies
    ///
    /// This only affects file logging.
    pub discord: log::Level,
}

impl Default for Logging {
    fn default() -> Self {
        Self {
            all: log::Level::Warn,
            self_level: log::Level::Debug,
            discord: log::Level::Info,
        }
    }
}

#[derive(Serialize, Deserialize)]
#[serde(remote = "log::Level")]
enum LevelDef {
    Error = 1,
    Warn,
    Info,
    Debug,
    Trace,
}
