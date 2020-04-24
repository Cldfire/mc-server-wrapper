use std::env;
use std::path::PathBuf;
use std::{
    collections::HashSet,
    sync::Arc,
    time::{Duration, Instant},
};

use tokio::io::BufReader;
use tokio::prelude::*;
use tokio::runtime::Runtime;
use tokio::{stream::StreamExt, sync::Mutex};

use twilight::model::id::ChannelId;

use mc_server_wrapper_lib::communication::*;
use mc_server_wrapper_lib::parse::*;
use mc_server_wrapper_lib::{McServer, McServerConfig};

use crate::discord::util::{format_online_players, sanitize_for_markdown};
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
        let online_players = Arc::new(Mutex::new(HashSet::new()));

        let discord_channel_id = opt.discord_channel_id.take()
            .unwrap_or_else(||
                env::var("DISCORD_CHANNEL_ID").unwrap_or_else(|_| "".into()).parse().unwrap_or(0)
            );
        let discord_token = opt.discord_token.take()
            .unwrap_or_else(||
                env::var("DISCORD_TOKEN").unwrap_or_else(|_| String::new())
            );

        let discord;
        // Set up discord bridge if enabled
        if opt.bridge_to_discord {
            if discord_channel_id == 0 {
                println!("Discord bridge was enabled but a channel ID to bridge to \
                        was not provided. Either set the environment variable \
                        `DISCORD_CHANNEL_ID` or provide it via the command line \
                        with the `--discord-channel-id` option");
                return;
            }

            if discord_token == "" {
                println!("Discord bridge was enabled but an API token for a Discord \
                        bot was not provided. Either set the environment variable \
                        `DISCORD_TOKEN` or provide it via the command line with the \
                        `--discord-token` option");
                return;
            }

            discord = DiscordBridge::new(discord_token, ChannelId(discord_channel_id))
                .await
                .expect("Could not connect to Discord");
            let cmd_sender_clone = mc_server.cmd_sender.clone();
            let discord_clone = discord.clone();
            let online_players_clone = online_players.clone();
            tokio::spawn(async move {
                let cmd_sender = cmd_sender_clone;
                let discord = discord_clone;
                let online_players = online_players_clone;
                let cmd_parser = DiscordBridge::command_parser();

                // For all received Discord events, map the event to a `ServerCommand`
                // (if necessary) and send it to the Minecraft server
                let mut events = discord.cluster().unwrap().events().await;
                while let Some(e) = events.next().await {
                    let discord = discord.clone();
                    let online_players = online_players.clone();
                    let mut cmd_sender_clone = cmd_sender.clone();
                    let cmd_parser_clone = cmd_parser.clone();

                    tokio::spawn(async move {
                        match discord.handle_discord_event(
                            e,
                            cmd_parser_clone,
                            online_players
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
        } else {
            discord = DiscordBridge::new_noop();
        }

        mc_server.cmd_sender.send(ServerCommand::StartServer).await.unwrap();
        let mut last_start_time = Instant::now();
        let mut our_stdin = BufReader::new(tokio::io::stdin()).lines();

        let progress_bar = ProgressBar::new(100);
        progress_bar.set_style(ProgressStyle::default_bar()
            .template("{bar:30[>20]} {pos:>2}%")
        );

        loop {
            tokio::select! {
                e = mc_server.event_receiver.next() => if let Some(e) = e {
                    match e {
                        ServerEvent::ConsoleEvent(console_msg, Some(specific_msg)) => {
                            let mut print_msg = true;

                            match specific_msg {
                                ConsoleMsgSpecific::PlayerLogout { name } => {
                                    discord.clone().send_channel_msg(format!(
                                        "_**{}** left the game_",
                                        sanitize_for_markdown(&name)
                                    ));

                                    let mut online_players = online_players.lock().await;
                                    online_players.remove(&name);
                                    discord.clone().set_channel_topic(
                                        format_online_players(&online_players, true)
                                    );
                                },
                                ConsoleMsgSpecific::PlayerLogin { name, .. } => {
                                    discord.clone().send_channel_msg(format!(
                                        "_**{}** joined the game_",
                                        sanitize_for_markdown(&name)
                                    ));

                                    let mut online_players = online_players.lock().await;
                                    online_players.insert(name);
                                    discord.clone().set_channel_topic(
                                        format_online_players(&online_players, true)
                                    );
                                },
                                ConsoleMsgSpecific::PlayerMsg { name, msg } => {
                                    discord.clone().send_channel_msg(format!(
                                        "**{}** {}",
                                        sanitize_for_markdown(name),
                                        msg
                                    ));
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

        discord.clone().set_channel_topic("Minecraft server is offline");
    });

    // TODO: we need this because the way tokio handles stdin involves blocking
    rt.shutdown_timeout(Duration::from_millis(5));
    Ok(())
}
