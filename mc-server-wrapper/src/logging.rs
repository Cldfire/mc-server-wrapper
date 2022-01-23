use mc_server_wrapper_lib::CONSOLE_MSG_LOG_TARGET;
use std::path::Path;
use time::format_description::FormatItem;
use tokio::sync::mpsc::Sender;

pub fn setup_logger<P: AsRef<Path>>(
    logfile_path: P,
    log_sender: Sender<String>,
    log_level_all: log::Level,
    log_level_self: log::Level,
    log_level_discord: log::Level,
) -> Result<(), fern::InitError> {
    let file_logger = fern::Dispatch::new()
        .format(|out, message, record| {
            const LOG_TIMESTAMP_FORMAT: &[FormatItem] = time::macros::format_description!(
                "[[[month]-[day]-[year]][[[hour repr:12 padding:none]:[minute]:[second] [period]]"
            );

            let formatted_time_now = || -> Option<String> {
                // TODO: log errors here somehow
                time::OffsetDateTime::now_local()
                    .ok()
                    .and_then(|datetime| datetime.format(&LOG_TIMESTAMP_FORMAT).ok())
            };

            out.finish(format_args!(
                "{}[{}][{}] {}",
                formatted_time_now().unwrap_or_else(|| String::from("time error")),
                record.target(),
                record.level(),
                message
            ))
        })
        .level(log_level_all.to_level_filter())
        .level_for("twilight_http", log_level_discord.to_level_filter())
        .level_for("twilight_gateway", log_level_discord.to_level_filter())
        .level_for("twilight-cache", log_level_discord.to_level_filter())
        .level_for(
            "twilight-command-parser",
            log_level_discord.to_level_filter(),
        )
        .level_for("twilight-model", log_level_discord.to_level_filter())
        .level_for(
            "twilight-cache-inmemory",
            log_level_discord.to_level_filter(),
        )
        .level_for("twilight-cache-trait", log_level_discord.to_level_filter())
        .level_for("mc_server_wrapper", log_level_self.to_level_filter())
        .level_for(
            *CONSOLE_MSG_LOG_TARGET.get().unwrap(),
            log::LevelFilter::Off,
        )
        .chain(fern::log_file(logfile_path)?);

    let tui_logger = fern::Dispatch::new()
        .level(log::LevelFilter::Error)
        .level_for("twilight_http", log::LevelFilter::Warn)
        .level_for("twilight_gateway", log::LevelFilter::Warn)
        .level_for("twilight-cache", log::LevelFilter::Warn)
        .level_for("twilight-command-parser", log::LevelFilter::Warn)
        .level_for("twilight-model", log::LevelFilter::Warn)
        .level_for("twilight-cache-inmemory", log::LevelFilter::Warn)
        .level_for("twilight-cache-trait", log::LevelFilter::Warn)
        .level_for("mc_server_wrapper", log::LevelFilter::Info)
        .level_for(
            *CONSOLE_MSG_LOG_TARGET.get().unwrap(),
            log::LevelFilter::Info,
        )
        .chain(fern::Output::call(move |record| {
            const CONSOLE_TIMESTAMP_FORMAT: &[FormatItem] = time::macros::format_description!(
                "[hour repr:12 padding:none]:[minute]:[second] [period]"
            );

            let formatted_time_now = || -> Option<String> {
                // TODO: log errors here somehow
                time::OffsetDateTime::now_local()
                    .ok()
                    .and_then(|datetime| datetime.format(&CONSOLE_TIMESTAMP_FORMAT).ok())
            };

            let record = format!(
                "[{}] [{}, {}]: {}",
                formatted_time_now().unwrap_or_else(|| String::from("time error")),
                record.target(),
                record.level(),
                record.args()
            );

            let log_sender_clone = log_sender.clone();
            // TODO: right now log messages can print out-of-order because we
            // don't block on sending them
            //
            // Tried using `Handle::block_on` but couldn't get it to not panic
            // with `Illegal instruction`
            //
            // Need to investigate
            tokio::spawn(async move {
                let _ = log_sender_clone.send(record).await;
            });
        }));

    fern::Dispatch::new()
        .chain(tui_logger)
        .chain(file_logger)
        .apply()?;

    Ok(())
}
