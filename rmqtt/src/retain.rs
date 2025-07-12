//! MQTT Retained Message Storage Implementation
//!
//! Provides hierarchical storage and efficient retrieval of MQTT retained messages with:
//! 1. **Topic Pattern Matching**:
//!    - Multi-level wildcard (#) and single-level wildcard (+) support
//!    - Metadata-aware filtering for system topics (starting with $)
//!
//! 2. **Message Management**:
//!    - Time-based expiration with automatic cleanup
//!    - Atomic counters for message statistics
//!    - Thread-safe operations using async RwLock
//!
//! 3. **Core Components**:
//!    - `RetainTree`: Trie-like structure for O(log n) topic operations
//!    - `TimedValue`: Wrapper for message expiration tracking
//!    - `DefaultRetainStorage`: Production implementation with plugin integration
//!
//! ## Design Highlights
//! - **Hierarchical Storage**:
//!   ```text
//!   Root
//!   ├── iot
//!   │   └── b
//!   │       ├── x (value=1)
//!   │       ├── y (value=2)
//!   │       └── z (value=3)
//!   └── x
//!       └── y
//!           └── z (value=4)
//!   ```
//!   Enables efficient wildcard pattern matching through trie traversal
//!
//! - **Expiration Mechanism**:
//!   ```rust,ignore
//!   TimedValue::new(retain, timeout) // Wraps message with TTL
//!   remove_expired_messages() // Scheduled cleanup task
//!   ```
//!   Automatically evicts stale messages using duration-based tracking
//!
//! - **Plugin Architecture**:
//!   ```rust,ignore
//!   #[async_trait]
//!   impl RetainStorage for DefaultRetainStorage {
//!       async fn set(...) { /* delegates to rmqtt-retainer plugin */ }
//!   }
//!   ```
//!   Enables extension through external plugins while maintaining core logic
//!
//! ## Key Operations
//! | Method                | Complexity | Description                     |
//! |-----------------------|------------|---------------------------------|
//! | `insert()`            | O(k)       | k = topic depth levels          |
//! | `matches()`           | O(k+m)     | m = matching branches           |
//! | `remove_expired()`    | O(n)       | n = total stored messages       |
//! | `retain()`            | O(n)       | Conditional bulk removal        |
//!
//! ## Usage Note
//! The base implementation intentionally delegates to `rmqtt-retainer` plugin for:
//! - Distributed storage support
//! - Enhanced persistence mechanisms
//! - Cluster-wide message synchronization
//!
//! See `rmqtt-retainer` documentation for production deployment recommendations

use std::str::FromStr;
use std::time::Duration;

use async_trait::async_trait;
use tokio::sync::RwLock;

use crate::topic::{Level, Topic};
use crate::types::{HashMap, Retain, TimedValue, TopicFilter, TopicName};
use crate::utils::{Counter, StatsMergeMode};
use crate::Result;

#[async_trait]
pub trait RetainStorage: Sync + Send {
    ///Whether retain is supported
    #[inline]
    fn enable(&self) -> bool {
        false
    }

    ///topic - concrete topic
    async fn set(&self, topic: &TopicName, retain: Retain, expiry_interval: Option<Duration>) -> Result<()>;

    ///topic_filter - Topic filter
    async fn get(&self, topic_filter: &TopicFilter) -> Result<Vec<(TopicName, Retain)>>;

    async fn count(&self) -> isize;

    async fn max(&self) -> isize;

    #[inline]
    fn stats_merge_mode(&self) -> StatsMergeMode {
        StatsMergeMode::None
    }
}

pub struct DefaultRetainStorage {
    pub messages: RwLock<RetainTree<TimedValue<Retain>>>,
    retaineds: Counter,
}

impl Default for DefaultRetainStorage {
    fn default() -> Self {
        Self::new()
    }
}

impl DefaultRetainStorage {
    #[inline]
    pub fn new() -> DefaultRetainStorage {
        Self { messages: RwLock::new(RetainTree::default()), retaineds: Counter::new() }
    }

    #[inline]
    pub async fn remove_expired_messages(&self) -> usize {
        let mut messages = self.messages.write().await;
        messages.retain(usize::MAX, |tv| {
            if tv.is_expired() {
                self.retaineds.dec();
                false
            } else {
                true
            }
        })
    }

    #[inline]
    pub async fn set_with_timeout(
        &self,
        topic: &TopicName,
        retain: Retain,
        timeout: Option<Duration>,
    ) -> Result<()> {
        let topic = Topic::from_str(topic)?;
        let mut messages = self.messages.write().await;
        let old = messages.remove(&topic);
        if !retain.publish.payload.is_empty() {
            messages.insert(&topic, TimedValue::new(retain, timeout));
            if old.is_none() {
                self.retaineds.inc();
            }
        } else if old.is_some() {
            self.retaineds.dec();
        }
        Ok(())
    }

    #[inline]
    pub async fn get_message(&self, topic_filter: &TopicFilter) -> Result<Vec<(TopicName, Retain)>> {
        let topic = Topic::from_str(topic_filter)?;
        let retains = self
            .messages
            .read()
            .await
            .matches(&topic)
            .drain(..)
            .filter_map(|(t, r)| {
                if r.is_expired() {
                    None
                } else {
                    Some((TopicName::from(t.to_string()), r.into_value()))
                }
            })
            .collect::<Vec<(TopicName, Retain)>>();
        Ok(retains)
    }
}

#[async_trait]
impl RetainStorage for DefaultRetainStorage {
    #[inline]
    async fn set(
        &self,
        _topic: &TopicName,
        _retain: Retain,
        _expiry_interval: Option<Duration>,
    ) -> Result<()> {
        log::warn!("Please use the \"rmqtt-retainer\" plugin as the main program no longer supports retain messages.");
        Ok(())
    }

    #[inline]
    async fn get(&self, _topic_filter: &TopicFilter) -> Result<Vec<(TopicName, Retain)>> {
        log::warn!("Please use the \"rmqtt-retainer\" plugin as the main program no longer supports retain messages.");
        Ok(Vec::new())
    }

    #[inline]
    async fn count(&self) -> isize {
        self.retaineds.count()
    }

    #[inline]
    async fn max(&self) -> isize {
        self.retaineds.max()
    }
}

pub type RetainTree<V> = Node<V>;

pub struct Node<V> {
    value: Option<V>,
    branches: HashMap<Level, Node<V>>,
}

impl<V> Default for Node<V> {
    #[inline]
    fn default() -> Node<V> {
        Self { value: None, branches: HashMap::default() }
    }
}

impl<V> Node<V>
where
    V: std::fmt::Debug + Clone,
{
    #[inline]
    pub fn insert(&mut self, topic: &Topic, value: V) {
        let mut path = topic.levels().clone();
        path.reverse();
        self._insert(path, value);
    }

    #[inline]
    fn _insert(&mut self, mut path: Vec<Level>, value: V) {
        if let Some(first) = path.pop() {
            self.branches.entry(first).or_default()._insert(path, value)
        } else {
            self.value.replace(value);
        }
    }

    #[inline]
    pub fn remove(&mut self, topic: &Topic) -> Option<V> {
        self._remove(topic.levels().as_ref())
    }

    #[inline]
    fn _remove(&mut self, path: &[Level]) -> Option<V> {
        if path.is_empty() {
            self.value.take()
        } else {
            let t = &path[0];
            if let Some(x) = self.branches.get_mut(t) {
                let res = x._remove(&path[1..]);
                if x.value.is_none() && x.branches.is_empty() {
                    self.branches.remove(t);
                }
                res
            } else {
                None
            }
        }
    }

    //remove all pairs `v` for which `f(&mut v)` returns `false`.
    #[inline]
    pub fn retain<F>(&mut self, max_limit: usize, mut f: F) -> usize
    where
        F: FnMut(&mut V) -> bool,
    {
        let mut removeds = 0;
        self._retain(&mut f, &mut removeds, max_limit);
        removeds
    }

    #[inline]
    fn _retain<F>(&mut self, f: &mut F, removeds: &mut usize, max_limit: usize)
    where
        F: FnMut(&mut V) -> bool,
    {
        if *removeds >= max_limit {
            return;
        }
        self.branches.retain(|_, child_node| {
            child_node._retain(f, removeds, max_limit);
            if let Some(v) = child_node.value_mut() {
                if !f(v) {
                    let _ = child_node.value.take();
                    *removeds += 1;
                }
            }
            !(child_node.value.is_none() && child_node.branches.is_empty())
        });
    }

    #[inline]
    pub fn matches(&self, topic: &Topic) -> Vec<(Topic, V)> {
        let mut out = Vec::new();
        self._matches(topic.levels(), Vec::new(), &mut out);
        out
    }

    #[inline]
    fn _matches(&self, path: &[Level], mut sub_path: Vec<Level>, out: &mut Vec<(Topic, V)>) {
        let add_to_out = |levels: Vec<Level>, v: V, out: &mut Vec<(Topic, V)>| {
            out.push((Topic::from(levels), v));
        };

        //let node_map = &self.branches;

        if self.branches.is_empty() || path.is_empty() {
            if path.is_empty() {
                //Precise matching
                if let Some(v) = self.value.as_ref() {
                    add_to_out(sub_path, v.clone(), out);
                }
            }
        } else if !path.is_empty() {
            if let Some(r) = self.branches.get(&path[0]) {
                //Precise matching
                sub_path.push(path[0].clone());

                if path.len() > 1 && path[1] == Level::MultiWildcard {
                    //# Match parent, subscription ending with #
                    if let Some(v) = r.value.as_ref() {
                        add_to_out(sub_path.clone(), v.clone(), out);
                    }
                }
                r._matches(&path[1..], sub_path, out);
            } else if matches!(path[0], Level::SingleWildcard) {
                //Single layer matching
                for (k, v) in self.branches.iter() {
                    if sub_path.is_empty() && !matches!(k, Level::Blank) && k.is_metadata() {
                        //TopicName names starting with the $character cannot be matched with topic
                        //filters starting with wildcards (# or +)
                        continue;
                    }
                    let mut sub_path = sub_path.clone();
                    sub_path.push(k.clone());

                    if path.len() > 1 && path[1] == Level::MultiWildcard {
                        //# Match parent, subscription ending with #
                        if let Some(v) = v.value.as_ref() {
                            add_to_out(sub_path.clone(), v.clone(), out);
                        }
                    }
                    v._matches(&path[1..], sub_path, out);
                }
            } else if path[0] == Level::MultiWildcard {
                //Multilayer matching
                for (k, v) in self.branches.iter() {
                    if sub_path.is_empty() && !matches!(k, Level::Blank) && k.is_metadata() {
                        //TopicName names starting with the $character cannot be matched with topic
                        //filters starting with wildcards (# or +)
                        continue;
                    }
                    let mut sub_path = sub_path.clone();
                    sub_path.push(k.clone());

                    if v.branches.is_empty() {
                        if let Some(v) = v.value.as_ref() {
                            add_to_out(sub_path, v.clone(), out);
                        }
                    } else {
                        if let Some(v) = v.value.as_ref() {
                            add_to_out(sub_path.clone(), v.clone(), out);
                        }
                        v._matches(path, sub_path, out);
                    }
                }
            }
        }
    }

    #[inline]
    pub fn value(&self) -> Option<&V> {
        self.value.as_ref()
    }

    #[inline]
    pub fn value_mut(&mut self) -> Option<&mut V> {
        self.value.as_mut()
    }

    #[inline]
    pub fn children(&self) -> &HashMap<Level, Node<V>> {
        &self.branches
    }

    #[inline]
    pub fn child(&self, l: &Level) -> Option<&Node<V>> {
        self.branches.get(l)
    }

    #[inline]
    pub fn values_size(&self) -> usize {
        let len: usize = self.branches.values().map(|n| n.values_size()).sum();
        if self.value.is_some() {
            len + 1
        } else {
            len
        }
    }

    #[inline]
    pub fn nodes_size(&self) -> usize {
        let len: usize = self.branches.values().map(|n| n.nodes_size()).sum();
        self.branches.len() + len
    }

    #[inline]
    pub fn list(&self, mut top: usize) -> Vec<String> {
        let mut out = Vec::new();
        let parent = Level::Blank;
        self._list(&mut out, &parent, &mut top, 0);
        out
    }

    #[inline]
    fn _list(&self, out: &mut Vec<String>, _parent: &Level, top: &mut usize, depth: usize) {
        if *top == 0 {
            return;
        }
        for (l, n) in self.branches.iter() {
            out.push(format!("{} {:?}", " ".repeat(depth * 3), l));
            *top -= 1;
            n._list(out, l, top, depth + 1);
            if *top == 0 {
                return;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::{RetainTree, Topic};

    fn match_one(tree: &RetainTree<i32>, topic_filter: &str, vs: &[i32]) -> bool {
        let mut matcheds = 0;
        let t = Topic::from_str(topic_filter).unwrap();
        //println!("[retain] {} ===> {:?}", topic_filter, tree.matches(&t));
        for (topic, v) in tree.matches(&t).iter() {
            println!("[retain] {topic_filter}({topic}) => {v:?}, {vs:?}");
            if !vs.contains(v) {
                return false;
            }
            matcheds += 1;
        }
        matcheds == vs.len()
    }

    #[test]
    fn retain() {
        let mut tree: RetainTree<i32> = RetainTree::default();
        tree.insert(&Topic::from_str("/iot/b/x").unwrap(), 1);
        tree.insert(&Topic::from_str("/iot/b/y").unwrap(), 2);
        tree.insert(&Topic::from_str("/iot/b/z").unwrap(), 3);
        tree.insert(&Topic::from_str("/iot/b").unwrap(), 123);
        tree.insert(&Topic::from_str("/x/y/z").unwrap(), 4);

        assert!(match_one(&tree, "/iot/b/y", &[2]));
        assert!(match_one(&tree, "/iot/b/+", &[1, 2, 3]));
        assert!(match_one(&tree, "/x/y/z", &[4]));
        assert!(!match_one(&tree, "/x/y/z", &[1]));

        tree.insert(&Topic::from_str("/xx/yy").unwrap(), -1);
        tree.insert(&Topic::from_str("/xx/yy/").unwrap(), 0);
        tree.insert(&Topic::from_str("/xx/yy/1").unwrap(), 1);
        tree.insert(&Topic::from_str("/xx/yy/2").unwrap(), 2);
        tree.insert(&Topic::from_str("/xx/yy/3").unwrap(), 3);

        tree.insert(&Topic::from_str("/xx/yy/3/4").unwrap(), 4);
        tree.insert(&Topic::from_str("/xx/yy/3/4/5").unwrap(), 5);

        assert!(match_one(&tree, "/xx/yy/+", &[0, 1, 2, 3]));
        assert!(match_one(&tree, "/xx/yy/3/+", &[4]));
        assert!(match_one(&tree, "/xx/yy/3/4/+", &[5]));
        assert!(match_one(&tree, "/xx/yy/1/+", &[]));

        println!("1 tree.values_size: {}", tree.values_size());
        println!("1 tree.nodes_size: {}", tree.nodes_size());
        tree.retain(usize::MAX, |_| false);
        println!("2 tree.values_size: {}", tree.values_size());
        println!("2 tree.nodes_size: {}", tree.nodes_size());
    }
}
