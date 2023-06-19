use std::{collections::BTreeMap, path::PathBuf, time::Instant};

use anyhow::Context;

use futures::{FutureExt, StreamExt};
use time::OffsetDateTime;
use tokio::sync::{mpsc, Mutex};

use once_cell::sync::OnceCell;
use scopeguard::defer;

use mc_server_wrapper_lib::{
    communication::*, parse::*, McServerConfig, McServerManager, CONSOLE_MSG_LOG_TARGET,
};

use log::*;
use tokio::task::AbortHandle;

use crate::discord::{util::sanitize_for_markdown, *};

use crate::liveview::LiveViewFromServer;
use crate::ui::TuiState;

use config::Config;
use crossterm::{
    event::EventStream,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{backend::CrosstermBackend, Terminal};
use structopt::StructOpt;
use util::{format_online_players, OnlinePlayerFormat};

mod config;
mod discord;
mod liveview;
mod logging;
mod ui;

static APPLICATION_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Maintains a hashset of players currently on the Minecraft server
///
/// Player name -> info
static ONLINE_PLAYERS: OnceCell<Mutex<BTreeMap<String, OnlinePlayerInfo>>> = OnceCell::new();

/// Info about online players
#[derive(Debug)]
pub struct OnlinePlayerInfo {
    joined_at: OffsetDateTime,
}

impl Default for OnlinePlayerInfo {
    fn default() -> Self {
        Self {
            joined_at: OffsetDateTime::now_utc(),
        }
    }
}

#[derive(StructOpt, Debug)]
pub struct Opt {
    /// Path to config
    #[structopt(
        short = "c",
        long,
        parse(from_os_str),
        default_value = "./mc-server-wrapper-config.toml"
    )]
    config: PathBuf,

    /// Generate a default config and then exit the program
    #[structopt(short = "g", long)]
    gen_config: bool,

    /// Print application version and then exit the program
    #[structopt(short = "V", long)]
    version: bool,

    /// Path to the Minecraft server jar
    #[structopt(parse(from_os_str))]
    server_path: Option<PathBuf>,

    /// Bridge server chat to discord
    #[structopt(short = "b", long)]
    bridge_to_discord: bool,
}

#[derive(Debug, Clone)]
pub enum EdgeToCoreCommand {
    MinecraftCommand(ServerCommand),
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    // See https://github.com/time-rs/time/issues/293#issuecomment-1005002386. The
    // unsoundness here is not in the `time` library, but in the Rust stdlib, and as
    // such it needs to be fixed there.
    unsafe {
        time::util::local_offset::set_soundness(time::util::local_offset::Soundness::Unsound);
    }

    log_panics::init();
    CONSOLE_MSG_LOG_TARGET.set("mc").unwrap();
    ONLINE_PLAYERS.set(Mutex::new(BTreeMap::new())).unwrap();

    let opt = Opt::from_args();
    let config_filepath = opt.config.clone();
    let mut config = Config::load(&config_filepath).await?;
    let mut notify_receiver = config.setup_watcher(config_filepath.clone());

    if opt.gen_config {
        return Ok(());
    }

    if opt.version {
        println!("mc-server-wrapper {APPLICATION_VERSION}");
        return Ok(());
    }

    config.merge_in_args(&opt)?;
    let (log_sender, mut log_receiver) = mpsc::channel(64);
    let (edge_to_core_command_tx, mut edge_to_core_command_rx) = mpsc::channel(64);
    let (live_view_server_tx, _) = tokio::sync::broadcast::channel(512);

    logging::setup_logger(
        config
            .minecraft
            .server_path
            .with_file_name("mc-server-wrapper.log"),
        log_sender,
        config.logging.all,
        config.logging.self_level,
        config.logging.discord,
    )
    .with_context(|| "Failed to set up logging")?;

    let mc_config = McServerConfig::new(
        config.minecraft.server_path.clone(),
        config.minecraft.memory,
        config.minecraft.jvm_flags.clone(),
        false,
    );
    let (mc_server, mc_cmd_sender, mut mc_event_receiver) = McServerManager::new();

    let stdout = std::io::stdout();
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let mut tui_state = TuiState::new(edge_to_core_command_tx.clone(), mc_server.clone());

    enable_raw_mode()?;
    terminal.backend_mut().execute(EnterAlternateScreen)?;
    defer! {
        std::io::stdout().execute(LeaveAlternateScreen).unwrap();
        disable_raw_mode().unwrap();
    }

    info!("Starting the Minecraft server");
    mc_cmd_sender
        .send(ServerCommand::StartServer {
            config: Some(mc_config),
        })
        .await
        .unwrap();
    let mut last_start_time = Instant::now();

    // TODO: start drawing UI before setting up discord
    let discord = if let Some(discord_config) = config.discord.as_ref() {
        if discord_config.enable_bridge {
            setup_discord(
                discord_config.token.clone(),
                discord_config.channel_id.into(),
                edge_to_core_command_tx.clone(),
                discord_config.update_status,
            )
            .await
            .with_context(|| "Failed to connect to Discord")?
        } else {
            DiscordBridge::new_noop()
        }
    } else {
        DiscordBridge::new_noop()
    };

    let mut web_server_abort_handle = None;

    let run_web_server = |web_server_abort_handle: &mut Option<AbortHandle>| {
        if web_server_abort_handle.is_some() {
            return;
        }

        let live_view_server_tx_clone = live_view_server_tx.clone();
        let edge_to_core_command_tx_clone = edge_to_core_command_tx.clone();
        let mc_server_clone = mc_server.clone();
        *web_server_abort_handle = Some(
            tokio::spawn(async move {
                liveview::run_web_server(
                    live_view_server_tx_clone,
                    edge_to_core_command_tx_clone,
                    mc_server_clone,
                )
                .await;
            })
            .abort_handle(),
        );
    };

    if config.web.as_ref().map(|w| w.enabled).unwrap_or_default() {
        run_web_server(&mut web_server_abort_handle);
    }

    let mut term_events = EventStream::new();

    // This loop handles both user input and events from the Minecraft server
    loop {
        // Make sure we are up-to-date on logs before drawing the UI
        while let Some(Some(record)) = log_receiver.recv().now_or_never() {
            let _ = live_view_server_tx.send(LiveViewFromServer::LogMessage(record.clone()));
            tui_state.logs_state.add_record(record);
        }

        {
            let online_players = ONLINE_PLAYERS.get().unwrap().lock().await;
            // TODO: figure out what to do if the terminal fails to draw
            let _ = terminal.draw(|f| tui_state.draw(f, &online_players));
        }

        tokio::select! {
            e = mc_event_receiver.recv() => if let Some(e) = e {
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
                                online_players.insert(name, OnlinePlayerInfo::default());
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
            Some(record) = log_receiver.recv() => {
                let _ = live_view_server_tx
                    .send(LiveViewFromServer::LogMessage(record.clone()));
                tui_state.logs_state.add_record(record);
            },
            Some(command_from_edge) = edge_to_core_command_rx.recv() => {
                handle_command_from_edge(command_from_edge, &mc_cmd_sender, &mut last_start_time).await;
            }
            maybe_term_event = term_events.next() => {
                match maybe_term_event {
                    Some(Ok(event)) => {
                        tui_state.handle_input(event).await;
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
            config_file_event = notify_receiver.recv() => {
                match config_file_event {
                    // this currently is not used for anything, it's here
                    // for future use
                    Some(event) => {
                        handle_config_file_event(event, &mut config, &opt).await;

                        match (&web_server_abort_handle, config.web.as_ref().map(|w| w.enabled).unwrap_or_default()) {
                            (Some(_), false) => web_server_abort_handle.take().unwrap().abort(),
                            (None, true) => run_web_server(&mut web_server_abort_handle),
                            _ => {}
                        }
                    },
                    // TODO: should we break or panic in these cases?
                    None => unreachable!()
                }
            }
            // TODO: get rid of this
            else => break,
        }
    }

    Ok(())
}

async fn handle_command_from_edge(
    command: EdgeToCoreCommand,
    mc_cmd_sender: &mpsc::Sender<ServerCommand>,
    last_start_time: &mut Instant,
) {
    match command {
        EdgeToCoreCommand::MinecraftCommand(server_command) => {
            match &server_command {
                ServerCommand::StartServer { .. } => {
                    info!("Starting the Minecraft server");
                    *last_start_time = Instant::now();
                }
                _ => {}
            }

            mc_cmd_sender.send(server_command).await.unwrap();
        }
    }
}

async fn handle_config_file_event(
    event: notify_debouncer_mini::DebounceEventResult,
    config: &mut Config,
    opt: &Opt,
) {
    match event {
        Ok(_) => match Config::load(opt.config.as_path()).await {
            Ok(mut new_config) => match new_config.merge_in_args(opt) {
                Ok(_) => {
                    *config = new_config;
                    info!("Config reloaded successfully");
                }
                Err(e) => error!("Reloading config failed: {}", e),
            },
            Err(e) => error!("Reloading config failed: {}", e),
        },
        Err(errors) => error!("Received errors from config file watcher: {:?}", errors),
    }
}
