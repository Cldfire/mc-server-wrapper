use std::path::PathBuf;
use std::io;
use std::env;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::fs::File;
use tokio::prelude::*;
use tokio::runtime::Runtime;
use futures::{SinkExt, StreamExt};
use futures::channel::mpsc;

use twilight::{
    gateway::{shard::Event, Cluster, ClusterConfig},
    http::Client as DiscordClient,
    model::channel::message::MessageType
};

use dotenv::dotenv;
use structopt::StructOpt;
use crate::server_wrapper::run_server;
use crate::error::*;
use crate::command::ServerCommand;

mod command;
mod error;
mod parse;
mod server_wrapper;

#[derive(StructOpt, Debug)]
#[structopt(name = "mc-wrapper")]
pub struct Opt {
    /// The path to the server jar to execute
    #[structopt(parse(from_os_str))]
    server_path: PathBuf,

    /// Discord bot token (for Discord chat bridge)
    #[structopt(long)]
    discord_token: Option<String>,

    /// Bridge server chat to discord
    #[structopt(short, long, default_value_if("discord_token", None, "true"))]
    bridge_to_discord: bool,

    /// The amount of memory in megabytes to allocate for the server
    #[structopt(short = "m", long = "memory", default_value = "1024")]
    memory: u16
}

/// Overwrites the `eula.txt` file with the contents `eula=true`.
async fn agree_to_eula(opt: &Opt) -> io::Result<()> {
    let mut file = File::create(opt.server_path.parent().unwrap().join("eula.txt")).await?;
    file.write_all(b"eula=true").await
}

async fn handle_discord_event(
    event: (u64, Event),
    _discord_client: Arc<DiscordClient>,
) -> Result<Option<ServerCommand>, Error> {
    match event {
        (_, Event::Ready(_)) => {
            println!("Discord bridge online");
            Ok(None)
        },
        (_, Event::GuildCreate(guild)) => {
            println!("Connected to guild {}", guild.name);
            Ok(None)
        },
        (_, Event::MessageCreate(msg)) => {
            // TODO: maybe some bot chatter should be allowed through?
            if msg.kind == MessageType::Regular && !msg.author.bot {
                Ok(Some(ServerCommand::SendChatMsg(msg.author.name.clone() + ": " + &msg.content)))
            } else {
                Ok(None)
            }
        },
        _ => { Ok(None) }
    }
}

fn main() -> Result<(), Error> {
    let mut rt = Runtime::new().unwrap();

    // TODO: use proc macro instead if shutdown_timeout no longer needed
    rt.block_on(async {
        dotenv().ok();
        let mut opt = Opt::from_args();
        let discord_token = opt.discord_token.take()
            .unwrap_or_else(||
                env::var("DISCORD_TOKEN").unwrap_or_else(|_| String::new())
            );
        let discord_client;
        let discord_cluster;
        
        // Set up discord bridge if enabled
        if opt.bridge_to_discord {
            discord_client = Some(Arc::new(DiscordClient::new(&discord_token)));

            let cluster_config = ClusterConfig::builder(&discord_token).build();
            discord_cluster = Some(Arc::new(Cluster::new(cluster_config)));
            discord_cluster.as_ref().unwrap().up().await
                .expect("Could not connect to Discord");
        } else {
            discord_client = None;
            discord_cluster = None;
        }

        let mut prev_stderr_output = vec![];
        let mut last_start_time;
        loop {
            let (servercmd_sender, servercmd_receiver) = mpsc::channel::<ServerCommand>(32);
            
            if opt.bridge_to_discord {
                let cluster_clone = discord_cluster.as_ref().unwrap().clone();
                let client_clone = discord_client.as_ref().unwrap().clone();
                let servercmd_sender_clone = servercmd_sender.clone();
    
                tokio::spawn(async move {
                    // For all received Discord events, map the event to a `ServerCommand`
                    // (if necessary) and forward it to the `server_cmd` sender
                    let mut events = cluster_clone.events().await;
                    while let Some(e) = events.next().await {
                        let client_clone = client_clone.clone();
                        let mut servercmd_sender_clone = servercmd_sender_clone.clone();
    
                        tokio::spawn(async move {
                            match handle_discord_event(e, client_clone).await {
                                Ok(Some(cmd)) => {
                                    let _ = servercmd_sender_clone.send(cmd).await;
                                },
                                // TODO: error handling
                                _ => {}
                            }
                        });
                    }
                });
            }

            last_start_time = Instant::now();
            match run_server(
                &opt,
                discord_client.clone(),
                discord_cluster.clone(),
                servercmd_sender.clone(),
                servercmd_receiver
            ).await {
                Ok((status, stderr_output)) => if status.success() {
                    break;
                } else {
                    // There are circumstances where the status will be failure
                    // and attempting to restart the server will always fail. We
                    // attempt to catch these cases by saving the stderr output
                    // from the last time we had to restart after failure and
                    // not restarting if the output is the same within a time
                    // window
                    // TODO: this is naive but will have to do for now
                    if stderr_output == prev_stderr_output &&
                        last_start_time.elapsed().as_secs() < 60 {
                        println!("Fatal error believed to have been encountered, not \
                            restarting server");
                        break;
                    } else {
                        prev_stderr_output = stderr_output;
                        println!("Restarting server...")
                    }
                },
                Err(ServerError::EulaNotAccepted) => {
                    println!("Agreeing to EULA!");
                    if let Err(e) = agree_to_eula(&opt).await {
                        println!("Failed to agree to EULA: {:?}", e);
                        break;
                    }
                },
                Err(ServerError::DiscordChannelIdNotSet) => {
                    println!("You enabled the Discord bridge but did not set the \
                            `DISCORD_CHANNEL_ID` environment variable; please \
                            set it to the ID of the channel you want messages \
                            bridged to");
                    break;
                }
            }
        }
    });

    // TODO: we need this because the way tokio handles stdin involves blocking
    rt.shutdown_timeout(Duration::from_millis(5));
    Ok(())
}
