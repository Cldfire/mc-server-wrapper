use std::path::PathBuf;
use std::io;

use tokio::fs::File;
use tokio::prelude::*;

use structopt::StructOpt;
use crate::server_wrapper::run_server;
use crate::error::ServerError;

mod server_wrapper;
mod error;
mod parse;

#[derive(StructOpt, Debug)]
#[structopt(name = "mc-wrapper")]
pub struct Opt {
    /// The path to the server jar to execute
    #[structopt(parse(from_os_str))]
    server_path: PathBuf,

    /// The amount of memory in megabytes to allocate for the server
    #[structopt(short = "m", long = "memory", default_value = "1024")]
    memory: u16
}

/// Overwrites the `eula.txt` file with the contents `eula=true`.
async fn agree_to_eula(opt: &Opt) -> io::Result<()> {
    let mut file = File::create(opt.server_path.parent().unwrap().join("eula.txt")).await?;
    file.write_all(b"eula=true").await
}

#[tokio::main]
async fn main() {
    let opt = Opt::from_args();

    loop {
        match run_server(&opt).await {
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
}
