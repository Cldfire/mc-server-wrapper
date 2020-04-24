use log::{info, warn};
use std::{collections::HashSet, sync::Arc};

use twilight::{
    command_parser::{Command, CommandParserConfig, Parser},
    gateway::shard::Event,
    gateway::{Cluster, ClusterConfig},
    http::Client as DiscordClient,
    model::channel::message::MessageType,
    model::gateway::GatewayIntents,
    model::id::ChannelId,
};

use mc_server_wrapper_lib::communication::*;
use mc_server_wrapper_lib::parse::*;
use minecraft_chat::{Color, MessageBuilder, Payload};

use util::format_online_players;

use crate::error::*;
use tokio::sync::Mutex;

pub mod util;

/// Represents a maybe-present Discord bridge to a single text channel
///
/// All operations are no-ops if this struct is constructed without the
/// necessary info to set up the Discord bridge.
///
/// This struct can be cloned and passed around as needed.
#[derive(Debug, Clone)]
pub struct DiscordBridge {
    client: Option<DiscordClient>,
    cluster: Option<Cluster>,
    /// The ID of the channel we're bridging to
    bridge_channel_id: ChannelId,
}

impl DiscordBridge {
    /// Sets up a bridge to Discord with the given `token` and `bridge_channel_id`
    pub async fn new(token: String, bridge_channel_id: ChannelId) -> Result<Self, Error> {
        let client = DiscordClient::new(&token);
        let cluster_config = ClusterConfig::builder(&token)
            .intents(Some(
                GatewayIntents::GUILD_MESSAGES | GatewayIntents::GUILDS,
            ))
            .build();
        let cluster = Cluster::new(cluster_config);
        cluster.up().await?;

        Ok(Self {
            client: Some(client),
            cluster: Some(cluster),
            bridge_channel_id,
        })
    }

    /// Constructs an instance of this struct that does nothing
    pub fn new_noop() -> Self {
        Self {
            client: None,
            cluster: None,
            bridge_channel_id: ChannelId(0),
        }
    }

    /// Provides access to the `Cluster` inside this struct
    pub fn cluster(&self) -> Option<&Cluster> {
        self.cluster.as_ref()
    }

    /// Constructs a command parser for Discord commands
    pub fn command_parser<'a>() -> Parser<'a> {
        let mut config = CommandParserConfig::new();

        config.command("list").add();

        // TODO: make this configurable
        config.add_prefix("!mc ");

        Parser::new(config)
    }

    /// Handle an event from Discord
    ///
    /// The event can optionally be mapped to a command to be sent to a Minecraft
    /// server.
    ///
    /// The provided `cmd_parser` is used to parse commands (not `ServerCommands`)
    /// from Discord messages.
    pub async fn handle_discord_event<'a>(
        &self,
        event: (u64, Event),
        cmd_parser: Parser<'a>,
        online_players: Arc<Mutex<HashSet<String>>>,
    ) -> Result<Option<ServerCommand>, Error> {
        match event {
            (_, Event::Ready(_)) => {
                info!("Discord bridge online");
                Ok(None)
            }
            (_, Event::GuildCreate(guild)) => {
                info!("Connected to guild {}", guild.name);
                Ok(None)
            }
            (_, Event::MessageCreate(msg)) => {
                if msg.kind == MessageType::Regular
                    && !msg.author.bot
                    && msg.channel_id == self.bridge_channel_id
                {
                    if let Some(command) = cmd_parser.parse(&msg.content) {
                        match command {
                            Command { name: "list", .. } => {
                                let response = {
                                    let online_players = online_players.lock().await;
                                    format_online_players(&online_players, false)
                                };

                                self.clone().send_channel_msg(response);
                            }
                            _ => {}
                        }

                        return Ok(None);
                    }

                    let tellraw_msg = MessageBuilder::builder(Payload::text("[D] "))
                        .bold(true)
                        .color(Color::LightPurple)
                        .then(Payload::text(&format!(
                            "<{}> {}",
                            &msg.author.name, &msg.content
                        )))
                        .bold(false)
                        .color(Color::White)
                        .build();

                    // TODO: This will not add the message to the server logs
                    println!(
                        "{}",
                        ConsoleMsg::new(
                            ConsoleMsgType::Info,
                            format!("[D] <{}> {}", &msg.author.name, &msg.content)
                        )
                    );

                    Ok(Some(ServerCommand::TellRaw(tellraw_msg.to_json().unwrap())))
                } else {
                    Ok(None)
                }
            }
            _ => Ok(None),
        }
    }

    /// Sends the given text to the channel being bridged to
    ///
    /// A new task is spawned to send the message
    pub fn send_channel_msg<T: Into<String> + Send + 'static>(self, text: T) {
        tokio::spawn(async move {
            if let Some(client) = self.client {
                if let Err(e) = client
                    .create_message(self.bridge_channel_id)
                    .content(text)
                    .await
                {
                    warn!("Failed to send Discord message: {}", e);
                }
            }
        });
    }

    /// Sets the topic of the channel being bridged to to `text`
    ///
    /// A new task is spawned to change the topic
    // TODO: currently does not work, see https://github.com/twilight-rs/twilight/issues/149
    pub fn set_channel_topic<T: Into<String> + Send + 'static>(self, text: T) {
        tokio::spawn(async move {
            if let Some(client) = self.client {
                if let Err(e) = client
                    .update_channel(self.bridge_channel_id)
                    .topic(text)
                    .await
                {
                    warn!("Failed to set Discord channel topic: {}", e);
                }
            }
        });
    }
}
