use log::{debug, info, warn};

use twilight::{
    cache::{
        twilight_cache_inmemory::{
            config::{EventType, InMemoryConfigBuilder},
            model::CachedMember,
        },
        InMemoryCache,
    },
    command_parser::{Command, CommandParserConfig, Parser},
    gateway::{Cluster, ClusterConfig, Event},
    http::Client as DiscordClient,
    model::{
        channel::{message::MessageType, Message},
        gateway::{payload::RequestGuildMembers, GatewayIntents},
        id::{ChannelId, GuildId, UserId},
    },
};

use mc_server_wrapper_lib::{communication::*, parse::*};
use minecraft_protocol::chat::{Color, Payload};

use util::{channel_name, format_mentions_in, format_online_players, tellraw_prefix};

use crate::ONLINE_PLAYERS;
use futures::future;
use std::{collections::HashMap, sync::Arc};
use tokio::{stream::StreamExt, sync::mpsc::Sender};

pub mod util;

static CHAT_PREFIX: &str = "[D] ";

/// Sets up a `DiscordBridge` and starts handling events
pub async fn setup_discord(
    token: String,
    bridge_channel_id: ChannelId,
    mc_cmd_sender: Sender<ServerCommand>,
) -> anyhow::Result<DiscordBridge> {
    info!("Setting up Discord");
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

            if let Err(e) = discord.inner.as_ref().unwrap().cache.update(&e.1).await {
                warn!("Failed to update Discord cache: {}", e);
            }

            tokio::spawn(async move {
                if let Err(e) = discord
                    .handle_discord_event(e, cmd_parser_clone, cmd_sender_clone)
                    .await
                {
                    warn!("Failed to handle Discord event: {}", e);
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
    inner: Option<DiscordBridgeInner>,
    /// The ID of the channel we're bridging to
    bridge_channel_id: ChannelId,
}

// Groups together optionally-present things
//
// Everything in here can be trivially cloned
#[derive(Debug, Clone)]
struct DiscordBridgeInner {
    client: DiscordClient,
    cluster: Cluster,
    cache: InMemoryCache,
}

impl DiscordBridge {
    /// Connects to Discord with the given `token` and `bridge_channel_id`
    pub async fn new(token: String, bridge_channel_id: ChannelId) -> anyhow::Result<Self> {
        let client = DiscordClient::new(&token);
        let cluster_config = ClusterConfig::builder(&token)
            .intents(Some(
                GatewayIntents::GUILDS
                    | GatewayIntents::GUILD_MESSAGES
                    | GatewayIntents::GUILD_MEMBERS,
            ))
            .build();
        let cluster = Cluster::new(cluster_config).await?;

        let cluster_spawn = cluster.clone();
        tokio::spawn(async move {
            cluster_spawn.up().await;
        });

        let cache_config = InMemoryConfigBuilder::new()
            .event_types(
                EventType::GUILD_CREATE
                    | EventType::GUILD_UPDATE
                    | EventType::GUILD_DELETE
                    | EventType::CHANNEL_CREATE
                    | EventType::CHANNEL_UPDATE
                    | EventType::CHANNEL_DELETE
                    | EventType::MEMBER_ADD
                    | EventType::MEMBER_CHUNK
                    | EventType::MEMBER_REMOVE
                    | EventType::MEMBER_UPDATE,
            )
            .build();
        let cache = InMemoryCache::from(cache_config);

        Ok(Self {
            inner: Some(DiscordBridgeInner {
                client,
                cluster,
                cache,
            }),
            bridge_channel_id,
        })
    }

    /// Constructs an instance of this struct that does nothing
    pub fn new_noop() -> Self {
        Self {
            inner: None,
            bridge_channel_id: ChannelId(0),
        }
    }

    /// Provides access to the `Cluster` inside this struct
    pub fn cluster(&self) -> Option<Cluster> {
        self.inner.as_ref().map(|i| i.cluster.clone())
    }

    /// Provides access to the `Client` inside this struct
    pub fn client(&self) -> Option<DiscordClient> {
        self.inner.as_ref().map(|i| i.client.clone())
    }

    /// Provides access to the `InMemoryCache` inside this struct
    pub fn cache(&self) -> Option<InMemoryCache> {
        self.inner.as_ref().map(|i| i.cache.clone())
    }

    /// Constructs a command parser for Discord commands
    pub fn command_parser<'a>() -> Parser<'a> {
        let mut config = CommandParserConfig::new();

        config.command("list").add();

        // TODO: make this configurable
        config.add_prefix("!mc ");

        Parser::new(config)
    }

    /// Get and cache the member specified by the given IDs
    ///
    /// The cached member will be returned so you can make use of the data right
    /// away.
    pub async fn get_and_cache_guild_member(
        &self,
        guild_id: GuildId,
        user_id: UserId,
    ) -> Option<Arc<CachedMember>> {
        let maybe_member = match self.client().unwrap().guild_member(guild_id, user_id).await {
            Ok(maybe_member) => maybe_member,
            Err(e) => {
                log::warn!(
                    "Failed to get guild member from API for guild_id {} and user_id {}: {}",
                    guild_id,
                    user_id,
                    e
                );
                None
            }
        };

        if let Some(member) = maybe_member {
            Some(self.cache().unwrap().cache_member(guild_id, member).await)
        } else {
            None
        }
    }

    /// Get cached info for the guild member specified by the given IDs
    ///
    /// `None` will be returned if something went wrong.
    ///
    /// This method first checks the cache for the member and, if the member
    /// isn't present, then performs an API request for the member's info,
    /// caching it upon success for future runs.
    pub async fn obtain_guild_member(
        &self,
        guild_id: GuildId,
        user_id: UserId,
    ) -> Option<Arc<CachedMember>> {
        // First check the cache
        let mut maybe_cached_member = match self.cache().unwrap().member(guild_id, user_id).await {
            Ok(maybe_cached_member) => maybe_cached_member,
            Err(e) => {
                log::warn!(
                    "Failed to get guild member from cache for guild_id {} and user_id {}: {}",
                    guild_id,
                    user_id,
                    e
                );
                None
            }
        };

        // The member wasn't cached; see if we can grab and cache their data with an API
        // request
        if maybe_cached_member.is_none() {
            maybe_cached_member = self.get_and_cache_guild_member(guild_id, user_id).await;
        }

        maybe_cached_member
    }

    /// Handle an event from Discord
    ///
    /// The provided `cmd_parser` is used to parse commands (not
    /// `ServerCommands`) from Discord messages.
    pub async fn handle_discord_event<'a>(
        &self,
        event: (u64, Event),
        cmd_parser: Parser<'a>,
        mc_cmd_sender: Sender<ServerCommand>,
    ) -> anyhow::Result<()> {
        match event {
            (_, Event::Ready(_)) => {
                info!("Discord bridge online");
            }
            (shard_id, Event::GuildCreate(guild)) => {
                // Log the name of the channel we're bridging to as well if it's
                // in this guild
                if let Some(channel_name) = guild
                    .channels
                    .get(&self.bridge_channel_id)
                    .and_then(|c| Some(channel_name(c)))
                {
                    info!(
                        "Connected to guild {}, bridging chat to '#{}'",
                        guild.name, channel_name
                    );

                    // This is the guild containing the channel we're bridging to. We want to
                    // initially cache all of the members in the guild so that we can later use
                    // the cached info to display nicknames when outputting Discord messages in
                    // Minecraft
                    // TODO: if bigger servers start using this it might be undesirable to cache
                    // all member info right out of the gate
                    self.cluster()
                        .unwrap()
                        .command(shard_id, &RequestGuildMembers::new_all(guild.id, None))
                        .await?;
                } else {
                    info!("Connected to guild {}", guild.name);
                }
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
        }

        Ok(())
    }

    /// Handles any attachments in the given message
    // TODO: doesn't handle nicknames
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
    ///
    /// This can only be called if `self.inner` is `Some`
    async fn handle_msg_content(&self, msg: &Message, mut mc_cmd_sender: Sender<ServerCommand>) {
        if msg.content.is_empty() {
            debug!("Empty message from Discord: {:?}", &msg);
            return;
        }

        let cache = self.cache().unwrap();
        let guild_id = msg.guild_id.unwrap_or(GuildId(0));

        // Get info about mentioned members from the cache and / or API if available
        let cached_mentioned_members = future::join_all(
            msg.mentions
                .keys()
                .map(|id| async move { self.obtain_guild_member(guild_id, *id).await }),
        )
        .await;

        // Use the cached info to format mentions with the member's nickname if one is
        // set
        let mut mentions_map = HashMap::new();
        for (mention, cmm) in msg.mentions.iter().zip(cached_mentioned_members.iter()) {
            mentions_map.insert(
                *mention.0,
                cmm.as_ref()
                    .and_then(|cm| cm.nick.as_deref())
                    .unwrap_or(mention.1.name.as_str()),
            );
        }

        let content = format_mentions_in(
            msg.content.clone(),
            mentions_map,
            &msg.mention_roles,
            cache.clone(),
        )
        .await;

        // Similar process to above, getting cached info for the message author so we
        // can use their nickname if set
        let cached_member = self.obtain_guild_member(guild_id, msg.author.id).await;

        let author_name = cached_member
            .as_ref()
            .and_then(|cm| cm.nick.as_ref())
            .unwrap_or(&msg.author.name);

        let tellraw_msg = tellraw_prefix()
            .then(Payload::text(&format!("<{}> {}", author_name, &content)))
            .build();

        // Tellraw commands do not get logged to the console, so we
        // make up for that here
        ConsoleMsg::new(
            ConsoleMsgType::Info,
            format!("{}<{}> {}", CHAT_PREFIX, author_name, &content),
        )
        .log();

        mc_cmd_sender
            .send(ServerCommand::TellRawAll(tellraw_msg.to_json().unwrap()))
            .await
            .ok();
    }

    /// Handles any embeds in the given message
    // TODO: doesn't handle nicknames
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
    /// A new task is spawned to send the message, and its `JoinHandle` is
    /// returned so its completion can be `await`ed if desired.
    pub fn send_channel_msg<T: Into<String> + Send + 'static>(
        self,
        text: T,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            if let Some(inner) = self.inner {
                let content_res = inner
                    .client
                    .create_message(self.bridge_channel_id)
                    .content(text);

                match content_res {
                    Ok(cm) => {
                        if let Err(e) = cm.await {
                            warn!("Failed to send Discord message: {}", e);
                        }
                    }
                    Err(validation_err) => {
                        warn!(
                            "Attempted to send invalid message to Discord: {}",
                            validation_err
                        );
                        // TODO: log message content that failed to validate
                        // when twilight returns ownership of it
                    }
                }
            }
        })
    }

    /// Sets the topic of the channel being bridged to to `text`
    ///
    /// A new task is spawned to send the message, and its `JoinHandle` is
    /// returned so its completion can be `await`ed if desired.
    // TODO: need to set channel topic way less frequently
    pub fn set_channel_topic<T: Into<String> + Send + 'static>(
        self,
        text: T,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            if let Some(inner) = self.inner {
                let content_res = inner
                    .client
                    .update_channel(self.bridge_channel_id)
                    .topic(text);

                match content_res {
                    Ok(cm) => {
                        if let Err(e) = cm.await {
                            warn!("Failed to set Discord channel topic: {}", e);
                        }
                    }
                    Err(validation_err) => {
                        warn!(
                            "Attempted to set Discord channel topic to invalid content: {}",
                            validation_err
                        );
                        // TODO: should also log here as described above
                    }
                }
            }
        })
    }
}
