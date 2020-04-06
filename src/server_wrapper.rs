use tokio::prelude::*;
use tokio::process::Command;
use tokio::io::BufReader;
use futures::StreamExt;
use futures::future::Future;
use futures::channel::mpsc::Receiver;

use std::process::{Stdio, ExitStatus};
use std::sync::Arc;

use indicatif::{ProgressBar, ProgressStyle};

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
pub async fn run_server(
    opt: &Opt,
    discord_client: Option<Arc<DiscordClient>>,
    _discord_cluster: Option<Arc<Cluster>>,
    // TODO: get rid of the `Future` in this type
    mut servercmd_receiver: Receiver<impl Future<Output=Result<Option<ServerCommand>, Error>> + Send + 'static>
) -> Result<ExitStatus, ServerError> {
    let folder = opt.server_path.as_path().parent().unwrap();
    let file = opt.server_path.file_name().unwrap();

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
                }
                ConsoleMsgSpecific::PlayerAuth { generic_msg, .. } => println!("{}", generic_msg),
                ConsoleMsgSpecific::PlayerLogin { generic_msg, .. } => println!("{}", generic_msg),
                ConsoleMsgSpecific::PlayerMsg { generic_msg, player, player_msg } => {
                    println!("{}", generic_msg);

                    if let Some(discord_client) = discord_client.clone() {
                        // TODO: error handling
                        tokio::spawn(async move {
                            discord_client
                                // TODO: don't hardcode
                                .create_message(ChannelId(694351655667367957))
                                .content("**".to_string() + &player + "**: " + &player_msg)
                                .await
                        });
                    }
                },
                ConsoleMsgSpecific::SpawnPrepareProgress { progress, .. } => {
                    progress_bar.set_position(progress as u64);

                    if progress == 100 {
                        progress_bar.finish();
                    }
                }
            }
        }

        ret
    });

    tokio::spawn(async move {
        while let Some(cmd) = servercmd_receiver.next().await {
            let cmd = cmd.await;

            if let Ok(Some(cmd)) = cmd {
                match cmd {
                    ServerCommand::SendChatMsg(msg) => {
                        let _ = stdin.write_all(("tellraw @a [\"".to_string() + "[Discord] " + &msg + "\"]\n").as_bytes()).await;

                        // TODO: This will not add the message to the server logs
                        println!("{}", ConsoleMsg {
                            timestamp: chrono::offset::Local::now().naive_local().time(),
                            thread_name: "".into(),
                            msg_type: ConsoleMsgType::Info,
                            msg: "[Discord] ".to_string() + &msg
                        });
                    }
                }
            }
        }
    });

    let (status, stdout_val, stderr_val) = tokio::join!(
        status_handle,
        stdout_handle,
        stderr_handle,
    );

    // If we received lines from the server's stderr, we treat those as more important
    // than anything else and return an error containing them
    //
    // This will need to be revised if any MC servers use stderr for anything
    // that would not prevent the server from being restarted.
    if stderr_val.as_ref().unwrap().len() > 0 {
        Err(ServerError::StdErrMsg(stderr_val.unwrap()))
    } else {
        stdout_val.unwrap().map(|_| status.unwrap())
    }
}
