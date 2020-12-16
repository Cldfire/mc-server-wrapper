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

use mc_server_wrapper_lib::{communication::*, McServerConfig, McServerManager};

#[derive(StructOpt, Debug)]
pub struct Opt {
    /// Path to the Minecraft server jar
    #[structopt(parse(from_os_str))]
    server_path: PathBuf,
}

#[tokio::main]
async fn main() {
    let opt = Opt::from_args();

    let config = McServerConfig::new(opt.server_path.clone(), 1024, None, true);
    let (_, mut cmd_sender, mut event_receiver) = McServerManager::new();
    cmd_sender
        .send(ServerCommand::StartServer {
            config: Some(config),
        })
        .await
        .unwrap();

    while let Some(e) = event_receiver.next().await {
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
                if let Some(ShutdownReason::EulaNotAccepted) = reason {
                    println!("Agreeing to EULA!");
                    cmd_sender.send(ServerCommand::AgreeToEula).await.unwrap();
                } else {
                    match process_result {
                        Ok(exit_status) => {
                            if !exit_status.success() {
                                eprintln!("Minecraft server process finished with {}", exit_status)
                            }
                        }
                        Err(e) => eprintln!("Minecraft server process finished with error: {}", e),
                    }

                    // Note that this example does not implement any kind of restart-after-crash
                    // functionality
                    cmd_sender
                        .send(ServerCommand::StopServer { forever: true })
                        .await
                        .unwrap();
                }
            }

            ServerEvent::AgreeToEulaResult(res) => {
                if let Err(e) = res {
                    eprintln!("Failed to agree to EULA: {:?}", e);
                    cmd_sender
                        .send(ServerCommand::StopServer { forever: true })
                        .await
                        .unwrap();
                } else {
                    cmd_sender
                        .send(ServerCommand::StartServer { config: None })
                        .await
                        .unwrap();
                }
            }
            ServerEvent::SetServerPropertyResult { .. } => {}
            ServerEvent::StartServerResult(res) => {
                if let Err(e) = res {
                    eprintln!("Failed to start the Minecraft server: {}", e);
                    cmd_sender
                        .send(ServerCommand::StopServer { forever: true })
                        .await
                        .unwrap();
                }
            }
        }
    }
}
