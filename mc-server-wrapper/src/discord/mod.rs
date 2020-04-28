use log::{debug, info, warn};

use twilight::{
    command_parser::{Command, CommandParserConfig, Parser},
    gateway::shard::Event,
    gateway::{Cluster, ClusterConfig},
    http::Client as DiscordClient,
    model::channel::{message::MessageType, Message},
    model::gateway::GatewayIntents,
    model::id::ChannelId,
};

use mc_server_wrapper_lib::communication::*;
use mc_server_wrapper_lib::parse::*;
use minecraft_chat::{Color, Payload};

use util::{format_mentions_in, format_online_players, tellraw_prefix};

use crate::error::*;
use crate::ONLINE_PLAYERS;
use tokio::{stream::StreamExt, sync::mpsc::Sender};

pub mod util;

static CHAT_PREFIX: &str = "[D] ";

/// Sets up a `DiscordBridge` and starts handling events
pub async fn setup_discord(
    token: String,
    bridge_channel_id: ChannelId,
    mc_cmd_sender: Sender<ServerCommand>,
) -> Result<DiscordBridge, Error> {
    let discord = DiscordBridge::new(token, bridge_channel_id).await?;

    let discord_clone = discord.clone();
    tokio::spawn(async move {
        let discord = discord_clone;
        let cmd_parser = DiscordBridge::command_parser();

        // For all received Discord events, map the event to a `ServerCommand`
        // (if necessary) and send it to the Minecraft server
        // TODO: don't unwrap here
        let mut events = discord.cluster().unwrap().events().await;
        while let Some(e) = events.next().await {
            let discord = discord.clone();
            let cmd_sender_clone = mc_cmd_sender.clone();
            let cmd_parser_clone = cmd_parser.clone();

            tokio::spawn(async move {
                if let Err(e) = discord
                    .handle_discord_event(e, cmd_parser_clone, cmd_sender_clone)
                    .await
                {
                    warn!("Failed to handle Discord event: {:?}", e);
                }
            });
        }
    });

    Ok(discord)
}

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
    /// Connects to Discord with the given `token` and `bridge_channel_id`
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
    /// The provided `cmd_parser` is used to parse commands (not `ServerCommands`)
    /// from Discord messages.
    pub async fn handle_discord_event<'a>(
        &self,
        event: (u64, Event),
        cmd_parser: Parser<'a>,
        mc_cmd_sender: Sender<ServerCommand>,
    ) -> Result<(), Error> {
        Ok(match event {
            (_, Event::Ready(_)) => {
                info!("Discord bridge online");
            }
            (_, Event::GuildCreate(guild)) => {
                info!("Connected to guild {}", guild.name);
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
                                    let online_players = ONLINE_PLAYERS.get().unwrap().lock().await;
                                    format_online_players(&online_players, false)
                                };

                                self.clone().send_channel_msg(response);
                            }
                            _ => {}
                        }

                        return Ok(());
                    }

                    self.handle_attachments_in_msg(&msg, mc_cmd_sender.clone())
                        .await;

                    self.handle_msg_content(&msg, mc_cmd_sender.clone()).await;

                    // We handle embeds after the message contents to replicate
                    // Discord's layout (embeds after message)
                    self.handle_embeds_in_msg(&msg, mc_cmd_sender).await;
                }
            }
            _ => {}
        })
    }

    /// Handles any attachments in the given message
    async fn handle_attachments_in_msg(
        &self,
        msg: &Message,
        mut mc_cmd_sender: Sender<ServerCommand>,
    ) {
        for attachment in &msg.attachments {
            let type_str = if attachment.height.is_some() {
                "image"
            } else {
                "file"
            };

            let tellraw_msg = tellraw_prefix()
                .then(Payload::text(&format!("{} uploaded ", &msg.author.name)))
                .italic(true)
                .color(Color::Gray)
                .then(Payload::text(type_str))
                .underlined(true)
                .italic(true)
                .color(Color::Gray)
                .hover_show_text(&format!(
                    "Click to open the {} in your web browser",
                    type_str
                ))
                .click_open_url(&attachment.url)
                .build();

            ConsoleMsg::new(
                ConsoleMsgType::Info,
                format!(
                    "{}{} uploaded {}: {}",
                    CHAT_PREFIX, &msg.author.name, type_str, attachment.url
                ),
            )
            .log();

            mc_cmd_sender
                .send(ServerCommand::TellRawAll(tellraw_msg.to_json().unwrap()))
                .await
                .ok();
        }
    }

    /// Handles the content of the message
    async fn handle_msg_content(&self, msg: &Message, mut mc_cmd_sender: Sender<ServerCommand>) {
        if msg.content.is_empty() {
            debug!("Empty message from Discord: {:?}", &msg);
            return;
        }

        let content = format_mentions_in(
            msg.content.clone(),
            msg.mentions
                .iter()
                .map(|(id, user)| (id, user.name.as_str()))
                .collect(),
            &msg.mention_roles,
        );

        let tellraw_msg = tellraw_prefix()
            .then(Payload::text(&format!(
                "<{}> {}",
                &msg.author.name, &content
            )))
            .build();

        // Tellraw commands do not get logged to the console, so we
        // make up for that here
        ConsoleMsg::new(
            ConsoleMsgType::Info,
            format!("{}<{}> {}", CHAT_PREFIX, &msg.author.name, &content),
        )
        .log();

        mc_cmd_sender
            .send(ServerCommand::TellRawAll(tellraw_msg.to_json().unwrap()))
            .await
            .ok();
    }

    /// Handles any embeds in the given message
    async fn handle_embeds_in_msg(&self, msg: &Message, mut mc_cmd_sender: Sender<ServerCommand>) {
        for embed in msg.embeds.iter().filter(|e| e.url.is_some()) {
            // TODO: this is kinda ugly
            let link_text = if embed.title.is_some() && embed.provider.is_some() {
                if embed.provider.as_ref().unwrap().name.is_some() {
                    format!(
                        "{} - {}",
                        embed.provider.as_ref().unwrap().name.as_ref().unwrap(),
                        embed.title.as_ref().unwrap()
                    )
                } else {
                    embed.url.clone().unwrap()
                }
            } else {
                embed.url.clone().unwrap()
            };

            let tellraw_msg = tellraw_prefix()
                .then(Payload::text(&format!("{} linked \"", &msg.author.name)))
                .italic(true)
                .color(Color::Gray)
                .then(Payload::text(&link_text))
                .underlined(true)
                .italic(true)
                .color(Color::Gray)
                .hover_show_text("Click to open the link in your web browser")
                .click_open_url(embed.url.as_ref().unwrap())
                .then(Payload::text("\""))
                .italic(true)
                .color(Color::Gray)
                .build();

            ConsoleMsg::new(
                ConsoleMsgType::Info,
                format!(
                    "{}{} linked \"{}\": {}",
                    CHAT_PREFIX,
                    &msg.author.name,
                    link_text,
                    embed.url.as_ref().unwrap()
                ),
            )
            .log();

            mc_cmd_sender
                .send(ServerCommand::TellRawAll(tellraw_msg.to_json().unwrap()))
                .await
                .ok();
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
