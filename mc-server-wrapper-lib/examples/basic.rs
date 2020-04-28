/*!
Simple usage of the library to transparently wrap a Minecraft server.

This example stays as close to the default server experience as possible;
the only changes are improved console output formatting and auto-agreeing
to the EULA.

This is a nice starter for quickly adding some simple event-driven functionality
to a Minecraft server.
*/

use std::path::PathBuf;
use tokio::stream::StreamExt;

use structopt::StructOpt;

use mc_server_wrapper_lib::communication::*;
use mc_server_wrapper_lib::{McServer, McServerConfig};

#[derive(StructOpt, Debug)]
pub struct Opt {
    /// Path to the Minecraft server jar
    #[structopt(parse(from_os_str))]
    server_path: PathBuf,
}

#[tokio::main]
async fn main() {
    let opt = Opt::from_args();

    let mc_config = McServerConfig::new(opt.server_path.clone(), 1024, None, true);
    let mut mc_server = McServer::new(mc_config).expect("minecraft server config was not valid");
    mc_server
        .cmd_sender
        .send(ServerCommand::StartServer)
        .await
        .unwrap();

    while let Some(e) = mc_server.event_receiver.next().await {
        match e {
            ServerEvent::ConsoleEvent(console_msg, Some(specific_msg)) => {
                println!("{}", console_msg);
                // You can match on and handle the `specific_msg`s as desired
                println!("      specific_msg: {:?}", specific_msg);
            }
            ServerEvent::ConsoleEvent(console_msg, None) => {
                println!("{}", console_msg);
            }
            ServerEvent::StdoutLine(line) => {
                println!("{}", line);
            }
            ServerEvent::StderrLine(line) => {
                eprintln!("{}", line);
            }

            ServerEvent::ServerStopped(process_result, reason) => {
                if let Some(reason) = reason {
                    match reason {
                        ShutdownReason::EulaNotAccepted => {
                            println!("Agreeing to EULA!");
                            mc_server
                                .cmd_sender
                                .send(ServerCommand::AgreeToEula)
                                .await
                                .unwrap();
                        }
                    }
                } else {
                    match process_result {
                        Ok(exit_status) => {
                            println!("Minecraft server process finished with {}", exit_status)
                        }
                        Err(e) => eprintln!("Minecraft server process finished with error: {}", e),
                    }

                    // Note that this example does not implement any kind of restart-after-crash
                    // functionality
                    mc_server
                        .cmd_sender
                        .send(ServerCommand::StopServer { forever: true })
                        .await
                        .unwrap();
                }
            }

            ServerEvent::AgreeToEulaResult(res) => {
                if let Err(e) = res {
                    eprintln!("Failed to agree to EULA: {:?}", e);
                    mc_server
                        .cmd_sender
                        .send(ServerCommand::StopServer { forever: true })
                        .await
                        .unwrap();
                } else {
                    mc_server
                        .cmd_sender
                        .send(ServerCommand::StartServer)
                        .await
                        .unwrap();
                }
            }
        }
    }
}
