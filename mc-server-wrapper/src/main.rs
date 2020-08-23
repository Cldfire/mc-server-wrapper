use std::{collections::HashSet, path::PathBuf, time::Instant};

use anyhow::{anyhow, Context};

use tokio::{
    stream::StreamExt,
    sync::{mpsc, Mutex},
};

use once_cell::sync::OnceCell;
use scopeguard::defer;

use twilight::model::id::ChannelId;

use mc_server_wrapper_lib::{
    communication::*, parse::*, McServerConfig, McServerManager, CONSOLE_MSG_LOG_TARGET,
};

use dotenv::dotenv;
use log::*;

use crate::discord::{util::sanitize_for_markdown, *};

use crate::ui::TuiState;

use crossterm::{
    event::{Event, EventStream, KeyCode},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use structopt::{clap::AppSettings, StructOpt};
use tui::{backend::CrosstermBackend, Terminal};
use util::{format_online_players, OnlinePlayerFormat};

mod discord;
mod logging;
mod ui;

/// Maintains a hashset of players currently on the Minecraft server
static ONLINE_PLAYERS: OnceCell<Mutex<HashSet<String>>> = OnceCell::new();

#[derive(StructOpt, Debug)]
#[structopt(settings(&[AppSettings::ColoredHelp]))]
pub struct Opt {
    /// Path to the Minecraft server jar
    #[structopt(parse(from_os_str))]
    server_path: PathBuf,

    /// Discord bot token (for Discord chat bridge)
    #[structopt(long, env, hide_env_values = true)]
    discord_token: Option<String>,

    /// Discord channel ID (for Discord chat bridge)
    #[structopt(long, env)]
    discord_channel_id: Option<u64>,

    /// Bridge server chat to discord
    #[structopt(short = "b", long)]
    bridge_to_discord: bool,

    /// Amount of memory in megabytes to allocate for the server
    #[structopt(short = "m", long, default_value = "1024")]
    memory: u16,

    /// Logging level for mc-server-wrapper dependencies [error, warn, info,
    /// debug, trace]
    ///
    /// This only affects file logging.
    #[structopt(long, env, default_value = "warn")]
    log_level_all: log::Level,

    /// Logging level for mc-server-wrapper [error, warn, info, debug, trace]
    ///
    /// This overrides --log-level-all and only affects file logging.
    #[structopt(long, env, default_value = "debug")]
    log_level_self: log::Level,

    /// Logging level for Discord-related dependencies [error, warn, info,
    /// debug, trace]
    ///
    /// This overrides --log-level-all and only affects file logging.
    #[structopt(long, env, default_value = "info")]
    log_level_discord: log::Level,

    /// Custom flags to pass to the JVM
    #[structopt(env, raw(true))]
    jvm_flags: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    log_panics::init();
    dotenv().ok();
    CONSOLE_MSG_LOG_TARGET.set("mc").unwrap();
    ONLINE_PLAYERS.set(Mutex::new(HashSet::new())).unwrap();

    let opt = Opt::from_args();
    let (log_sender, mut log_receiver) = mpsc::channel(64);
    let stdout = std::io::stdout();
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let mut tui_state = TuiState::new();

    enable_raw_mode()?;
    terminal.backend_mut().execute(EnterAlternateScreen)?;
    defer! {
        std::io::stdout().execute(LeaveAlternateScreen).unwrap();
        disable_raw_mode().unwrap();
    }

    logging::setup_logger(
        opt.server_path.with_file_name("mc-server-wrapper.log"),
        log_sender,
        opt.log_level_all,
        opt.log_level_self,
        opt.log_level_discord,
    )
    .with_context(|| "Failed to set up logging")?;

    let mc_config = McServerConfig::new(opt.server_path.clone(), opt.memory, opt.jvm_flags, false);
    let (mc_server, mut mc_cmd_sender, mut mc_event_receiver) = McServerManager::new();

    info!("Starting the Minecraft server");
    mc_cmd_sender
        .send(ServerCommand::StartServer {
            config: Some(mc_config),
        })
        .await
        .unwrap();
    let mut last_start_time = Instant::now();

    // TODO: start drawing UI before setting up discord
    let discord = if opt.bridge_to_discord {
        if opt.discord_channel_id.is_none() {
            return Err(anyhow!(
                "Discord bridge was enabled but a channel ID to bridge to was not provided"
            ));
        }

        if opt.discord_token.is_none() {
            return Err(anyhow!(
                "Discord bridge was enabled but an API token for a Discord bot was not provided"
            ));
        }

        setup_discord(
            opt.discord_token.unwrap(),
            ChannelId(opt.discord_channel_id.unwrap()),
            mc_cmd_sender.clone(),
            // Allows for disabling status updates while developing to avoid messing
            // with a prod bot account
            // TODO: move to config
            match std::env::var("NO_UPDATE_STATUS") {
                Ok(s) if s == "1" => false,
                _ => true,
            },
        )
        .await
        .with_context(|| "Failed to connect to Discord")?
    } else {
        DiscordBridge::new_noop()
    };

    let mut term_events = EventStream::new();

    // This loop handles both user input and events from the Minecraft server
    loop {
        // Make sure we are up-to-date on logs before drawing the UI
        while let Ok(record) = log_receiver.try_recv() {
            tui_state.logs_state.add_record(record);
        }

        // TODO: figure out what to do if the terminal fails to draw
        let _ = terminal.draw(|mut f| tui_state.draw(&mut f));

        tokio::select! {
            e = mc_event_receiver.next() => if let Some(e) = e {
                match e {
                    ServerEvent::ConsoleEvent(console_msg, Some(specific_msg)) => {
                        if let ConsoleMsgType::Unknown(ref s) = console_msg.msg_type {
                            warn!("Encountered unknown message type from Minecraft: {}", s);
                        }

                        let mut should_log = true;

                        match specific_msg {
                            ConsoleMsgSpecific::PlayerLogout { name } => {
                                discord.clone().send_channel_msg(format!(
                                    "_**{}** left the game_",
                                    sanitize_for_markdown(&name)
                                ));

                                let mut online_players = ONLINE_PLAYERS.get().unwrap().lock().await;
                                online_players.remove(&name);
                                discord.clone().update_status(format_online_players(
                                    &online_players,
                                    OnlinePlayerFormat::BotStatus
                                ));
                            },
                            ConsoleMsgSpecific::PlayerLogin { name, .. } => {
                                discord.clone().send_channel_msg(format!(
                                    "_**{}** joined the game_",
                                    sanitize_for_markdown(&name)
                                ));

                                let mut online_players = ONLINE_PLAYERS.get().unwrap().lock().await;
                                online_players.insert(name);
                                discord.clone().update_status(format_online_players(
                                    &online_players,
                                    OnlinePlayerFormat::BotStatus
                                ));
                            },
                            ConsoleMsgSpecific::PlayerMsg { name, msg } => {
                                discord.clone().send_channel_msg(format!(
                                    "**{}** {}",
                                    sanitize_for_markdown(name),
                                    msg
                                ));
                            },
                            ConsoleMsgSpecific::SpawnPrepareProgress { progress } => {
                                tui_state.logs_state.set_progress_percent(progress as u32);
                                should_log = false;
                            },
                            ConsoleMsgSpecific::SpawnPrepareFinish { .. } => {
                                tui_state.logs_state.set_progress_percent(100);
                            },
                            ConsoleMsgSpecific::FinishedLoading { .. } => {
                                let online_players = ONLINE_PLAYERS.get().unwrap().lock().await;
                                discord.clone().update_status(format_online_players(
                                    &online_players,
                                    OnlinePlayerFormat::BotStatus
                                ));
                            },
                            _ => {}
                        }

                        if should_log {
                            console_msg.log();
                        }
                    },
                    ServerEvent::ConsoleEvent(console_msg, None) => {
                        console_msg.log();
                    },
                    ServerEvent::StdoutLine(line) => {
                        info!(target: CONSOLE_MSG_LOG_TARGET.get().unwrap(), "{}", line);
                    },
                    ServerEvent::StderrLine(line) => {
                        warn!(target: CONSOLE_MSG_LOG_TARGET.get().unwrap(), "{}", line);
                    },

                    ServerEvent::ServerStopped(process_result, reason) => {
                        if let Some(ShutdownReason::EulaNotAccepted) = reason {
                            info!("Agreeing to EULA!");
                            mc_cmd_sender.send(ServerCommand::AgreeToEula).await.unwrap();
                        } else {
                            let mut sent_restart_command = false;

                            // How we handle this depends on whether or not we asked the server to stop
                            if let Some(ShutdownReason::RequestedToStop) = reason {
                                match process_result {
                                    Ok(exit_status) => if exit_status.success() {
                                        info!("Minecraft server process exited successfully");
                                    } else {
                                        warn!("Minecraft server process exited non-successfully with code {}", &exit_status);
                                    },
                                    Err(e) => {
                                        // TODO: print exit status here as well?
                                        error!("Minecraft server process exited non-successfully with error {}", e);
                                    }
                                }
                            } else {
                                // We did not ask the server to stop
                                match process_result {
                                    Ok(exit_status) => {
                                        warn!("Minecraft server process exited with code {}", &exit_status);
                                        discord.clone().send_channel_msg("The Minecraft server crashed!");

                                        // Attempt to restart the server if it's been up for at least 5 minutes
                                        // TODO: make this configurable
                                        // TODO: maybe parse logs for things that definitely indicate a crash?
                                        if last_start_time.elapsed().as_secs() > 300 {
                                            mc_cmd_sender.send(ServerCommand::StartServer { config: None }).await.unwrap();

                                            last_start_time = Instant::now();
                                            sent_restart_command = true;
                                        } else {
                                            error!("Fatal error believed to have been encountered, not restarting server");
                                        }
                                    },
                                    Err(e) => {
                                        error!("Minecraft server process exited with error: {}, not restarting server", e);
                                    }
                                }
                            }

                            if sent_restart_command {
                                discord.clone().send_channel_msg("Restarting the Minecraft server...");
                                discord.clone().update_status("server is restarting");
                                info!("Restarting server...");
                            } else {
                                discord.clone().update_status("server is offline");
                                info!("Start the Minecraft server back up with `start` or shutdown the wrapper with `stop`");
                            }
                        }
                    },

                    ServerEvent::AgreeToEulaResult(res) => {
                        if let Err(e) = res {
                            error!("Failed to agree to EULA: {:?}", e);
                            mc_cmd_sender.send(ServerCommand::StopServer { forever: true }).await.unwrap();
                        } else {
                            mc_cmd_sender.send(ServerCommand::StartServer { config: None }).await.unwrap();
                            last_start_time = Instant::now();
                        }
                    }
                    ServerEvent::StartServerResult(res) => {
                        // TODO: it's impossible to read start failures right now because the TUI
                        // leaves the alternate screen right away and the logs are gone
                        if let Err(e) = res {
                            error!("Failed to start the Minecraft server: {}", e);
                            mc_cmd_sender.send(ServerCommand::StopServer { forever: true }).await.unwrap();
                        }
                    }
                }
            } else {
                break;
            },
            Some(record) = log_receiver.next() => {
                tui_state.logs_state.add_record(record);
            },
            maybe_term_event = term_events.next() => {
                match maybe_term_event {
                    Some(Ok(event)) => {
                        if let Event::Key(key_event) = event {
                            match key_event.code {
                                KeyCode::Enter => {
                                    if mc_server.running().await {
                                        mc_cmd_sender.send(ServerCommand::WriteCommandToStdin(tui_state.input_state.value().to_string())).await.unwrap();
                                    } else {
                                        // TODO: create a command parser for user input?
                                        // https://docs.rs/clap/2.33.1/clap/struct.App.html#method.get_matches_from_safe
                                        match tui_state.input_state.value() {
                                            "start" => {
                                                info!("Starting the Minecraft server");
                                                mc_cmd_sender.send(ServerCommand::StartServer { config: None }).await.unwrap();
                                                last_start_time = Instant::now();
                                            },
                                            "stop" => {
                                                mc_cmd_sender.send(ServerCommand::StopServer { forever: true }).await.unwrap();
                                            },
                                            _ => {}
                                        }
                                    }

                                    tui_state.input_state.clear();
                                },
                                _ => {}
                            }
                        }

                        tui_state.handle_input(event);
                    },
                    Some(Err(e)) => {
                        error!("TUI input error: {}", e);
                    },
                    None => {
                        // TODO: need to make sure that after this is reached it isn't reached again
                        mc_cmd_sender.send(ServerCommand::StopServer { forever: true }).await.unwrap();
                    },
                }
            },
            // TODO: get rid of this
            else => break,
        }
    }

    Ok(())
}
