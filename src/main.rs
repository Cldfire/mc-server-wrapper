use std::path::PathBuf;
use std::io;
use std::env;
use std::sync::Arc;

use tokio::fs::File;
use tokio::prelude::*;
use tokio::runtime::Runtime;
use futures::StreamExt;

use twilight::{
    gateway::{shard::Event, Cluster, ClusterConfig},
    http::Client as DiscordClient,
    model::channel::message::MessageType
};

use dotenv::dotenv;
use structopt::StructOpt;
use crate::server_wrapper::run_server;
use crate::error::*;

mod server_wrapper;
mod error;
mod parse;

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
    discord_client: Arc<DiscordClient>,
) -> Result<(), Error> {
    match event {
        (id, Event::Ready(_)) => {
            println!("Connected on shard {}", id);
        }
        (_, Event::MessageCreate(msg)) => {
            // TODO: maybe some bot chatter should be allowed through?
            if msg.kind == MessageType::Regular && !msg.author.bot {
                // TODO: send message to mc server
            }
        }
        _ => {}
    }

    Ok(())
}

fn main() -> Result<(), Error> {
    let mut rt = Runtime::new().unwrap();

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

            let cluster_clone = discord_cluster.as_ref().unwrap().clone();
            let client_clone = discord_client.as_ref().unwrap().clone();

            tokio::spawn(async move {
                let mut events = cluster_clone.clone().events().await;
                while let Some(event) = events.next().await {
                    tokio::spawn(handle_discord_event(event, client_clone.clone()));
                }
            });
        } else {
            discord_client = None;
            discord_cluster = None;
        }
    
        loop {
            match run_server(&opt, discord_client.clone(), discord_cluster.clone()).await {
                Ok(status) => if status.success() {
                    break;
                } else {
                    println!("Restarting server...");
                },
                Err(ServerError::EulaNotAccepted) => {
                    println!("Agreeing to EULA!");
                    if let Err(e) = agree_to_eula(&opt).await {
                        println!("Failed to agree to EULA: {:?}", e);
                        break;
                    }
                },
                Err(ServerError::StdErr(_)) => {
                    println!("Fatal error believed to have been encountered, not \
                                restarting server");
                    break;
                }
            }
        }
    });

    Ok(())
}
