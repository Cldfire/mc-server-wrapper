use std::process::{Command, Stdio};
use std::io::{BufRead, BufReader};
use std::thread;

use crate::error::ServerError;
use crate::Opt;
use crate::parse::ConsoleMsgSpecific;

/// Launch a minecraft server using the provided `Opt` struct containing arguments
/// entered by the user.
/// 
/// This is a blocking function that returns after the server child process has
/// exited.
pub fn start_server(opt: &Opt) -> Result<(), ServerError> {
    let folder = opt.server_path.as_path().parent().unwrap();
    let file = opt.server_path.file_name().unwrap();

    let process = Command::new("sh")
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

    let _stdin = process.stdin.unwrap();
    let stdout = BufReader::new(process.stdout.unwrap());
    let stderr = BufReader::new(process.stderr.unwrap());
    

    let stdout_handle = thread::spawn(move || {
        let mut ret = Ok(());

        for line in stdout.lines().map(|l| l.unwrap()) {
            let parsed = ConsoleMsgSpecific::parse_from(&line);

            match parsed {
                ConsoleMsgSpecific::GenericMsg(generic_msg) => println!("{}", generic_msg),
                ConsoleMsgSpecific::MustAcceptEula(generic_msg) => {
                    println!("{}", generic_msg);
                    ret = Err(ServerError::EulaNotAccepted);
                }
                ConsoleMsgSpecific::PlayerAuth { generic_msg, .. } => println!("{}", generic_msg),
                ConsoleMsgSpecific::PlayerLogin { generic_msg, .. } => println!("{}", generic_msg),
                ConsoleMsgSpecific::PlayerMsg { generic_msg, .. } => println!("{}", generic_msg),
                ConsoleMsgSpecific::SpawnPrepareProgress { generic_msg, .. } => println!("{}", generic_msg)
            }
        }

        ret
    });

    let stderr_handle = thread::spawn(|| {
        for line in stderr.lines() {
            println!("ERR: {}", line.unwrap());
        }
    });

    let ret = stdout_handle.join().unwrap();
    stderr_handle.join().unwrap();

    ret
}
