use std::path::PathBuf;
use std::fs::File;
use std::io::Write;

use structopt::StructOpt;
use crate::server_wrapper::start_server;
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
// TODO: Error handling
fn agree_to_eula(opt: &Opt) {
    println!("Agreeing to EULA!");
    let mut file = File::create(opt.server_path.parent().unwrap().join("eula.txt")).unwrap();
    file.write_all(b"eula=true").unwrap();
}

fn main() {
    let opt = Opt::from_args();

    loop {
        match start_server(&opt) {
            Ok(()) => break,
            Err(ServerError::EulaNotAccepted) => {
                agree_to_eula(&opt);
            }
        }
    }
}
