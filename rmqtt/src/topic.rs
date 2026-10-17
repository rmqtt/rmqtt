//! MQTT Topic Handling Module
//!
//! Provides comprehensive implementation for MQTT topic parsing, validation and wildcard pattern
//! matching following MQTT 3.1.1/5.0 specifications. Implements hierarchical topic structure
//! with support for:
//! - Normal levels (`sport/tennis`)
//! - System metadata levels (`$SYS/...`)
//! - Single-level (`+`) and multi-level (`#`) wildcards
//!
//! ## Core Components
//! 1. **Topic Structure**:
//!    - `Level` enum representing different topic segment types:
//!      ```rust
//!      pub enum Level {
//!          Normal(String),       // Standard topic segment
//!          Metadata(String),    // System metadata starting with $
//!          Blank,               // Empty segment (e.g., leading/trailing /)
//!          SingleWildcard,      // + wildcard
//!          MultiWildcard        // # wildcard
//!      }
//!      ```
//!    - `Topic` struct containing ordered collection of Levels
//!
//! 2. **Validation Rules**:
//!    - Metadata levels must start with `$` and cannot contain wildcards
//!    - Multi-level wildcard `#` must be last segment
//!    - Single-level wildcard `+` matches exactly one level
//!    - Prohibits invalid characters (`+`/`#`) in normal levels
//!
//! 3. **Pattern Matching**:
//!    - Implements MQTT wildcard matching semantics:
//!      ```rust,ignore
//!      topic!("sport/+/#").matches_str("sport/tennis/player1") // true
//!      ```
//!    - Supports system metadata isolation (wildcards don't match $ topics)
//!
//! ## Design Features
//! - **Type-Safe Parsing**:
//!   - Implements `FromStr` with detailed error reporting
//!   - Strict validation during parsing
//! - **Zero-Copy Operations**:
//!   - Uses string references for pattern matching
//! - **Extensible Formatting**:
//!   - Implements `Display` trait for human-readable output
//!   - Provides `WriteTopicExt` for efficient byte-level writing
//!
//! ## Usage Example
//! ```rust,ignore
//! let topic: Topic = "sport/+/player1/#".parse().unwrap();
//! assert!(topic.matches_str("sport/tennis/player1/stats"));
//! ```
//!
//! Implements MQTT spec requirements for:
//! - Topic structure validation (§4.7)
//! - Wildcard matching rules (§4.7.1-4.7.3)
//! - System topic isolation (§4.7.3)
//!
//! : MQTT topic structure fundamentals
//! : Wildcard matching semantics

use std::fmt::{self, Write};
use std::{io, ops, str::FromStr};

use serde::{Deserialize, Serialize};

fn is_metadata<T: AsRef<str>>(s: T) -> bool {
    s.as_ref().starts_with('$')
}

/// Errors that may occur during MQTT topic parsing or validation.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum TopicError {
    /// The topic string is malformed or violates MQTT spec rules.
    InvalidTopic(String),
    /// A specific level within the topic is invalid.
    InvalidLevel(String),
}

impl fmt::Display for TopicError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TopicError::InvalidTopic(s) => {
                write!(f, "InvalidTopic({s})")
            }
            TopicError::InvalidLevel(s) => {
                write!(f, "InvalidLevel({s})")
            }
        }
    }
}

/// Represents a single level/segment within an MQTT topic.
///
/// Each level of a topic filter (delimited by `/`) is parsed into one of these variants,
/// representing literal text, wildcards, or empty segments.
#[derive(Debug, Eq, PartialEq, Ord, PartialOrd, Clone, Hash, Serialize, Deserialize)]
pub enum Level {
    Normal(String),
    Metadata(String), // $SYS
    Blank,
    SingleWildcard, // Single level wildcard +
    MultiWildcard,  // Multi-level wildcard #
}

impl Level {
    /// Parse a level from a string reference.
    pub fn parse<T: AsRef<str>>(s: T) -> Result<Level, TopicError> {
        Level::from_str(s.as_ref())
    }

    /// Create a normal (literal) level, rejecting wildcards and `$`-prefixed values.
    pub fn normal<T: AsRef<str>>(s: T) -> Result<Level, TopicError> {
        if s.as_ref().contains(['+', '#']) {
            return Err(TopicError::InvalidLevel(format!(
                "invalid normal level `{}` contains +|#",
                s.as_ref()
            )));
        }

        if s.as_ref().starts_with('$') {
            return Err(TopicError::InvalidLevel(format!(
                "invalid normal level `{}` starts with $",
                s.as_ref()
            )));
        }

        Ok(Level::Normal(String::from(s.as_ref())))
    }

    /// Create a metadata level (starting with `$`), rejecting wildcards.
    pub fn metadata<T: AsRef<str>>(s: T) -> Result<Level, TopicError> {
        if s.as_ref().contains(['+', '#']) {
            return Err(TopicError::InvalidLevel(format!(
                "invalid metadata level `{}` contains +|#",
                s.as_ref()
            )));
        }

        if !s.as_ref().starts_with('$') {
            return Err(TopicError::InvalidLevel(format!(
                "invalid metadata level `{}` not starts with $",
                s.as_ref()
            )));
        }

        Ok(Level::Metadata(String::from(s.as_ref())))
    }

    /// Return the underlying string value, if this is a `Normal` or `Metadata` level.
    #[inline]
    pub fn value(&self) -> Option<&str> {
        match *self {
            Level::Normal(ref s) | Level::Metadata(ref s) => Some(s),
            _ => None,
        }
    }

    /// Return `true` if this is a `Normal` level.
    #[inline]
    pub fn is_normal(&self) -> bool {
        matches!(*self, Level::Normal(_))
    }

    /// Return `true` if this is a `Metadata` level (starting with `$`).
    #[inline]
    pub fn is_metadata(&self) -> bool {
        matches!(*self, Level::Metadata(_))
    }

    /// Validate this level according to MQTT spec rules.
    ///
    /// A normal level must not contain `+`, `#`, or start with `$`.
    /// A metadata level must start with `$` and not contain `+` or `#`.
    /// Wildcard and blank levels are always valid.
    #[inline]
    pub fn is_valid(&self) -> bool {
        match *self {
            Level::Normal(ref s) => !s.starts_with('$') && !s.contains(['+', '#']),
            Level::Metadata(ref s) => s.starts_with('$') && !s.contains(['+', '#']),
            _ => true,
        }
    }
}

macro_rules! matches {
    ($topic:expr, $levels:expr) => {{
        let mut lhs = $topic.0.iter();

        for rhs in $levels {
            match lhs.next() {
                Some(&Level::SingleWildcard) => {
                    if !rhs.match_level(&Level::SingleWildcard) {
                        break;
                    }
                }
                Some(&Level::MultiWildcard) => {
                    return rhs.match_level(&Level::MultiWildcard);
                }
                Some(level) if rhs.match_level(level) => continue,
                _ => return false,
            }
        }

        match lhs.next() {
            Some(&Level::MultiWildcard) => true,
            Some(_) => false,
            None => true,
        }
    }};
}

/// A parsed MQTT topic filter represented as an ordered list of [`Level`] segments.
///
/// Topics are split on `/` delimiters and validated according to MQTT v3.1.1/v5.0
/// specification rules for wildcards and system-level isolation.
#[derive(Debug, Eq, PartialEq, Ord, PartialOrd, Clone, Hash, Serialize, Deserialize)]
pub struct Topic(Vec<Level>);

impl Topic {
    /// Return a reference to the underlying vector of topic levels.
    #[inline]
    pub fn levels(&self) -> &Vec<Level> {
        &self.0
    }

    /// Validate the entire topic according to MQTT specification rules.
    ///
    /// Checks that wildcards are positioned correctly (multi-level `#` must be last)
    /// and each level is valid.
    #[inline]
    pub fn is_valid(&self) -> bool {
        self.0
            .iter()
            .position(|level| !level.is_valid())
            .or_else(|| {
                self.0.iter().enumerate().position(|(pos, level)| match *level {
                    Level::MultiWildcard => pos != self.0.len() - 1,
                    _ => false,
                })
            })
            .is_none()
    }

    /// Check whether this topic filter matches another parsed topic.
    pub fn matches(&self, topic: &Topic) -> bool {
        matches!(self, &topic.0)
    }

    /// Check whether this topic filter matches a topic string.
    ///
    /// The string is split on `/` and matched according to MQTT wildcard rules.
    pub fn matches_str<S: AsRef<str> + ?Sized>(&self, topic: &S) -> bool {
        matches!(self, topic.as_ref().split('/'))
    }

    /// Convert the topic to a string, optionally ignoring the first `n` levels.
    ///
    /// # Example
    /// ```
    /// let topic: rmqtt::topic::Topic = "a/b/c/d".parse().unwrap();
    /// assert_eq!(topic.to_string_skip(2), "c/d");
    ///
    /// let topic: rmqtt::topic::Topic = "sport/tennis/player1/stats".parse().unwrap();
    /// assert_eq!(topic.to_string_skip(0), "sport/tennis/player1/stats");
    /// assert_eq!(topic.to_string_skip(2), "player1/stats");
    /// assert_eq!(topic.to_string_skip(4), "");
    /// assert_eq!(topic.to_string_skip(6), "");
    /// ```
    #[inline]
    pub fn to_string_skip(&self, skip: usize) -> String {
        self.0.iter().skip(skip).map(|lvl| lvl.to_string()).collect::<Vec<_>>().join("/")
    }
}

/// Check whether `topic` can be used as a PUBLISH **topic name**.
///
/// A topic name is not a topic filter: `+` and `#` are wildcards reserved for
/// subscriptions (§4.7.1), and a PUBLISH whose Topic Name carries one is a
/// violation a conforming peer answers by dropping the connection —
/// [MQTT-3.3.2-2]. The name is also a UTF-8 Encoded String, so it must be at
/// least one character long ([MQTT-4.7.3-1]), must not contain U+0000, and must
/// fit in the 65535 bytes its two-byte length prefix can express (§1.5.4).
///
/// Blank levels are *not* rejected: `a//b` and `/` are legal names — only a
/// wholly empty name is illegal.
///
/// ```
/// use rmqtt::topic::is_valid_topic_name;
///
/// assert!(is_valid_topic_name("a/b/c"));
/// assert!(is_valid_topic_name("/"));
/// assert!(!is_valid_topic_name(""));
/// assert!(!is_valid_topic_name("a/#"));
/// assert!(!is_valid_topic_name("a/+/b"));
/// ```
#[inline]
pub fn is_valid_topic_name<T: AsRef<str>>(topic: T) -> bool {
    let topic = topic.as_ref();
    !topic.is_empty() && topic.len() <= u16::MAX as usize && !topic.contains(['+', '#', '\0'])
}

/// The `$share/<ShareName>/<TopicFilter>` prefix of a shared subscription
/// ([MQTT-4.8.2-1]).
const SHARED_SUBSCRIPTION_PREFIX: &str = "$share/";

/// The `$limit/<n>/<TopicFilter>` prefix of a limited subscription (rmqtt
/// extension, see `types::parse_topic_filter`).
const LIMIT_SUBSCRIPTION_PREFIX: &str = "$limit/";

/// The `$exclusive/<TopicFilter>` prefix of an exclusive subscription (rmqtt
/// extension, see `types::parse_topic_filter`).
const EXCLUSIVE_SUBSCRIPTION_PREFIX: &str = "$exclusive/";

/// Check whether `topic` can be used as an MQTT topic **filter** (a
/// subscription pattern).
///
/// This is the counterpart of [`is_valid_topic_name`]: `+` and `#` *are* legal
/// here — and only here — with `#` restricted to the last level (§4.7.1.2,
/// [MQTT-4.7.1-1]). Like a name, a filter is a UTF-8 Encoded String, so it must
/// be at least one character long ([MQTT-4.7.3-1]), must not contain U+0000,
/// and must fit in the 65535 bytes its two-byte length prefix can express
/// (§1.5.4).
///
/// The `$share/<group>/<filter>` form is deliberately *not* special-cased: it
/// parses as an ordinary filter whose first level is the metadata level
/// `$share`. Such a filter only matches a topic *named* `$share/<group>/...`,
/// i.e. not the shared subscription the author meant, so the shared form is
/// recognised by [`is_shared_subscription`] and validated by
/// [`is_valid_subscription_filter`] instead of being folded in here.
///
/// ```
/// use rmqtt::topic::is_valid_topic_filter;
///
/// assert!(is_valid_topic_filter("a/b/#"));
/// assert!(is_valid_topic_filter("+"));
/// assert!(!is_valid_topic_filter(""));
/// assert!(!is_valid_topic_filter("a/#/b"));
/// ```
#[inline]
pub fn is_valid_topic_filter<T: AsRef<str>>(topic: T) -> bool {
    let topic = topic.as_ref();
    !topic.is_empty()
        && topic.len() <= u16::MAX as usize
        && !topic.contains('\0')
        && Topic::from_str(topic).is_ok()
}

/// `true` when `topic` is written in the `$share/<ShareName>/<TopicFilter>`
/// form of a shared subscription ([MQTT-4.8.2-1]).
///
/// Only the prefix is examined — whether the group and the filter are usable is
/// what [`is_valid_subscription_filter`] decides.
#[inline]
pub fn is_shared_subscription<T: AsRef<str>>(topic: T) -> bool {
    topic.as_ref().starts_with(SHARED_SUBSCRIPTION_PREFIX)
}

/// `true` when `topic` is written in the `$limit/<n>/<TopicFilter>` form of a
/// limited subscription (rmqtt extension).
///
/// Only the prefix is examined — whether `<n>` is a number and the filter behind
/// it is usable is what `types::parse_topic_filter` decides.
#[inline]
pub fn is_limit_subscription<T: AsRef<str>>(topic: T) -> bool {
    topic.as_ref().starts_with(LIMIT_SUBSCRIPTION_PREFIX)
}

/// `true` when `topic` is written in the `$exclusive/<TopicFilter>` form of an
/// exclusive subscription (rmqtt extension).
#[inline]
pub fn is_exclusive_subscription<T: AsRef<str>>(topic: T) -> bool {
    topic.as_ref().starts_with(EXCLUSIVE_SUBSCRIPTION_PREFIX)
}

/// Check whether `topic` is a legal filter to put in a SUBSCRIBE.
///
/// Accepts an ordinary filter, or the shared form
/// `$share/<ShareName>/<TopicFilter>` where the filter behind the ShareName is
/// itself legal ([MQTT-4.8.2-1], [MQTT-4.8.2-2]).
///
/// An empty ShareName (`$share//a`) is accepted, because
/// `types::parse_topic_filter` accepts it: [MQTT-4.8.2-1] requires at least one
/// character, but the broker has always taken an empty group as it stands, and a
/// predicate used to decide whether a *peer* can be subscribed must not be
/// stricter than the parser it is describing. Tightening both sides is a
/// protocol-facing change of its own; until then this predicate reports what the
/// broker does.
///
/// ```
/// use rmqtt::topic::is_valid_subscription_filter;
///
/// assert!(is_valid_subscription_filter("a/b/#"));
/// assert!(is_valid_subscription_filter("$share/g/a/b/#"));
/// assert!(is_valid_subscription_filter("$share//a/b"));
/// assert!(!is_valid_subscription_filter("$share/g"));
/// assert!(!is_valid_subscription_filter("$share/+/a"));
/// ```
#[inline]
pub fn is_valid_subscription_filter<T: AsRef<str>>(topic: T) -> bool {
    let topic = topic.as_ref();
    match topic.strip_prefix(SHARED_SUBSCRIPTION_PREFIX) {
        None => is_valid_topic_filter(topic),
        Some(rest) => match rest.split_once('/') {
            Some((share_name, filter)) => {
                !share_name.contains(['+', '#', '\0']) && is_valid_topic_filter(filter)
            }
            // `$share/` or `$share/<group>` with no filter part at all.
            None => false,
        },
    }
}

impl From<&[Level]> for Topic {
    fn from(s: &[Level]) -> Self {
        let mut v = vec![];

        v.extend_from_slice(s);

        Topic(v)
    }
}

impl From<Vec<Level>> for Topic {
    fn from(v: Vec<Level>) -> Self {
        Topic(v)
    }
}

impl From<Topic> for Vec<Level> {
    fn from(t: Topic) -> Self {
        t.0
    }
}

impl ops::Deref for Topic {
    type Target = Vec<Level>;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl ops::DerefMut for Topic {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

#[macro_export]
macro_rules! topic {
    ($s:expr) => {
        $s.parse::<Topic>()
    };
}

pub(crate) trait MatchLevel {
    fn match_level(&self, level: &Level) -> bool;
}

impl MatchLevel for Level {
    fn match_level(&self, level: &Level) -> bool {
        match *level {
            Level::Normal(ref lhs) => {
                if let Level::Normal(ref rhs) = *self {
                    lhs == rhs
                } else {
                    false
                }
            }
            Level::Metadata(ref lhs) => {
                if let Level::Metadata(ref rhs) = *self {
                    lhs == rhs
                } else {
                    false
                }
            }
            Level::Blank => true,
            Level::SingleWildcard | Level::MultiWildcard => !self.is_metadata(),
        }
    }
}

impl<T: AsRef<str>> MatchLevel for T {
    fn match_level(&self, level: &Level) -> bool {
        match *level {
            Level::Normal(ref lhs) => !is_metadata(self) && lhs == self.as_ref(),
            Level::Metadata(ref lhs) => is_metadata(self) && lhs == self.as_ref(),
            Level::Blank => self.as_ref().is_empty(),
            Level::SingleWildcard | Level::MultiWildcard => !is_metadata(self),
        }
    }
}

impl FromStr for Level {
    type Err = TopicError;

    #[inline]
    fn from_str(s: &str) -> Result<Self, TopicError> {
        match s {
            "+" => Ok(Level::SingleWildcard),
            "#" => Ok(Level::MultiWildcard),
            "" => Ok(Level::Blank),
            _ => {
                if s.contains(['+', '#']) {
                    Err(TopicError::InvalidLevel(format!("invalid level `{s}` contains +|#")))
                } else if is_metadata(s) {
                    Ok(Level::Metadata(String::from(s)))
                } else {
                    Ok(Level::Normal(String::from(s)))
                }
            }
        }
    }
}

impl FromStr for Topic {
    type Err = TopicError;

    #[inline]
    fn from_str(s: &str) -> Result<Self, TopicError> {
        s.split('/').map(Level::from_str).collect::<Result<Vec<_>, TopicError>>().map(Topic).and_then(
            |topic| {
                if topic.is_valid() {
                    Ok(topic)
                } else {
                    Err(TopicError::InvalidTopic(format!("invalid topic `{s}`")))
                }
            },
        )
    }
}

impl fmt::Display for Level {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Level::Normal(ref s) | Level::Metadata(ref s) => f.write_str(s.as_str()),
            Level::Blank => Ok(()),
            Level::SingleWildcard => f.write_char('+'),
            Level::MultiWildcard => f.write_char('#'),
        }
    }
}

impl fmt::Display for Topic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;

        for level in &self.0 {
            if first {
                first = false;
            } else {
                f.write_char('/')?;
            }

            level.fmt(f)?;
        }

        Ok(())
    }
}

#[allow(dead_code)]
pub(crate) trait WriteTopicExt: io::Write {
    fn write_level(&mut self, level: &Level) -> io::Result<usize> {
        match *level {
            Level::Normal(ref s) | Level::Metadata(ref s) => self.write(s.as_str().as_bytes()),
            Level::Blank => Ok(0),
            Level::SingleWildcard => self.write(b"+"),
            Level::MultiWildcard => self.write(b"#"),
        }
    }

    fn write_topic(&mut self, topic: &Topic) -> io::Result<usize> {
        let mut n = 0;
        let mut first = true;

        for level in topic.levels() {
            if first {
                first = false;
            } else {
                n += self.write(b"/")?;
            }

            n += self.write_level(level)?;
        }

        Ok(n)
    }
}

impl<W: io::Write + ?Sized> WriteTopicExt for W {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_level() {
        assert!(Level::normal("sport").unwrap().is_normal());
        assert!(Level::metadata("$SYS").unwrap().is_metadata());

        assert_eq!(Level::normal("sport").unwrap().value(), Some("sport"));
        assert_eq!(Level::metadata("$SYS").unwrap().value(), Some("$SYS"));

        assert_eq!(Level::normal("sport").unwrap(), "sport".parse().expect(""));
        assert_eq!(Level::metadata("$SYS").unwrap(), "$SYS".parse().expect(""));

        assert!(Level::Normal(String::from("sport")).is_valid());
        assert!(Level::Metadata(String::from("$SYS")).is_valid());

        assert!(!Level::Normal(String::from("$sport")).is_valid());
        assert!(!Level::Metadata(String::from("SYS")).is_valid());

        assert!(!Level::Normal(String::from("sport#")).is_valid());
        assert!(!Level::Metadata(String::from("SYS+")).is_valid());
    }

    #[test]
    fn test_is_valid_topic_name() {
        for name in ["a", "a/b/c", "/", "a//b", "$SYS/broker/load", "x/", "sport.tennis"] {
            assert!(is_valid_topic_name(name), "`{name}` must be a valid topic name");
        }

        for name in ["", "#", "+", "a/#", "a/+/b", "a/b/+", "sport+", "a#b", "a\0b"] {
            assert!(!is_valid_topic_name(name), "`{name}` must not be a valid topic name");
        }

        assert!(is_valid_topic_name("a".repeat(u16::MAX as usize)));
        assert!(!is_valid_topic_name("a".repeat(u16::MAX as usize + 1)));
    }

    #[test]
    fn test_is_valid_topic_filter() {
        for filter in ["a", "#", "+", "a/b/c", "a/#", "a/+/b", "+/+", "/", "a//b", "$SYS/#", "/+", "x/"] {
            assert!(is_valid_topic_filter(filter), "`{filter}` must be a valid topic filter");
        }

        for filter in ["", "a/#/b", "#/a", "a/#/", "a#", "#a", "sport+", "a/+/b#", "a\0b"] {
            assert!(!is_valid_topic_filter(filter), "`{filter}` must not be a valid topic filter");
        }

        assert!(is_valid_topic_filter("a".repeat(u16::MAX as usize)));
        assert!(!is_valid_topic_filter("a".repeat(u16::MAX as usize + 1)));

        // The shared form is a plain filter as far as this predicate is
        // concerned: it parses, and it matches a topic literally named
        // `$share/...`. That is why callers that need to spot the shared form
        // use `is_shared_subscription` rather than relying on this predicate.
        assert!(is_valid_topic_filter("$share/g/a/b/#"));
        assert!(is_valid_topic_filter("$share//a"));
    }

    #[test]
    fn test_is_shared_subscription() {
        assert!(is_shared_subscription("$share/g/a"));
        assert!(is_shared_subscription("$share/"));
        assert!(is_shared_subscription("$share//a"));

        for filter in ["", "$share", "share/g/a", "a/$share/g"] {
            assert!(!is_shared_subscription(filter), "`{filter}` must not look shared");
        }
    }

    #[test]
    fn test_is_limit_and_exclusive_subscription() {
        for filter in ["$limit/10/a/#", "$limit//a"] {
            assert!(is_limit_subscription(filter), "`{filter}` must look limited");
            assert!(!is_exclusive_subscription(filter), "`{filter}` must not look exclusive");
        }

        for filter in ["$exclusive/a/#", "$exclusive/"] {
            assert!(is_exclusive_subscription(filter), "`{filter}` must look exclusive");
            assert!(!is_limit_subscription(filter), "`{filter}` must not look limited");
        }

        for filter in ["", "$limit", "$exclusive", "limit/10/a", "a/$limit/10/b"] {
            assert!(
                !is_limit_subscription(filter) && !is_exclusive_subscription(filter),
                "`{filter}` must look neither limited nor exclusive"
            );
        }
    }

    #[test]
    fn test_is_valid_subscription_filter() {
        for filter in [
            "a",
            "#",
            "+",
            "a/b/#",
            "$share/g/a",
            "$share/g/a/b/#",
            "$share/g/#",
            "$share/g/+",
            // An empty ShareName violates [MQTT-4.8.2-1], but `parse_topic_filter`
            // accepts it and this predicate must agree with the parser.
            "$share//a",
            "$share//a/b/#",
        ] {
            assert!(
                is_valid_subscription_filter(filter),
                "`{filter}` must be a valid filter to subscribe with"
            );
        }

        for filter in [
            "",
            "a/#/b",
            "a#",
            // A shared subscription must carry a group and a filter.
            "$share/",
            "$share/g",
            "$share/g/",
            "$share/+/a",
            "$share/#/a",
            // …and the filter behind the group must itself be legal.
            "$share/g/a/#/b",
            "$share/g/a#",
        ] {
            assert!(
                !is_valid_subscription_filter(filter),
                "`{filter}` must not be a valid filter to subscribe with"
            );
        }
    }

    #[test]
    fn test_valid_topic() {
        assert!(Topic(vec![
            Level::normal("sport").unwrap(),
            Level::normal("tennis").unwrap(),
            Level::normal("player1").unwrap()
        ])
        .is_valid());

        assert!(Topic(vec![
            Level::normal("sport").unwrap(),
            Level::normal("tennis").unwrap(),
            Level::MultiWildcard
        ])
        .is_valid());
        assert!(Topic(vec![
            Level::metadata("$SYS").unwrap(),
            Level::normal("tennis").unwrap(),
            Level::MultiWildcard
        ])
        .is_valid());

        assert!(Topic(vec![
            Level::normal("sport").unwrap(),
            Level::SingleWildcard,
            Level::normal("player1").unwrap()
        ])
        .is_valid());

        assert!(!Topic(vec![
            Level::normal("sport").unwrap(),
            Level::MultiWildcard,
            Level::normal("player1").unwrap()
        ])
        .is_valid());
        assert!(Topic(vec![
            Level::normal("sport").unwrap(),
            Level::metadata("$SYS").unwrap(),
            Level::normal("player1").unwrap()
        ])
        .is_valid());
    }

    #[test]
    fn test_parse_topic() {
        assert!(topic!("sport/tennis/player1").unwrap().matches(&Topic::from(vec![
            Level::normal("sport").unwrap(),
            Level::normal("tennis").unwrap(),
            Level::normal("player1").unwrap()
        ])));

        assert!(topic!("").unwrap().matches(&Topic(vec![Level::Blank])));
        assert!(topic!("/finance")
            .unwrap()
            .matches(&Topic::from(vec![Level::Blank, Level::normal("finance").unwrap()])));

        assert!(topic!("$SYS").unwrap().matches(&Topic::from(vec![Level::metadata("$SYS").unwrap()])));
        assert!("sport/$SYS".parse::<Topic>().is_ok());
        assert!("valetudo/jimmy/$state".parse::<Topic>().is_ok());
    }

    #[test]
    fn test_multi_wildcard_topic() {
        assert!(topic!("sport/tennis/#").unwrap().matches(&Topic::from(vec![
            Level::normal("sport").unwrap(),
            Level::normal("tennis").unwrap(),
            Level::MultiWildcard
        ])));

        assert!(topic!("#").unwrap().matches(&Topic::from(vec![Level::MultiWildcard])));
        assert!("sport/tennis#".parse::<Topic>().is_err());
        assert!("sport/tennis/#/ranking".parse::<Topic>().is_err());
    }

    #[test]
    fn test_single_wildcard_topic() {
        assert!(topic!("+").unwrap().matches(&Topic::from(vec![Level::SingleWildcard])));

        assert!(topic!("+/tennis/#").unwrap().matches(&Topic::from(vec![
            Level::SingleWildcard,
            Level::normal("tennis").unwrap(),
            Level::MultiWildcard
        ])));

        assert!(topic!("sport/+/player1").unwrap().matches(&Topic::from(vec![
            Level::normal("sport").unwrap(),
            Level::SingleWildcard,
            Level::normal("player1").unwrap()
        ])));

        assert!("sport+".parse::<Topic>().is_err());
    }

    #[test]
    fn test_write_topic() {
        let mut v = vec![];
        let t = vec![Level::SingleWildcard, Level::normal("tennis").unwrap(), Level::MultiWildcard].into();

        assert_eq!(v.write_topic(&t).expect(""), 10);
        assert_eq!(v, b"+/tennis/#");

        assert_eq!(format!("{t}"), "+/tennis/#");
        assert_eq!(t.to_string(), "+/tennis/#");
    }

    #[test]
    fn test_matches() {
        assert!("test".match_level(&Level::normal("test").unwrap()));
        assert!("$SYS".match_level(&Level::metadata("$SYS").unwrap()));

        let t: Topic = "sport/tennis/player1/#".parse().expect("");

        assert!(t.matches_str("sport/tennis/player1"));
        assert!(t.matches_str("sport/tennis/player1/ranking"));
        assert!(t.matches_str("sport/tennis/player1/score/wimbledon"));

        assert!(Topic::from_str("sport/#").expect("").matches_str("sport"));

        let t: Topic = "sport/tennis/+".parse().expect("");

        assert!(t.matches_str("sport/tennis/player1"));
        assert!(t.matches_str("sport/tennis/player2"));
        assert!(!t.matches_str("sport/tennis/player1/ranking"));

        let t: Topic = "sport/+".parse().expect("");

        assert!(!t.matches_str("sport"));
        assert!(t.matches_str("sport/"));

        assert!(Topic::from_str("+/+").expect("").matches_str("/finance"));
        assert!(Topic::from_str("/+").expect("").matches_str("/finance"));
        assert!(!Topic::from_str("+").expect("").matches_str("/finance"));

        assert!(!Topic::from_str("#").expect("").matches_str("$SYS"));
        assert!(!Topic::from_str("+/monitor/Clients").expect("").matches_str("$SYS/monitor/Clients"));
        assert!(Topic::from_str("$SYS/#").expect("").matches_str("$SYS/"));
        assert!(Topic::from_str("$SYS/monitor/+").expect("").matches_str("$SYS/monitor/Clients"));
    }

    /// [MQTT-4.7.1-1]: a level may be empty, so a blank level in the middle or at
    /// either end is a legal topic name and must survive a round trip. Only the
    /// zero-length *name* is refused, and the codec does that on the wire
    /// (`rmqtt_codec::v3::decode`), not this parser.
    #[test]
    fn test_blank_levels() {
        for name in ["a//b", "a/", "/a", "/"] {
            let topic = Topic::from_str(name).expect("blank levels are legal");
            assert!(topic.is_valid(), "`{name}` must stay valid");
            assert_eq!(topic.to_string(), name, "`{name}` must round-trip");
        }

        let topic = Topic::from_str("a//b").unwrap();
        assert_eq!(topic.levels().len(), 3);
        assert_eq!(topic.levels()[1], Level::Blank);
    }

    /// `#` is legal only as the last level, and never as part of a level.
    #[test]
    fn test_number_sign_position() {
        for name in ["#", "a/#", "a/b/#"] {
            assert!(Topic::from_str(name).is_ok(), "`{name}` must be accepted");
        }
        for name in ["#/a", "a/#/b", "a/#/", "a#", "#a"] {
            assert!(Topic::from_str(name).is_err(), "`{name}` must be rejected");
        }
    }

    /// A `$`-prefixed level is legal in *any* position (GitHub issue #537):
    /// [MQTT-4.7.2-1] reserves only topic names that *begin* with `$`.
    #[test]
    fn test_dollar_level_is_valid_in_any_position() {
        for name in ["$SYS", "$SYS/a", "a/$state", "a/b/$state", "homie/dev/$state", "a/$b/+", "$SYS/#"] {
            let topic = Topic::from_str(name).unwrap_or_else(|e| panic!("`{name}` must be accepted: {e}"));
            assert!(topic.is_valid(), "`{name}` must stay valid");
            assert_eq!(topic.to_string(), name);
        }
    }

    /// A metadata level matches only a metadata level: the same text without the
    /// `$` is a different topic.
    #[test]
    fn test_metadata_level_matches_only_metadata() {
        assert!(Topic::from_str("a/$b").unwrap().matches_str("a/$b"));
        assert!(!Topic::from_str("a/$b").unwrap().matches_str("a/b"));
        assert!(!Topic::from_str("a/b").unwrap().matches_str("a/$b"));
        assert!(Topic::from_str("$SYS/a").unwrap().matches_str("$SYS/a"));
    }

    /// The `$` isolation of [MQTT-4.7.2-1] is about the *first* level, and `#`
    /// honours it. An explicit `$` level is how a client reaches a system topic.
    #[test]
    fn test_root_wildcard_does_not_match_dollar_name() {
        assert!(!Topic::from_str("#").unwrap().matches_str("$SYS"));
        assert!(!Topic::from_str("#").unwrap().matches_str("$SYS/a"));
        assert!(!Topic::from_str("+/a").unwrap().matches_str("$SYS/a"));
        assert!(!Topic::from_str("+/+").unwrap().matches_str("$SYS/a"));
        assert!(Topic::from_str("$SYS/#").unwrap().matches_str("$SYS/a"));
        assert!(Topic::from_str("$SYS/+").unwrap().matches_str("$SYS/a"));
        assert!(Topic::from_str("$SYS").unwrap().matches_str("$SYS"));
    }

    /// A `$` level *below* the root is an ordinary level for the wildcards above
    /// it, which is what the Homie convention relies on.
    #[test]
    fn test_hash_reaches_a_dollar_level_below_the_root() {
        assert!(Topic::from_str("#").unwrap().matches_str("a/$b"));
        assert!(Topic::from_str("#").unwrap().matches_str("homie/dev/$state"));
        assert!(Topic::from_str("a/+/$b").unwrap().matches_str("a/x/$b"));
    }

    /// ⚠️ Recorded deviation from [MQTT-4.7.2-1]. The `matches!` macro `break`s out
    /// of its loop when a `+` meets a `$` level, and the tail of the macro then
    /// inspects what is left of the *filter* only — so a `+` that happens to be the
    /// filter's last level reports a match anyway. `TopicTree` (`trie.rs`) and the
    /// retain tree (`retain.rs`) refuse these, so routing is unaffected;
    /// `Topic::matches_str` backs the ACL matcher, where this is a fail-open. The
    /// values are today's behaviour, asserted so that the inconsistency stays
    /// visible; narrowing it to the first level is a follow-up, not a silent change.
    #[test]
    fn test_trailing_plus_is_swallowed_against_dollar_name() {
        assert!(Topic::from_str("+").unwrap().matches_str("$SYS"));
        assert!(Topic::from_str("+").unwrap().matches_str("$SYS/a"));
        assert!(Topic::from_str("a/+").unwrap().matches_str("a/$b"));
        // …but not once the `+` has a level after it, and never for `#`.
        assert!(!Topic::from_str("+/a").unwrap().matches_str("$SYS/a"));
        assert!(!Topic::from_str("a/+/b").unwrap().matches_str("a/$x/b"));
        assert!(!Topic::from_str("a/#").unwrap().matches_str("a/$b"));
    }

    /// A `$` *inside* a level is an ordinary character: only a level that *begins*
    /// with `$` is a metadata level. That single rule is what separates
    /// `homie/$dev/$state` (two metadata levels) from `homie/dev$/sta$te` (none), and
    /// it is decided by `Level::from_str`, so the parsed levels are asserted here
    /// rather than the strings.
    #[test]
    fn test_dollar_inside_a_level_is_ordinary() {
        use Level::{Metadata, Normal};

        let cases: [(&str, Vec<Level>); 5] = [
            (
                "homie/$dev/$state",
                vec![Normal("homie".into()), Metadata("$dev".into()), Metadata("$state".into())],
            ),
            (
                "homie/dev$/$state",
                vec![Normal("homie".into()), Normal("dev$".into()), Metadata("$state".into())],
            ),
            ("homie/dev/sta$te", vec![Normal("homie".into()), Normal("dev".into()), Normal("sta$te".into())]),
            ("homie/d$ev/state", vec![Normal("homie".into()), Normal("d$ev".into()), Normal("state".into())]),
            ("homie/$/state", vec![Normal("homie".into()), Metadata("$".into()), Normal("state".into())]),
        ];

        for (name, levels) in cases {
            let topic = Topic::from_str(name).unwrap_or_else(|e| panic!("`{name}` must be accepted: {e}"));
            assert!(topic.is_valid(), "`{name}` must stay valid");
            assert_eq!(topic.levels().as_slice(), levels.as_slice(), "`{name}` parsed the wrong levels");
        }

        // The constructors draw the same line: a non-leading `$` is not metadata, and
        // a metadata level may not carry a wildcard — including right after the `$`.
        assert!(Level::normal("dev$").is_ok());
        assert!(Level::metadata("dev$").is_err());
        assert!(Level::metadata("$+").is_err());
        assert!(!Level::Metadata(String::from("dev$")).is_valid());
    }

    /// The wildcards around those levels. A `$`-prefixed level is not matched by a
    /// `+` standing in for it — unless the `+` is the filter's last level, which is
    /// the deviation recorded above — while `#` placed *above* the level reaches the
    /// whole name, and a `$` inside a level is matched like any other text.
    #[test]
    fn test_wildcards_around_a_dollar_level() {
        // Ordinary levels, `$` inside or not.
        assert!(Topic::from_str("homie/+/+").unwrap().matches_str("homie/dev$/sta$te"));
        assert!(Topic::from_str("homie/+/+").unwrap().matches_str("homie/d$ev/state"));
        assert!(Topic::from_str("homie/+").unwrap().matches_str("homie/dev$"));

        // A `$`-prefixed level is only reachable by an explicit term, as long as the
        // filter has a level left after the `+` …
        assert!(!Topic::from_str("homie/+/+").unwrap().matches_str("homie/$dev/$state"));
        assert!(!Topic::from_str("homie/+/$state").unwrap().matches_str("homie/$dev/$state"));
        assert!(!Topic::from_str("homie/#").unwrap().matches_str("homie/$dev/$state"));
        // … and the recorded deviation is what happens when it has none.
        assert!(Topic::from_str("homie/+").unwrap().matches_str("homie/$dev"));

        // `#` above the level covers the name, and the explicit form matches it.
        assert!(Topic::from_str("#").unwrap().matches_str("homie/$dev/$state"));
        assert!(Topic::from_str("homie/$dev/$state").unwrap().matches_str("homie/$dev/$state"));

        // The same text with and without the `$` is two different levels.
        assert!(Topic::from_str("homie/dev$").unwrap().matches_str("homie/dev$"));
        assert!(!Topic::from_str("homie/dev$").unwrap().matches_str("homie/dev"));
    }
}
