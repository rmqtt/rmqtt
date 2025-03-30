use std::fmt::{self, Write};
use std::{io, ops, str::FromStr};

use serde::{Deserialize, Serialize};

fn is_metadata<T: AsRef<str>>(s: T) -> bool {
    s.as_ref().starts_with('$')
}

#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum TopicError {
    InvalidTopic(String),
    InvalidLevel(String),
}

impl fmt::Display for TopicError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TopicError::InvalidTopic(s) => {
                write!(f, "InvalidTopic({})", s)
            }
            TopicError::InvalidLevel(s) => {
                write!(f, "InvalidLevel({})", s)
            }
        }
    }
}

#[derive(Debug, Eq, PartialEq, Ord, PartialOrd, Clone, Hash, Serialize, Deserialize)]
pub enum Level {
    Normal(String),
    Metadata(String), // $SYS
    Blank,
    SingleWildcard, // Single level wildcard +
    MultiWildcard,  // Multi-level wildcard #
}

impl Level {
    pub fn parse<T: AsRef<str>>(s: T) -> Result<Level, TopicError> {
        Level::from_str(s.as_ref())
    }

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

    #[inline]
    pub fn value(&self) -> Option<&str> {
        match *self {
            Level::Normal(ref s) | Level::Metadata(ref s) => Some(s),
            _ => None,
        }
    }

    #[inline]
    pub fn is_normal(&self) -> bool {
        matches!(*self, Level::Normal(_))
    }

    #[inline]
    pub fn is_metadata(&self) -> bool {
        matches!(*self, Level::Metadata(_))
    }

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

#[derive(Debug, Eq, PartialEq, Ord, PartialOrd, Clone, Hash, Serialize, Deserialize)]
pub struct Topic(Vec<Level>);

impl Topic {
    #[inline]
    pub fn levels(&self) -> &Vec<Level> {
        &self.0
    }

    #[inline]
    pub fn is_valid(&self) -> bool {
        self.0
            .iter()
            .position(|level| !level.is_valid())
            .or_else(|| {
                self.0.iter().enumerate().position(|(pos, level)| match *level {
                    Level::MultiWildcard => pos != self.0.len() - 1,
                    Level::Metadata(_) => pos != 0,
                    _ => false,
                })
            })
            .is_none()
    }

    pub fn matches(&self, topic: &Topic) -> bool {
        matches!(self, &topic.0)
    }

    pub fn matches_str<S: AsRef<str> + ?Sized>(&self, topic: &S) -> bool {
        matches!(self, topic.as_ref().split('/'))
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
                    Err(TopicError::InvalidLevel(format!("invalid level `{}` contains +|#", s)))
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
                    Err(TopicError::InvalidTopic(format!("invalid topic `{}`", s)))
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
        assert!(!Topic(vec![
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
        assert!("sport/$SYS".parse::<Topic>().is_err());
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

        assert_eq!(format!("{}", t), "+/tennis/#");
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
        assert!(Topic::from_str(&"$SYS/#").expect("").matches_str("$SYS/"));
        assert!(Topic::from_str("$SYS/monitor/+").expect("").matches_str("$SYS/monitor/Clients"));
    }
}
