use tokio::prelude::*;
use tokio::process::Command;
use tokio::io::BufReader;

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
use crate::parse::ConsoleMsgSpecific;

/// Run a minecraft server using the provided `Opt` struct containing arguments
/// entered by the user.
pub async fn run_server(
    opt: &Opt,
    discord_client: Option<Arc<DiscordClient>>,
    discord_cluster: Option<Arc<Cluster>>
) -> Result<ExitStatus, ServerError> {
    let folder = opt.server_path.as_path().parent().unwrap();
    let file = opt.server_path.file_name().unwrap();

    let mut process = Command::new("sh")
        .stdin(Stdio::inherit())
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

    let (status, stdout_val, stderr_val) = tokio::join!(
        status_handle,
        stdout_handle,
        stderr_handle
    );

    // If we received lines from the server's stderr, we treat those as more important
    // than anything else and return an error containing them
    //
    // This will need to be revised if any MC servers use stderr for anything
    // that would not prevent the server from being restarted.
    if stderr_val.as_ref().unwrap().len() > 0 {
        Err(ServerError::StdErr(stderr_val.unwrap()))
    } else {
        stdout_val.unwrap().map(|_| status.unwrap())
    }
}
