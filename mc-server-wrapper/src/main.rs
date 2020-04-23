use std::env;
use std::path::PathBuf;
use std::{
    collections::HashSet,
    time::{Duration, Instant},
};

use tokio::io::BufReader;
use tokio::prelude::*;
use tokio::runtime::Runtime;
use tokio::stream::StreamExt;

use twilight::{
    gateway::{Cluster, ClusterConfig},
    http::Client as DiscordClient,
    model::gateway::GatewayIntents,
    model::id::ChannelId,
};

use mc_server_wrapper_lib::communication::*;
use mc_server_wrapper_lib::parse::*;
use mc_server_wrapper_lib::{McServer, McServerConfig};

use crate::discord::*;
use crate::error::*;
use dotenv::dotenv;
use indicatif::{ProgressBar, ProgressStyle};
use structopt::StructOpt;

mod discord;
mod error;

#[derive(StructOpt, Debug)]
#[structopt(name = "mc-wrapper")]
pub struct Opt {
    /// Path to the server jar to execute
    #[structopt(parse(from_os_str))]
    server_path: PathBuf,

    /// Discord bot token (for Discord chat bridge)
    // Note that we cannot set this flag using structopt's env feature because
    // it prints the value if it's set when you view help output, and we
    // of course don't want that for private tokens
    #[structopt(long)]
    discord_token: Option<String>,

    /// Discord channel ID (for Discord chat bridge)
    #[structopt(long)]
    discord_channel_id: Option<u64>,

    /// Bridge server chat to discord
    #[structopt(short, long)]
    bridge_to_discord: bool,

    /// Amount of memory in megabytes to allocate for the server
    #[structopt(short = "m", long = "memory", default_value = "1024")]
    memory: u16,
}

fn main() -> Result<(), Error> {
    let mut rt = Runtime::new().unwrap();

    // TODO: use proc macro instead if shutdown_timeout no longer needed
    rt.block_on(async {
        dotenv().ok();
        let mut opt = Opt::from_args();

        let mc_config = McServerConfig {
            server_path: opt.server_path.clone(),
            memory: opt.memory
        };
        let mut mc_server = McServer::new(mc_config);

        let discord_channel_id = opt.discord_channel_id.take()
            .unwrap_or_else(||
                env::var("DISCORD_CHANNEL_ID").unwrap_or("".into()).parse().unwrap_or(0)
            );
        let discord_token = opt.discord_token.take()
            .unwrap_or_else(||
                env::var("DISCORD_TOKEN").unwrap_or_else(|_| String::new())
            );

        let discord_client;
        // Set up discord bridge if enabled
        if opt.bridge_to_discord {
            if discord_channel_id == 0 {
                println!("Discord bridge was enabled but a channel ID to bridge to \
                        was not provided. Either set the environment variable \
                        `DISCORD_CHANNEL_ID` or provide it via the command line \
                        with the `--discord-channel-id` option");
                return ();
            }

            if discord_token == "" {
                println!("Discord bridge was enabled but an API token for a Discord \
                        bot was not provided. Either set the environment variable \
                        `DISCORD_TOKEN` or provide it via the command line with the \
                        `--discord-token` option");
                return ();
            }

            let discord_client_temp = DiscordClient::new(&discord_token);
            let cluster_config = ClusterConfig::builder(&discord_token)
                // We only care about guild message events
                .intents(Some(GatewayIntents::GUILD_MESSAGES))
                .build();
            let discord_cluster = Cluster::new(cluster_config);
            discord_cluster.up().await.expect("Could not connect to Discord");
            let cmd_sender_clone = mc_server.cmd_sender.clone();

            let discord_client_clone = discord_client_temp.clone();
            let discord_cluster_clone = discord_cluster.clone();
            tokio::spawn(async move {
                let discord_client = discord_client_clone;
                let discord_cluster = discord_cluster_clone;
                let cmd_sender = cmd_sender_clone;

                // For all received Discord events, map the event to a `ServerCommand`
                // (if necessary) and send it to the Minecraft server
                let mut events = discord_cluster.events().await;
                while let Some(e) = events.next().await {
                    let client_clone = discord_client.clone();
                    let mut cmd_sender_clone = cmd_sender.clone();

                    tokio::spawn(async move {
                        match handle_discord_event(
                            e,
                            client_clone,
                            ChannelId(discord_channel_id)
                        ).await {
                            Ok(Some(cmd)) => {
                                let _ = cmd_sender_clone.send(cmd).await;
                            },
                            // TODO: error handling
                            _ => {}
                        }
                    });
                }
            });

            discord_client = Some(discord_client_temp);
        } else {
            discord_client = None;
        }

        mc_server.cmd_sender.send(ServerCommand::StartServer).await.unwrap();
        let mut last_start_time = Instant::now();
        let mut our_stdin = BufReader::new(tokio::io::stdin()).lines();

        let progress_bar = ProgressBar::new(100);
        progress_bar.set_style(ProgressStyle::default_bar()
            .template("{bar:30[>20]} {pos:>2}%")
        );

        let mut online_players = HashSet::new();

        loop {
            tokio::select! {
                e = mc_server.event_receiver.next() => if let Some(e) = e {
                    match e {
                        ServerEvent::ConsoleEvent(console_msg, Some(specific_msg)) => {
                            let mut print_msg = true;

                            match specific_msg {
                                ConsoleMsgSpecific::PlayerLogout { name } => {
                                    let _ = tokio::spawn(send_channel_msg(
                                        discord_client.clone(),
                                        ChannelId(discord_channel_id),
                                        "_**".to_string() + &name + "** left the game_"
                                    )).await;

                                    online_players.remove(&name);
                                    let _ = tokio::spawn(set_channel_topic(
                                        discord_client.clone(),
                                        ChannelId(discord_channel_id),
                                        format_online_players_topic(&online_players)
                                    )).await;
                                },
                                ConsoleMsgSpecific::PlayerLogin { name, .. } => {
                                    // TODO: error handling strategy?
                                    let _ = tokio::spawn(send_channel_msg(
                                        discord_client.clone(),
                                        ChannelId(discord_channel_id),
                                        "_**".to_string() + &name + "** joined the game_"
                                    )).await;

                                    online_players.insert(name);
                                    let _ = tokio::spawn(set_channel_topic(
                                        discord_client.clone(),
                                        ChannelId(discord_channel_id),
                                        format_online_players_topic(&online_players)
                                    )).await;
                                },
                                ConsoleMsgSpecific::PlayerMsg { name, msg } => {
                                    let _ = tokio::spawn(send_channel_msg(
                                        discord_client.clone(),
                                        ChannelId(discord_channel_id),
                                        "**".to_string() + &name + "**  " + &msg
                                    )).await;
                                },
                                ConsoleMsgSpecific::SpawnPrepareProgress { progress, .. } => {
                                    progress_bar.set_position(progress as u64);
                                    print_msg = false;
                                },
                                ConsoleMsgSpecific::SpawnPrepareFinish { time_elapsed_ms, .. } => {
                                    progress_bar.finish_and_clear();
                                    print_msg = false;
                                    println!("  (finished in {} ms)", time_elapsed_ms);
                                },
                                _ => {}
                            }

                            if print_msg {
                                println!("{}", console_msg);
                            }
                        },
                        ServerEvent::ConsoleEvent(console_msg, None) => {
                            println!("{}", console_msg);
                        },
                        ServerEvent::StdoutLine(line) => {
                            println!("{}", line);
                        },
                        ServerEvent::StderrLine(line) => {
                            println!("{}", line);
                        },

                        ServerEvent::ServerStopped(exit_status, reason) => if let Some(reason) = reason {
                            match reason {
                                ShutdownReason::EulaNotAccepted => {
                                    println!("Agreeing to EULA!");
                                    mc_server.cmd_sender.send(ServerCommand::AgreeToEula).await.unwrap();
                                }
                            }
                        } else if exit_status.success() {
                            // TODO: we eventually need to not stop the server forever here
                            //
                            // have a `ShutdownReason` along the lines of "you told me to stop"
                            mc_server.cmd_sender.send(ServerCommand::StopServer { forever: true }).await.unwrap();
                        } else {
                            // There are circumstances where the status will be failure
                            // and attempting to restart the server will always fail. We
                            // attempt to catch these cases by not restarting if the
                            // server crashed twice within a small time window
                            if last_start_time.elapsed().as_secs() < 60 {
                                println!("Fatal error believed to have been encountered, not \
                                    restarting server");
                                mc_server.cmd_sender.send(ServerCommand::StopServer { forever: true }).await.unwrap();
                            } else {
                                println!("Restarting server...");
                                mc_server.cmd_sender.send(ServerCommand::StartServer).await.unwrap();
                                last_start_time = Instant::now();
                                // TODO: tell discord that the mc server crashed
                            }
                        },

                        ServerEvent::AgreeToEulaResult(res) => {
                            if let Err(e) = res {
                                println!("Failed to agree to EULA: {:?}", e);
                                mc_server.cmd_sender.send(ServerCommand::StopServer { forever: true }).await.unwrap();
                            } else {
                                mc_server.cmd_sender.send(ServerCommand::StartServer).await.unwrap();
                                last_start_time = Instant::now();
                            }
                        }
                    }
                } else {
                    // `McServer` instance was shut down forever. Exit program
                    break;
                },
                Some(line) = our_stdin.next() => {
                    if let Ok(line) = line {
                        mc_server.cmd_sender.send(ServerCommand::WriteCommandToStdin(line)).await.unwrap();
                    } else {
                        break;
                    }
                },
                else => break,
            }
        }

        let _ = set_channel_topic(
            discord_client.clone(),
            ChannelId(discord_channel_id),
            "Minecraft server is offline".into()
        ).await;
    });

    // TODO: we need this because the way tokio handles stdin involves blocking
    rt.shutdown_timeout(Duration::from_millis(5));
    Ok(())
}
