use serde::de::{self, Deserializer};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct Log {
    #[serde(default = "Log::to_default")]
    pub to: To,
    #[serde(default = "Log::level_default")]
    pub level: Level,
    #[serde(default = "Log::dir_default")]
    pub dir: String,
    #[serde(default = "Log::file_default")]
    pub file: String,
}

impl Default for Log {
    #[inline]
    fn default() -> Self {
        Self {
            to: Self::to_default(),
            level: Self::level_default(),
            dir: Self::dir_default(),
            file: Self::file_default(),
        }
    }
}

impl Log {
    #[inline]
    fn to_default() -> To {
        To::Console
    }
    #[inline]
    fn level_default() -> Level {
        Level::Info
    }
    #[inline]
    fn dir_default() -> String {
        "/var/log/rmqtt".into()
    }
    #[inline]
    fn file_default() -> String {
        "rmqtt.log".into()
    }
    #[inline]
    pub fn filename(&self) -> String {
        let file = &self.file;
        if file.is_empty() {
            return "".into();
        }
        if self.dir.is_empty() {
            return file.to_owned();
        }
        let dir = self.dir.trim_end_matches(['/', '\\']);
        format!("{dir}/{file}")
    }
}

#[derive(Debug, Clone, Copy)]
pub enum To {
    Off,
    File,
    Console,
    Both,
}

impl To {
    #[inline]
    pub fn file(&self) -> bool {
        matches!(self, To::Both | To::File)
    }
    #[inline]
    pub fn console(&self) -> bool {
        matches!(self, To::Both | To::Console)
    }
    #[inline]
    pub fn off(&self) -> bool {
        matches!(self, To::Off)
    }
}

impl<'de> Deserialize<'de> for To {
    #[inline]
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let to = match (String::deserialize(deserializer)?).to_ascii_lowercase().as_str() {
            "off" => To::Off,
            "file" => To::File,
            "console" => To::Console,
            "both" => To::Both,
            _ => To::Both,
        };

        Ok(to)
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Level {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    #[inline]
    pub fn as_str(&self) -> &'static str {
        match self {
            Level::Trace => "trace",
            Level::Debug => "debug",
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Error => "error",
        }
    }
}

impl std::fmt::Display for Level {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for Level {
    #[inline]
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let level = String::deserialize(deserializer)?;
        let level = match level.to_ascii_lowercase().as_str() {
            "trace" => Level::Trace,
            "debug" => Level::Debug,
            "info" => Level::Info,
            "warn" => Level::Warn,
            "error" => Level::Error,
            _ => return Err(de::Error::missing_field("level")),
        };
        Ok(level)
    }
}
