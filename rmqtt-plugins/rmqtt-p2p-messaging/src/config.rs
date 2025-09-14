use serde::de::{self, Deserializer};
use serde::ser::{self};
use serde::{Deserialize, Serialize};

use rmqtt::Result;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PluginConfig {
    /// P2P message routing mode.
    ///
    /// Determines where the `p2p` segment appears in the MQTT topic when sending
    /// peer-to-peer messages.
    ///
    /// Modes:
    /// - `"prefix"`: `p2p/{clientid}/{topic}` — the `p2p` identifier is at the start.
    /// - `"suffix"`: `{topic}/p2p/{clientid}` — the `p2p` identifier is at the end.
    /// - `"both"`: supports both prefix and suffix formats.
    ///
    /// Default: `"prefix"`.
    #[serde(
        default = "PluginConfig::mode_default",
        serialize_with = "PluginConfig::serialize_mode",
        deserialize_with = "PluginConfig::deserialize_mode"
    )]
    pub mode: Mode,
}

impl PluginConfig {
    /// Returns the default P2P mode (`Prefix`).
    #[inline]
    fn mode_default() -> Mode {
        Mode::Prefix
    }

    /// Serializes the `Mode` enum into a string for JSON.
    #[inline]
    fn serialize_mode<S>(enc: &Mode, s: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: ser::Serializer,
    {
        let enc_str = match enc {
            Mode::Prefix => "prefix",
            Mode::Suffix => "suffix",
            Mode::Both => "both",
        };
        enc_str.serialize(s)
    }

    /// Deserializes a string into the `Mode` enum from JSON.
    ///
    /// Only `"prefix"`, `"suffix"`, or `"both"` are valid. Others will return an error.
    #[inline]
    fn deserialize_mode<'de, D>(deserializer: D) -> std::result::Result<Mode, D::Error>
    where
        D: Deserializer<'de>,
    {
        let enc: String = String::deserialize(deserializer)?;
        match enc.as_str() {
            "prefix" => Ok(Mode::Prefix),
            "suffix" => Ok(Mode::Suffix),
            "both" => Ok(Mode::Both),
            _ => Err(de::Error::custom("Invalid mode, only 'prefix', 'suffix', or 'both' are supported.")),
        }
    }

    /// Converts the configuration into a `serde_json::Value`.
    #[inline]
    pub fn to_json(&self) -> Result<serde_json::Value> {
        Ok(serde_json::to_value(self)?)
    }
}

/// P2P message routing mode enum.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Mode {
    /// `p2p` at the beginning of the topic.
    Prefix,
    /// `p2p` at the end of the topic.
    Suffix,
    /// Both prefix and suffix formats are supported.
    Both,
}
