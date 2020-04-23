use std::collections::HashSet;

use twilight::{
    gateway::shard::Event, http::Client as DiscordClient, model::channel::message::MessageType,
    model::id::ChannelId,
};

use mc_server_wrapper_lib::communication::*;
use mc_server_wrapper_lib::parse::*;
use minecraft_chat::{Color, MessageBuilder, Payload};

use crate::error::*;

pub async fn handle_discord_event(
    event: (u64, Event),
    _discord_client: DiscordClient,
    bridge_channel_id: ChannelId,
) -> Result<Option<ServerCommand>, Error> {
    match event {
        (_, Event::Ready(_)) => {
            println!("Discord bridge online");
            Ok(None)
        }
        (_, Event::GuildCreate(guild)) => {
            println!("Connected to guild {}", guild.name);
            Ok(None)
        }
        (_, Event::MessageCreate(msg)) => {
            // TODO: maybe some bot chatter should be allowed through?
            // TODO: error handling
            if msg.kind == MessageType::Regular
                && !msg.author.bot
                && msg.channel_id == bridge_channel_id
            {
                let tellraw_msg = MessageBuilder::builder(Payload::text("[D] "))
                    .bold(true)
                    .color(Color::LightPurple)
                    .then(Payload::text(
                        &("<".to_string() + &msg.author.name + "> " + &msg.content),
                    ))
                    .bold(false)
                    .color(Color::White)
                    .build();

                // TODO: This will not add the message to the server logs
                println!(
                    "{}",
                    ConsoleMsg::new(
                        ConsoleMsgType::Info,
                        "[D] <".to_string() + &msg.author.name + "> " + &msg.content
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

/// Sends the given text to the given Discord channel if the given client is `Some`.
pub async fn send_channel_msg(
    discord_client: Option<DiscordClient>,
    channel_id: ChannelId,
    text: String,
) -> Result<(), Error> {
    if let Some(discord_client) = discord_client {
        discord_client
            .create_message(channel_id)
            .content(text)
            .await?;
    }

    Ok(())
}

/// Sets the topic of the given channel to `text` if the given client is `Some`.
// TODO: currently does not work, see https://github.com/twilight-rs/twilight/issues/149
pub async fn set_channel_topic(
    discord_client: Option<DiscordClient>,
    channel_id: ChannelId,
    text: String,
) -> Result<(), Error> {
    if let Some(discord_client) = discord_client {
        discord_client
            .update_channel(channel_id)
            .topic(text)
            .await?;
    }

    Ok(())
}

/// Utility function to return a neatly formatted string of online players for
/// a channel topic.
pub fn format_online_players_topic(online_players: &HashSet<String>) -> String {
    // Sort the players for stable name order
    let mut online_players_vec: Vec<_> = online_players.into_iter().collect();
    online_players_vec.sort();

    match online_players.len() {
        0 => "Nobody is playing Minecraft".into(),
        1 => format!("{} is playing Minecraft", online_players_vec[0]),
        2 => format!(
            "{} and {} are playing Minecraft",
            online_players_vec[0], online_players_vec[1]
        ),
        n => {
            let mut string = format!(
                "{}, {}, and {}",
                online_players_vec[0], online_players_vec[1], online_players_vec[2]
            );

            if n > 3 {
                string.push_str(&format!(" (+ {} more)", n - 3));
            }

            string.push_str(" are playing Minecraft");
            string
        }
    }
}

#[cfg(test)]
mod test {
    mod format_online_players_topic {
        use crate::discord::format_online_players_topic;
        use std::collections::HashSet;

        #[test]
        fn no_players() {
            let online_players = HashSet::new();
            let formatted = format_online_players_topic(&online_players);

            assert_eq!(&formatted, "Nobody is playing Minecraft");
        }

        #[test]
        fn one_player() {
            let mut online_players = HashSet::new();
            online_players.insert("p1".into());
            let formatted = format_online_players_topic(&online_players);

            assert_eq!(&formatted, "p1 is playing Minecraft");
        }

        #[test]
        fn two_players() {
            let mut online_players = HashSet::new();
            online_players.insert("p1".into());
            online_players.insert("p2".into());
            let formatted = format_online_players_topic(&online_players);

            assert_eq!(&formatted, "p1 and p2 are playing Minecraft");
        }

        #[test]
        fn three_players() {
            let mut online_players = HashSet::new();
            online_players.insert("p1".into());
            online_players.insert("p2".into());
            online_players.insert("p3".into());
            let formatted = format_online_players_topic(&online_players);

            assert_eq!(&formatted, "p1, p2, and p3 are playing Minecraft");
        }

        #[test]
        fn four_players() {
            let mut online_players = HashSet::new();
            online_players.insert("p1".into());
            online_players.insert("p2".into());
            online_players.insert("p3".into());
            online_players.insert("p4".into());
            let formatted = format_online_players_topic(&online_players);

            assert_eq!(
                &formatted,
                "p1, p2, and p3 (+ 1 more) are playing Minecraft"
            );
        }

        #[test]
        fn seven_players() {
            let mut online_players = HashSet::new();
            online_players.insert("p1".into());
            online_players.insert("p2".into());
            online_players.insert("p3".into());
            online_players.insert("p4".into());
            online_players.insert("p5".into());
            online_players.insert("p6".into());
            online_players.insert("p7".into());
            let formatted = format_online_players_topic(&online_players);

            assert_eq!(
                &formatted,
                "p1, p2, and p3 (+ 4 more) are playing Minecraft"
            );
        }
    }
}
