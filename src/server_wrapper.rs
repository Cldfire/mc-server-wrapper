use tokio::prelude::*;
use tokio::process::Command;
use tokio::io::BufReader;
use futures::{StreamExt, SinkExt};
use futures::channel::mpsc::{Sender, Receiver};

use std::process::{Stdio, ExitStatus};
use std::sync::Arc;

use indicatif::{ProgressBar, ProgressStyle};
use minecraft_chat::{MessageBuilder, Payload, Color};

use twilight::{
    gateway::Cluster,
    http::Client as DiscordClient,
    model::id::ChannelId
};

use crate::error::*;
use crate::Opt;
use crate::parse::{ConsoleMsgSpecific, ConsoleMsg, ConsoleMsgType};
use crate::command::ServerCommand;

/// Run a minecraft server using the provided `Opt` struct containing arguments
/// entered by the user.
///
/// Returns a tuple of (ExitStatus of server, lines from server stderr).
pub async fn run_server(
    opt: &Opt,
    discord_client: Option<Arc<DiscordClient>>,
    _discord_cluster: Option<Arc<Cluster>>,
    mut servercmd_sender: Sender<ServerCommand>,
    mut servercmd_receiver: Receiver<ServerCommand>
) -> Result<(ExitStatus, Vec<String>), ServerError> {
    let folder = opt.server_path.as_path().parent().unwrap();
    let file = opt.server_path.file_name().unwrap();
    let discord_channel_id = opt.discord_channel_id.unwrap();

    // TODO: support running from inside folder containing server jar
    // (don't run cd)
    let mut process = Command::new("sh")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .args(&[
            "-c",
            &format!(
                "cd {} && exec java -Xms{}M -Xmx{}M -jar {} nogui",
                folder.to_str().unwrap(),
                opt.memory,
                opt.memory,
                file.to_str().unwrap()
            )
        ]).spawn().unwrap();
    
    let mut our_stdin = BufReader::new(tokio::io::stdin()).lines();
    let mut stdin = process.stdin.take().unwrap();
    let mut stdout = BufReader::new(process.stdout.take().unwrap()).lines();
    let mut stderr = BufReader::new(process.stderr.take().unwrap()).lines();

    let status_handle = tokio::spawn(async {
        process.await.expect("child process encountered an error")
    });

    let stderr_handle = tokio::spawn(async move {
        let mut lines = Vec::new();

        while let Some(line) = stderr.next_line().await.unwrap() {
            println!("ERR: {}", &line);
            lines.push(line);
        }

        lines
    });

    let stdout_handle = tokio::spawn(async move {
        let mut ret = Ok(());

        let progress_bar = ProgressBar::new(100);
        progress_bar.set_style(ProgressStyle::default_bar()
            .template("{bar:30[>20]} {pos:>2}%")
        );

        while let Some(line) = stdout.next_line().await.unwrap() {
            let parsed = match ConsoleMsgSpecific::try_parse_from(&line) {
                Some(msg) => msg,
                None => {
                    // spigot servers print lines that reach this branch ("\n",
                    // "Loading libraries, please wait...")
                    println!("{}", line);
                    continue;
                }
            };
    
            match parsed {
                ConsoleMsgSpecific::GenericMsg(generic_msg) => println!("{}", generic_msg),
                ConsoleMsgSpecific::MustAcceptEula(generic_msg) => {
                    println!("{}", generic_msg);
                    ret = Err(ServerError::EulaNotAccepted);
                },
                ConsoleMsgSpecific::PlayerLostConnection { generic_msg, .. } => println!("{}", generic_msg),
                ConsoleMsgSpecific::PlayerLogout { generic_msg, name } => {
                    println!("{}", generic_msg);

                    if let Some(discord_client) = discord_client.clone() {
                        tokio::spawn(async move {
                            discord_client
                                .create_message(ChannelId(discord_channel_id))
                                .content("_**".to_string() + &name + "** left the game_")
                                .await
                        });
                    }
                },
                ConsoleMsgSpecific::PlayerAuth { generic_msg, .. } => println!("{}", generic_msg),
                ConsoleMsgSpecific::PlayerLogin { generic_msg, name, .. } => {
                    println!("{}", generic_msg);

                    if let Some(discord_client) = discord_client.clone() {
                        tokio::spawn(async move {
                            discord_client
                                .create_message(ChannelId(discord_channel_id))
                                .content("_**".to_string() + &name + "** joined the game_")
                                .await
                        });
                    }
                },
                ConsoleMsgSpecific::PlayerMsg { generic_msg, name, msg } => {
                    println!("{}", generic_msg);

                    if let Some(discord_client) = discord_client.clone() {
                        // TODO: error handling
                        tokio::spawn(async move {
                            discord_client
                                .create_message(ChannelId(discord_channel_id))
                                .content("**".to_string() + &name + "**  " + &msg)
                                .await
                        });
                    }
                },
                ConsoleMsgSpecific::SpawnPrepareProgress { progress, .. } => {
                    progress_bar.set_position(progress as u64);
                },
                ConsoleMsgSpecific::SpawnPrepareFinish { time_elapsed_ms, .. } => {
                    progress_bar.finish_and_clear();
                    println!("  (finished in {} ms)", time_elapsed_ms);
                }
            }
        }

        if !progress_bar.is_finished() {
            progress_bar.finish_and_clear();
        }
        ret
    });

    let input_handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                Some(cmd) = servercmd_receiver.next() => {
                    match cmd {
                        ServerCommand::SendDiscordMsg{ username, msg } => {
                            let tellraw_msg = MessageBuilder::builder(Payload::text("[D] "))
                                .bold(true)
                                .color(Color::LightPurple)
                                .then(Payload::text(&("<".to_string() + &username + "> " + &msg)))
                                .bold(false)
                                .color(Color::White)
                                .build();

                            let _ = stdin.write_all(
                                ("tellraw @a ".to_string() + &tellraw_msg.to_json().unwrap() + "\n")
                                .as_bytes()
                            ).await;
    
                            // TODO: This will not add the message to the server logs
                            println!("{}", ConsoleMsg {
                                timestamp: chrono::offset::Local::now().naive_local().time(),
                                thread_name: "".into(),
                                msg_type: ConsoleMsgType::Info,
                                msg: "[D] <".to_string() + &username + "> " + &msg
                            });
                        },
                        ServerCommand::ServerClosed => break
                    }
                },

                Some(line) = our_stdin.next() => {
                    if let Ok(line) = line {
                        let _ = stdin.write_all((line + "\n").as_bytes()).await;
                    } else {
                        break;
                    }
                },
                else => break,
            }
        }
    });

    let (status, stdout_val, stderr_val) = tokio::join!(
        status_handle,
        stdout_handle,
        stderr_handle,
    );

    servercmd_sender.send(ServerCommand::ServerClosed).await.unwrap();
    let _ = input_handle.await;

    stdout_val.unwrap().map(|_| (status.unwrap(), stderr_val.unwrap()))
}
