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

use crate::discord::{
    util::{format_online_players, sanitize_for_markdown},
    *,
};

use crate::ui::TuiState;

use crossterm::{
    cursor::MoveTo,
    event::{Event, EventStream, KeyCode},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use structopt::{clap::AppSettings, StructOpt};
use tui::{
    backend::{Backend, CrosstermBackend},
    Terminal,
};
use unicode_width::UnicodeWidthStr;

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

    /// Enable JMX monitoring
    #[structopt(short = "j", long)]
    enable_jmx: bool,

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

    info!("Starting the Minecraft server");
    if let Some(jvm_flags) = opt.jvm_flags.as_ref() {
        info!("Custom JVM flags: {}", jvm_flags);
    }
    let mc_config = McServerConfig::new(
        opt.server_path.clone(),
        opt.memory,
        opt.enable_jmx,
        None,
        opt.jvm_flags,
        false,
    );
    let (mc_server, mut mc_cmd_sender, mut mc_event_receiver) = McServerManager::new().await;

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
        let size = terminal.backend().size().unwrap();
        // Move the cursor back into the input box
        // This is ugly but it's the only way to do it as of right now
        terminal
            .backend_mut()
            .execute(MoveTo(
                2 + tui_state.input_state.value().width() as u16,
                size.height - 2,
            ))
            .unwrap();

        tokio::select! {
            e = mc_event_receiver.next() => if let Some(e) = e {
                match e {
                    ServerEvent::ConsoleEvent(console_msg, Some(specific_msg)) => {
                        if let ConsoleMsgType::Unknown(ref s) = console_msg.msg_type {
                            warn!("Encountered unknown message type from Minecraft: {}", s);
                        }

                        // TODO: re-add progress bar support for world loading at some point?
                        // TODO: parse when server is done booting so we can set Discord
                        // channel topic
                        match specific_msg {
                            ConsoleMsgSpecific::PlayerLogout { name } => {
                                discord.clone().send_channel_msg(format!(
                                    "_**{}** left the game_",
                                    sanitize_for_markdown(&name)
                                ));

                                let mut online_players = ONLINE_PLAYERS.get().unwrap().lock().await;
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

                                let mut online_players = ONLINE_PLAYERS.get().unwrap().lock().await;
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
                            _ => {}
                        }

                        console_msg.log();
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
                        discord.clone().set_channel_topic("Minecraft server is offline");

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
                                info!("Restarting server...");
                                discord.clone().send_channel_msg("Restarting the Minecraft server...");
                            } else {
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

                    ServerEvent::GetAverageTickTimeResult(res) => {
                        match res {
                            Ok(time) => info!("Average tick time: {}", time),
                            Err(e) => error!("Failed to get average tick time: {}", e),
                        }
                    }
                    ServerEvent::GetTickTimesResult(res) => {
                        match res {
                            Ok(time) => info!("Tick times: {:?}", time),
                            Err(e) => error!("Failed to get tick times: {}", e),
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
                                        match tui_state.input_state.value() {
                                            "avgtick" => mc_cmd_sender.send(ServerCommand::GetAverageTickTime).await.unwrap(),
                                            "ticktimes" => mc_cmd_sender.send(ServerCommand::GetTickTimes).await.unwrap(),
                                            _ => mc_cmd_sender.send(ServerCommand::WriteCommandToStdin(tui_state.input_state.value().to_string())).await.unwrap(),
                                        }
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
