use std::path::PathBuf;
use std::{collections::HashSet, time::Instant};

use anyhow::anyhow;
use anyhow::Context;

use tokio::sync::mpsc;
use tokio::{stream::StreamExt, sync::Mutex};

use once_cell::sync::OnceCell;
use scopeguard::defer;

use twilight::model::id::ChannelId;

use mc_server_wrapper_lib::communication::*;
use mc_server_wrapper_lib::parse::*;
use mc_server_wrapper_lib::CONSOLE_MSG_LOG_TARGET;
use mc_server_wrapper_lib::{McServer, McServerConfig};

use dotenv::dotenv;
use log::*;

use crate::discord::util::{format_online_players, sanitize_for_markdown};
use crate::discord::*;

use crate::ui::TuiState;

use crossterm::{
    cursor::MoveTo,
    event::EventStream,
    event::{Event, KeyCode},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use structopt::clap::AppSettings;
use structopt::StructOpt;
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

    /// Logging level for Discord-related dependencies [error, warn, info, debug,
    /// trace]
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
    // TODO: don't expect here
    let mut mc_server = McServer::new(mc_config)?;

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
            mc_server.cmd_sender.clone(),
        )
        .await
        .with_context(|| "Failed to connect to Discord")?
    } else {
        DiscordBridge::new_noop()
    };

    mc_server
        .cmd_sender
        .send(ServerCommand::StartServer)
        .await
        .unwrap();
    let mut last_start_time = Instant::now();
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
            e = mc_server.event_receiver.next() => if let Some(e) = e {
                match e {
                    ServerEvent::ConsoleEvent(console_msg, Some(specific_msg)) => {
                        if let ConsoleMsgType::Unknown(ref s) = console_msg.msg_type {
                            warn!("Encountered unknown message type from Minecraft: {}", s);
                        }

                        // TODO: re-add progress bar support for world loading at some point?
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

                    ServerEvent::ServerStopped(process_result, reason) => if let Some(reason) = reason {
                        match reason {
                            ShutdownReason::EulaNotAccepted => {
                                info!("Agreeing to EULA!");
                                mc_server.cmd_sender.send(ServerCommand::AgreeToEula).await.unwrap();
                            }
                        }
                    } else {
                        // TODO: clean this up so there's less repetition
                        // introduce a `should_restart` variable
                        match process_result {
                            Ok(exit_status) => if exit_status.success() {
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
                                    error!("Fatal error believed to have been encountered, not restarting server");
                                    mc_server.cmd_sender.send(ServerCommand::StopServer { forever: true }).await.unwrap();
                                } else {
                                    info!("Restarting server...");
                                    mc_server.cmd_sender.send(ServerCommand::StartServer).await.unwrap();
                                    last_start_time = Instant::now();
                                    // TODO: tell discord that the mc server crashed
                                }
                            },
                            Err(e) => {
                                error!("Minecraft server process finished with error: {}, not restarting server", e);
                                mc_server.cmd_sender.send(ServerCommand::StopServer { forever: true }).await.unwrap();
                            }
                        }
                    }

                    ServerEvent::AgreeToEulaResult(res) => {
                        if let Err(e) = res {
                            error!("Failed to agree to EULA: {:?}", e);
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
            Some(record) = log_receiver.next() => {
                tui_state.logs_state.add_record(record);
            },
            maybe_term_event = term_events.next() => {
                match maybe_term_event {
                    Some(Ok(event)) => {
                        if let Event::Key(key_event) = event {
                            match key_event.code {
                                KeyCode::Enter => {
                                    mc_server.cmd_sender.send(ServerCommand::WriteCommandToStdin(tui_state.input_state.value())).await.unwrap();
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
                        mc_server.cmd_sender.send(ServerCommand::StopServer { forever: true }).await.unwrap();
                    },
                }
            },
            else => break,
        }
    }

    // TODO: need to await completion of this, otherwise it panics
    // discord.clone().set_channel_topic("Minecraft server is offline");

    Ok(())
}
