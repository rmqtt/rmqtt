use std::ops::{Deref, DerefMut};
use std::str::FromStr;
use std::sync::Arc;

use parking_lot::RwLock;
use serde::de::{self, Deserialize, Deserializer};

use super::ValueMut;

#[derive(Debug, Clone, Deserialize)]
pub struct Log {
    #[serde(default = "Log::to_default")]
    pub to: ValueMut<To>,
    #[serde(default = "Log::level_default")]
    pub level: ValueMut<Level>,
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
    fn to_default() -> ValueMut<To> {
        ValueMut::new(To::Console)
    }
    #[inline]
    fn level_default() -> ValueMut<Level> {
        ValueMut::new(Level { inner: slog::Level::Debug })
    }
    #[inline]
    fn dir_default() -> String {
        "/etc/log/rmqtt".into()
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
        let dir = self.dir.trim_end_matches(|c| c == '/' || c == '\\');
        format!("{}/{}", dir, file)
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
pub struct Level {
    inner: slog::Level,
}

impl Level {
    #[inline]
    pub fn inner(&self) -> slog::Level {
        self.inner
    }
}

impl Deref for Level {
    type Target = slog::Level;
    #[inline]
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for Level {
    #[inline]
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl<'de> Deserialize<'de> for Level {
    #[inline]
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
    {
        let level = String::deserialize(deserializer)?;
        let level = slog::Level::from_str(&level).map_err(|_e| de::Error::missing_field("level"))?;
        Ok(Level { inner: level })
    }
}

#[derive(Debug, Clone)]
pub struct LevelMut {
    inner: Arc<RwLock<slog::Level>>,
}

impl LevelMut {
    #[inline]
    pub fn get(&self) -> slog::Level {
        *self.inner.read()
    }

    #[inline]
    pub fn set(&self, level: slog::Level) {
        *self.inner.write() = level;
    }
}

impl<'de> Deserialize<'de> for LevelMut {
    #[inline]
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: Deserializer<'de>,
    {
        let level = String::deserialize(deserializer)?;
        let level = slog::Level::from_str(&level).map_err(|_e| de::Error::missing_field("level"))?;
        Ok(LevelMut { inner: Arc::new(RwLock::new(level)) })
    }
}
