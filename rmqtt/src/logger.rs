use std::fs::{File, OpenOptions};
use std::io::{self, Write};

pub use slog::Logger;
use slog::{o, Drain, Record};
use slog_scope::GlobalLoggerGuard;
use slog_term::{CountingWriter, RecordDecorator, ThreadSafeTimestampFn};

use crate::{MqttError, Result, Runtime};

use super::settings::log::{Level, To};

/// Initializes a logger using `slog` and `slog_scope`.
///
/// This function creates a `GlobalLoggerGuard` and sets the global logger to the `logger` passed
/// in the `Runtime` instance. It also initializes `slog_stdlog` with the log level specified in
/// the `Runtime` settings.
pub fn logger_init() -> GlobalLoggerGuard {
    let level = slog_log_to_level(Runtime::instance().settings.log.level.inner());
    let logger = Runtime::instance().logger.clone();
    // Make sure to save the guard, see documentation for more information
    let guard = slog_scope::set_global_logger(logger.clone());
    // register slog_stdlog as the log handler with the log crate
    slog_stdlog::init_with_level(level).unwrap();
    guard
}

fn slog_log_to_level(level: slog::Level) -> log::Level {
    match level {
        slog::Level::Trace => log::Level::Trace,
        slog::Level::Debug => log::Level::Debug,
        slog::Level::Info => log::Level::Info,
        slog::Level::Warning => log::Level::Warn,
        slog::Level::Error => log::Level::Error,
        slog::Level::Critical => log::Level::Error,
    }
}

/// Creates a new `slog::Logger` with two `Drain`s: one for printing to the console and another for
/// printing to a file.
///
/// This function takes three arguments: `filename`, which specifies the name of the file to print
/// to; `to`, which specifies where to print the logs (either the console or a file); and `level`,
/// which specifies the minimum log level to print. The function sets the format for the logs and
/// creates the two `Drain`s using the provided parameters. It then combines the two `Drain`s using a
/// `Tee` and returns the resulting `Logger`.
pub fn config_logger(filename: String, to: To, level: Level) -> slog::Logger {
    let custom_timestamp =
        |io: &mut dyn io::Write| write!(io, "{}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"));

    let print_msg_header = |fn_timestamp: &dyn ThreadSafeTimestampFn<Output = io::Result<()>>,
                            mut rd: &mut dyn RecordDecorator,
                            record: &Record,
                            _use_file_location: bool|
     -> io::Result<bool> {
        rd.start_timestamp()?;
        fn_timestamp(&mut rd)?;

        rd.start_whitespace()?;
        write!(rd, " ")?;

        rd.start_level()?;
        write!(rd, "{}", record.level().as_short_str())?;

        rd.start_location()?;
        if record.function().is_empty() {
            write!(rd, " {}.{} | ", record.module(), record.line())?;
        } else {
            write!(rd, " {}::{}.{} | ", record.module(), record.function(), record.line())?;
        }

        rd.start_msg()?;
        let mut count_rd = CountingWriter::new(&mut rd);
        write!(count_rd, "{}", record.msg())?;
        Ok(count_rd.count() != 0)
    };

    //Console
    let plain = slog_term::PlainSyncDecorator::new(std::io::stdout());
    let stdout_drain = slog_term::FullFormat::new(plain)
        .use_custom_timestamp(custom_timestamp)
        .use_custom_header_print(print_msg_header)
        .build()
        .fuse();

    let stdout_drain = stdout_drain.filter_level(level.inner()).fuse();

    //File
    let decorator = slog_term::PlainSyncDecorator::new(open_file(&filename).unwrap());
    let file_drain = slog_term::FullFormat::new(decorator)
        .use_custom_timestamp(custom_timestamp)
        .use_custom_header_print(print_msg_header)
        .build()
        .fuse();

    //@TODO config ...
    let file_drain = slog_async::Async::new(file_drain)
        .chan_size(100_000)
        .overflow_strategy(slog_async::OverflowStrategy::DropAndReport)
        .build()
        .fuse();

    let file_drain = file_drain.filter_level(level.inner()).fuse();

    match to {
        To::Console => slog::Logger::root(stdout_drain, o!()),
        To::File => slog::Logger::root(file_drain, o!()),
        To::Both => slog::Logger::root(slog::Duplicate::new(stdout_drain, file_drain).fuse(), o!()),
        To::Off => slog::Logger::root(slog::Discard, o!()),
    }
}

fn open_file(filename: &str) -> Result<File> {
    OpenOptions::new()
        .create(true)
        .write(true)
        .append(true)
        .open(filename)
        .map_err(|e| MqttError::from(format!("logger file config error, filename: {}, {:?}", filename, e)))
}
