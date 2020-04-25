use mc_server_wrapper_lib::CONSOLE_MSG_LOG_TARGET;
use std::path::Path;

pub fn setup_logger<P: AsRef<Path>>(
    logfile_path: P,
    log_level_all: log::Level,
    log_level_self: log::Level,
    log_level_discord: log::Level,
) -> Result<(), fern::InitError> {
    let colors = fern::colors::ColoredLevelConfig::new();

    let file_logger = fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "{}[{}][{}] {}",
                chrono::Local::now().format("[%m-%d-%Y][%-I:%M:%S %p]"),
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

    let stdout_logger = fern::Dispatch::new()
        .format(move |out, message, record| {
            out.finish(format_args!(
                "[{}] [{}, {}]: {}",
                chrono::Local::now().format("%-I:%M:%S %p"),
                record.target(),
                colors.color(record.level()),
                message
            ))
        })
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
        .chain(std::io::stdout());

    fern::Dispatch::new()
        .chain(stdout_logger)
        .chain(file_logger)
        .apply()?;

    Ok(())
}
