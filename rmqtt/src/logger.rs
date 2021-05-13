use anyhow::Result;
use slog::*;
use slog_logfmt::Logfmt;
use std::fs::{File, OpenOptions};
use std::io::{self, Stdout};

use super::settings::{
    log::{Level, To},
    ValueMut,
};

use crate::Runtime;
pub use slog::Logger;

pub fn logger_init() {
    log::set_boxed_logger(Box::new(LoggerEx(Runtime::instance().logger.clone()))).unwrap();
    let l = Runtime::instance().settings.log.level.get().inner();
    log::set_max_level(slog_log_to_level(l).to_level_filter());
}

struct LoggerEx(Logger);

impl log::Log for LoggerEx {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
    }

    fn log(&self, r: &log::Record) {
        let level = log_to_slog_level(r.metadata().level());
        let args = r.args();
        let target = r.target();
        let location = &record_as_location(r);
        let s = slog::RecordStatic { location, level, tag: target };

        self.0.log(&slog::Record::new(&s, args, b!()))
    }

    fn flush(&self) {}
}

fn log_to_slog_level(level: log::Level) -> slog::Level {
    match level {
        log::Level::Trace => slog::Level::Trace,
        log::Level::Debug => slog::Level::Debug,
        log::Level::Info => slog::Level::Info,
        log::Level::Warn => slog::Level::Warning,
        log::Level::Error => slog::Level::Error,
    }
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

fn record_as_location(r: &log::Record) -> slog::RecordLocation {
    let module = r.module_path_static().unwrap_or("<unknown>");
    let file = r.file_static().unwrap_or("<unknown>");
    let line = r.line().unwrap_or_default();

    slog::RecordLocation { file, line, column: 0, function: "", module }
}

pub fn config_logger(filename: String, to: ValueMut<To>, level: ValueMut<Level>) -> slog::Logger {
    let drain = Logfmt::new(WriteFilter::new(filename, to))
        .set_prefix(move |io: &mut dyn io::Write, rec: &Record| -> slog::Result {
            write!(
                io,
                "{date} {level_str} {module}{func}.{line} | {msg}\t",
                date = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
                level_str = rec.level().as_short_str(),
                msg = rec.msg(),
                line = rec.line(),
                module = rec.module(),
                func = if rec.function() != "" { format!("::{}", rec.function()) } else { "".into() },
            )?;

            Ok(())
        })
        .build()
        .fuse();

    let drain = LevelFilter { drain, level }.fuse();

    let drain = slog_async::Async::new(drain)
        .chan_size(4096 * 4)
        .overflow_strategy(slog_async::OverflowStrategy::DropAndReport)
        .build()
        .fuse();

    slog::Logger::root(drain, o!())
}

struct LevelFilter<D> {
    drain: D,
    level: ValueMut<Level>,
}

impl<D> Drain for LevelFilter<D>
where
    D: Drain,
{
    type Ok = Option<D::Ok>;
    type Err = Option<D::Err>;

    fn log(
        &self,
        record: &slog::Record,
        values: &slog::OwnedKVList,
    ) -> std::result::Result<Self::Ok, Self::Err> {
        if record.level().is_at_least(*self.level.get()) {
            self.drain.log(record, values).map(Some).map_err(Some)
        } else {
            Ok(None)
        }
    }
}

struct WriteFilter {
    filename: String,
    to: ValueMut<To>,

    file: Option<File>,
    console: Stdout,
}

impl WriteFilter {
    fn new(filename: String, to: ValueMut<To>) -> Self {
        Self { filename, to, file: None, console: std::io::stdout() }
    }

    fn file(&mut self) -> &File {
        if self.file.is_none() {
            self.file = Some(open_file(&self.filename).unwrap());
        }
        self.file.as_ref().unwrap()
    }
}

impl io::Write for WriteFilter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = match self.to.get() {
            To::Console => self.console.write(buf)?,
            To::File => self.file().write(buf)?,
            To::Both => {
                let _ = self.console.write(buf)?;
                self.file().write(buf)?
            }
            To::Off => buf.len(),
        };
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.to.get() {
            To::Console => self.console.flush()?,
            To::File => self.file().flush()?,
            To::Both => {
                self.console.flush()?;
                self.file().flush()?;
            }
            To::Off => {}
        };
        Ok(())
    }
}

fn open_file(filename: &str) -> Result<File> {
    OpenOptions::new()
        .create(true)
        .write(true)
        .append(true)
        .open(filename)
        .map_err(|e| anyhow::Error::msg(format!("logger file config error, filename: {}, {:?}", filename, e)))
}
