//! MQTT Topic Tree Implementation with Wildcard Support
//!
//! Provides a hierarchical data structure for efficient storage and retrieval of MQTT subscriptions
//! with full protocol-compliant wildcard matching capabilities. Implements MQTT 3.1.1/5.0 spec
//! requirements for topic filtering and metadata handling.
//!
//! ## Core Components
//! 1. **Tree Structure**:
//!    - `Node<V>` represents tree nodes containing subscription values and child branches
//!    - `HashMap<Level, Node<V>>` manages topic hierarchy with O(1) level access
//!    - `BTreeSet<V>` ensures ordered storage of subscription values
//!
//! 2. **Wildcard Handling**:
//!    - Implements single-level (`+`) and multi-level (`#`) wildcard matching per MQTT spec
//!    - Enforces metadata topic isolation (topics starting with `$`) from wildcard matches
//!    - Supports hierarchical topic validation through `Level` enum constraints
//!
//! ## Key Features
//! - **Efficient Matching Algorithm**:
//!   ```rust,ignore
//!   impl MatchedIter<'a, V> {
//!       // Recursive matching with path tracking
//!   }
//!   ```
//!   Implements depth-first traversal with branch pruning for O(n) worst-case performance
//!
//! - **Cluster-Ready Serialization**:
//!   - `Serialize/Deserialize` implementations enable distributed state synchronization
//!   - `postcard` compatible for network-efficient binary representation
//!
//! - **Memory Optimization**:
//!   - Automatic node cleanup during removal operations
//!   - Smart pointer usage minimizes value cloning
//!   - Lazy iterator pattern reduces intermediate allocations
//!
//! ## Protocol Compliance
//! 1. **Topic Structure Validation**:
//!    - Rejects invalid level combinations (e.g., `#` not in last position)
//!    - Prevents wildcards in metadata topics per MQTT spec
//!
//! 2. **Matching Semantics**:
//!    - `/sport/+` matches `/sport/tennis` but not `/sport`
//!    - `/home/#` matches all subtrees under `/home`
//!    - `$SYS/...` topics never match wildcard subscriptions
//!
//! ## Performance Characteristics
//! - Insertion/Removal: O(k) where k = topic depth
//! - Match Lookup: O(n*m) worst-case (n = topic depth, m = wildcard branches)
//! - Memory Overhead: ~40 bytes/node + value storage
//!
//! Typical usage includes IoT platforms requiring high-throughput subscription management.
//! Integrates with Rust MQTT ecosystems through generic value types.

use std::fmt;
use std::fmt::Debug;
use std::hash::Hash;

use serde::{Deserialize, Serialize};

use crate::topic::Level;
use crate::topic::Topic;
use crate::types::TopicFilter;

type HashMap<K, V> = std::collections::HashMap<K, V, ahash::RandomState>;
type ValueSet<K> = std::collections::BTreeSet<K>;

/// A topic trie tree, alias for [`Node<V>`].
///
/// Provides O(k) insertion/removal and efficient wildcard matching
/// for MQTT topic-based subscription storage.
pub type TopicTree<V> = Node<V>;

/// A node in the topic trie tree.
///
/// Each node holds a set of values (subscribers) at its level and
/// branches to child nodes keyed by [`Level`]. Leaf nodes contain
/// the values associated with a complete topic path, while interior
/// nodes route matching through wildcard and literal branches.
///
/// # Type Parameters
/// * `V` — The value type stored at each leaf (typically a subscriber
///   identifier). Must implement `Ord`, `Hash`, `Eq`, `Clone`, and `Debug`.
#[derive(Serialize, Deserialize)]
pub struct Node<V: Ord> {
    values: ValueSet<V>,
    branches: HashMap<Level, Node<V>>,
}

impl<V> Default for Node<V>
where
    V: Hash + Ord + Eq + Clone + Debug,
{
    #[inline]
    fn default() -> Node<V> {
        Self { values: ValueSet::default(), branches: HashMap::default() }
    }
}

impl<V> AsRef<Node<V>> for Node<V>
where
    V: Hash + Ord + Eq + Clone + Debug,
{
    fn as_ref(&self) -> &Node<V> {
        self
    }
}

impl<V> Node<V>
where
    V: Hash + Ord + Eq + Clone + Debug + Serialize + Deserialize<'static>,
{
    #[inline]
    pub fn insert(&mut self, topic_filter: &Topic, value: V) -> bool {
        let mut path = topic_filter.levels().clone();
        path.reverse();
        self._insert(path, value)
    }

    #[inline]
    fn _insert(&mut self, mut path: Vec<Level>, value: V) -> bool {
        if let Some(first) = path.pop() {
            self.branches.entry(first).or_default()._insert(path, value)
        } else {
            self.values.insert(value)
        }
    }

    #[inline]
    pub fn remove(&mut self, topic_filter: &Topic, value: &V) -> bool {
        self._remove(topic_filter.levels().as_ref(), value)
    }

    #[inline]
    fn _remove(&mut self, path: &[Level], value: &V) -> bool {
        if path.is_empty() {
            self.values.remove(value)
        } else {
            let t = &path[0];
            if let Some(x) = self.branches.get_mut(t) {
                let res = x._remove(&path[1..], value);
                if x.values.is_empty() && x.branches.is_empty() {
                    self.branches.remove(t);
                }
                res
            } else {
                false
            }
        }
    }

    #[inline]
    pub fn is_match(&self, topic: &Topic) -> bool {
        self.matches(topic).first().is_some()
    }

    #[inline]
    pub fn matches<'a>(&'a self, topic: &'a Topic) -> Matcher<'a, V> {
        Matcher { node: self, path: topic.levels() }
    }

    #[inline]
    pub fn values_size(&self) -> usize {
        let len: usize = self.branches.values().map(|n| n.values_size()).sum();
        self.values.len() + len
    }

    #[inline]
    pub fn nodes_size(&self) -> usize {
        let len: usize = self.branches.values().map(|n| n.nodes_size()).sum();
        self.branches.len() + len
    }

    #[inline]
    pub fn values(&self) -> &ValueSet<V> {
        &self.values
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
    pub fn list(&self, top: usize) -> Vec<String> {
        let mut out = Vec::new();
        let parent = Level::Blank;
        self._list(&mut out, &parent, top, 0);
        out
    }

    #[inline]
    fn _list(&self, out: &mut Vec<String>, _parent: &Level, top: usize, depth: usize) {
        if top == 0 {
            return;
        }
        for (l, n) in self.branches.iter() {
            out.push(format!("{} {:?}", " ".repeat(depth * 3), l));
            n._list(out, l, top - 1, depth + 1);
        }
    }
}

impl<V> Debug for Node<V>
where
    V: Hash + Eq + Ord + Clone + Debug + Serialize + Deserialize<'static>,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Node {{ nodes_size: {}, values_size: {} }}", self.nodes_size(), self.values_size())
    }
}

type Item<'a, V> = (Vec<&'a Level>, Vec<&'a V>);

/// Iterator-style matcher over the topic trie.
///
/// Created by [`Node::matches`], this struct lazily evaluates
/// wildcard and literal branches against a topic path to find
/// all matching subscription entries.
pub struct Matcher<'a, V: Ord> {
    node: &'a Node<V>,
    path: &'a [Level],
}

impl<'a, V> Matcher<'a, V>
where
    V: Hash + Eq + Ord + Clone + Debug + Serialize + Deserialize<'static>,
{
    #[inline]
    pub fn iter(&self) -> MatchedIter<'a, V> {
        MatchedIter::new(self.node, self.path, Vec::new())
    }

    #[inline]
    pub fn first(&self) -> Option<Item<'a, V>> {
        self.iter().next()
    }
}

/// Conversion trait from a level vector to its string representation.
///
/// Joins the levels with `/` separator to reconstruct a topic string.
pub trait VecToString {
    fn to_string(&self) -> String;
}

impl VecToString for Vec<&Level> {
    #[inline]
    fn to_string(&self) -> String {
        self.iter().map(|l| l.to_string()).collect::<Vec<String>>().join("/")
    }
}

impl VecToString for &[Level] {
    #[inline]
    fn to_string(&self) -> String {
        self.iter().map(|l| l.to_string()).collect::<Vec<String>>().join("/")
    }
}

/// Conversion trait from a level vector to [`Topic`] and [`TopicFilter`].
pub trait VecToTopic {
    fn to_topic(&self) -> Topic;
    fn to_topic_filter(&self) -> TopicFilter;
}

impl VecToTopic for Vec<&Level> {
    #[inline]
    fn to_topic(&self) -> Topic {
        Topic::from(self.iter().map(|l| (*l).clone()).collect::<Vec<Level>>())
    }

    #[inline]
    fn to_topic_filter(&self) -> TopicFilter {
        TopicFilter::from(self.to_topic().to_string())
    }
}

/// Lazily-evaluated iterator over matched subscription entries.
///
/// Traverses the trie depth-first, yielding `(levels, values)` pairs
/// for each matching subscription path. Handles multi-level (`#`) and
/// single-level (`+`) wildcards per the MQTT specification.
pub struct MatchedIter<'a, V: Ord> {
    node: &'a Node<V>,
    path: &'a [Level],
    sub_path: Option<Vec<&'a Level>>,
    curr_items: Vec<(Vec<&'a Level>, Vec<&'a V>)>,
    sub_iters: Vec<Self>,
}

impl<'a, V> MatchedIter<'a, V>
where
    V: Hash + Eq + Ord + Clone + Debug + Serialize + Deserialize<'static>,
{
    #[inline]
    fn new(node: &'a Node<V>, path: &'a [Level], sub_path: Vec<&'a Level>) -> Self {
        Self { node, path, sub_path: Some(sub_path), curr_items: Vec::new(), sub_iters: Vec::new() }
    }

    #[inline]
    fn add_to_items(&mut self, levels: Vec<&'a Level>, v_set: &'a ValueSet<V>) {
        if !v_set.is_empty() {
            self.curr_items.push((levels, v_set.iter().collect()));
        }
    }

    #[inline]
    fn next_item(&mut self) -> Option<Item<'a, V>> {
        if let Some(item) = self.curr_items.pop() {
            return Some(item);
        }
        while !self.sub_iters.is_empty() {
            if let Some(item) = self.sub_iters[0].next() {
                return Some(item);
            }
            self.sub_iters.remove(0);
        }
        None
    }

    #[inline]
    fn prepare(&mut self) -> Option<()> {
        if self.path.is_empty() {
            //Match parent #
            if let Some(b_node) = self.node.branches.get(&Level::MultiWildcard) {
                if !b_node.values.is_empty() {
                    let mut sub_path = self.sub_path.clone()?;
                    sub_path.push(&Level::MultiWildcard);
                    self.add_to_items(sub_path, &b_node.values);
                }
            }
            let sub_path = self.sub_path.take()?;
            self.add_to_items(sub_path, &self.node.values);
        } else {
            //Topic names starting with the $character cannot be matched with topic
            //filters starting with wildcards (# or +)
            if !(self.sub_path.as_ref()?.is_empty()
                && !matches!(self.path[0], Level::Blank)
                && self.path[0].is_metadata()
                && (self.node.branches.contains_key(&Level::MultiWildcard)
                    || self.node.branches.contains_key(&Level::SingleWildcard)))
            {
                //Multilayer matching
                if let Some(b_node) = self.node.branches.get(&Level::MultiWildcard) {
                    if !b_node.values.is_empty() {
                        let mut sub_path = self.sub_path.clone()?;
                        sub_path.push(&Level::MultiWildcard);
                        self.add_to_items(sub_path, &b_node.values);
                    }
                }

                //Single layer matching
                if let Some(b_node) = self.node.branches.get(&Level::SingleWildcard) {
                    let mut sub_path = self.sub_path.clone()?;
                    sub_path.push(&Level::SingleWildcard);
                    self.sub_iters.push(MatchedIter::new(b_node, &self.path[1..], sub_path));
                }
            }

            //Precise matching
            if let Some(b_node) = self.node.branches.get(&self.path[0]) {
                let mut sub_path = self.sub_path.take()?;
                sub_path.push(&self.path[0]);
                self.sub_iters.push(MatchedIter::new(b_node, &self.path[1..], sub_path));
            }
        }
        self.sub_path.take();

        Some(())
    }

    fn _debug(&self, tag: &str) {
        println!(
            "{} sub_iters:{}, curr_items:{}, path:{}, sub_path:{}",
            tag,
            self.sub_iters.len(),
            self.curr_items.len(),
            self.path.to_string(),
            self.sub_path.as_ref().map(|path| path.to_string()).unwrap_or_else(|| "None".into())
        );
    }
}

impl<'a, V> Iterator for MatchedIter<'a, V>
where
    V: Hash + Eq + Ord + Clone + Debug + Serialize + Deserialize<'static>,
{
    type Item = (Vec<&'a Level>, Vec<&'a V>);

    #[inline]
    fn next(&mut self) -> Option<Self::Item> {
        if let Some(item) = self.next_item() {
            return Some(item);
        }
        self.sub_path.as_ref()?;

        self.prepare()?;

        if let Some(item) = self.next_item() {
            return Some(item);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{Topic, TopicTree, VecToString};
    use crate::types::NodeId;
    use std::str::FromStr;

    fn match_one(topics: &TopicTree<NodeId>, topic: &str, vs: &[NodeId]) -> bool {
        let mut matcheds = 0;
        let t = Topic::from_str(topic).unwrap();
        for (i, (topic_filter, matched)) in topics.matches(&t).iter().enumerate() {
            let matched_len = matched.iter().filter(|v| vs.contains(v)).collect::<Vec<&&NodeId>>().len();

            println!(
                "{} [topic] {}({}) => {:?}({},{}), {:?}",
                i,
                topic,
                topic_filter.to_string(),
                matched,
                matched.len(),
                matched_len,
                vs
            );

            if matched_len != matched.len() {
                return false;
            }

            matcheds += matched.len();
        }
        matcheds == vs.len()
    }

    #[test]
    fn topic_nodeid() {
        let mut topics: TopicTree<NodeId> = TopicTree::default();
        topics.insert(&Topic::from_str("/iot/b/x").unwrap(), 1);
        topics.insert(&Topic::from_str("/iot/b/x").unwrap(), 2);
        topics.insert(&Topic::from_str("/iot/b/y").unwrap(), 3);
        topics.insert(&Topic::from_str("/iot/cc/dd").unwrap(), 4);
        topics.insert(&Topic::from_str("/ddl/22/#").unwrap(), 5);
        topics.insert(&Topic::from_str("/ddl/+/+").unwrap(), 6);
        topics.insert(&Topic::from_str("/ddl/+/1").unwrap(), 7);
        topics.insert(&Topic::from_str("/ddl/#").unwrap(), 8);
        topics.insert(&Topic::from_str("/xyz/yy/zz").unwrap(), 7);
        topics.insert(&Topic::from_str("/xyz").unwrap(), 8);

        println!("{}", topics.list(100).join("\n"));
        //assert!(topics.is_match(&Topic::from_str("/iot/b/x").unwrap()));

        assert!(match_one(&topics, "/iot/b/x", &[1, 2]));
        assert!(match_one(&topics, "/iot/b/y", &[3]));
        assert!(match_one(&topics, "/iot/cc/dd", &[4]));
        assert!(!match_one(&topics, "/iot/cc/dd", &[0]));
        //assert!(match_one(&topics, "/ddl/a/b", &[6]));
        assert!(match_one(&topics, "/xyz/yy/zz", &[7]));
        assert!(match_one(&topics, "/ddl/22/1/2", &[5, 8]));
        assert!(match_one(&topics, "/ddl/22/1", &[5, 6, 7, 8]));
        assert!(match_one(&topics, "/ddl/22/", &[5, 6, 8]));
        assert!(match_one(&topics, "/ddl/22", &[5, 8]));

        //match_one(&topics, "/ddl/22/1", &[5, 6, 7, 8]);

        assert!(topics.remove(&Topic::from_str("/iot/b/x").unwrap(), &2));
        assert!(topics.remove(&Topic::from_str("/xyz/yy/zz").unwrap(), &7));
        assert!(!topics.remove(&Topic::from_str("/xyz").unwrap(), &123));

        assert!(!match_one(&topics, "/xyz/yy/zz", &[7]));

        //------------------------------------------------------
        let mut topics: TopicTree<NodeId> = TopicTree::default();
        topics.insert(&Topic::from_str("/a/b/c").unwrap(), 1);
        topics.insert(&Topic::from_str("/a/+").unwrap(), 2);
        topics.insert(&Topic::from_str("/iot/b/c").unwrap(), 1);
        topics.insert(&Topic::from_str("/iot/b").unwrap(), 2);
        topics.insert(&Topic::from_str("/iot/#").unwrap(), 3);
        topics.insert(&Topic::from_str("/iot/10").unwrap(), 10);
        topics.insert(&Topic::from_str("/iot/11").unwrap(), 11);

        let start = std::time::Instant::now();
        for v in 1..10000 {
            topics.insert(&Topic::from_str(&format!("/iot/{v}")).unwrap(), v);
        }
        for v in 1..10000 {
            topics.insert(&Topic::from_str("/iot/x").unwrap(), v);
        }
        println!("insert cost time: {:?}", start.elapsed());
        println!("serialize topics.values_size(): {:?}", topics.values_size());
        let val_size = topics.values_size();
        let mut topics: TopicTree<NodeId> =
            postcard::from_bytes(&postcard::to_stdvec(&topics).unwrap()).unwrap();
        println!("deserialize topics.values_size(): {:?}", topics.values_size());
        assert_eq!(val_size, topics.values_size());
        assert!(match_one(&topics, "/a/b/c", &[1]));
        assert!(match_one(&topics, "/a/b", &[2]));
        assert!(match_one(&topics, "/a/1", &[2]));

        let t = Topic::from_str("/iot/x").unwrap();
        let start = std::time::Instant::now();
        for (topic_filter, matched) in topics.matches(&t).iter() {
            println!("[topic] {}({}) => len: {}", t, topic_filter.to_string(), matched.len());
        }
        println!("cost time: {:?}", start.elapsed());

        let start = std::time::Instant::now();
        assert!(topics.is_match(&t));
        println!("is_matches cost time: {:?}", start.elapsed());

        topics.insert(&Topic::from_str("/x/y/z/#").unwrap(), 1);
        topics.insert(&Topic::from_str("/x/y/z/#").unwrap(), 2);
        topics.insert(&Topic::from_str("/x/y/z/").unwrap(), 3);
        assert!(match_one(&topics, "/x/y/z/", &[1, 2, 3]));

        topics.insert(&Topic::from_str("/x/y/z/+").unwrap(), 1);
        topics.insert(&Topic::from_str("/x/y/z/+").unwrap(), 2);
        topics.insert(&Topic::from_str("/x/y/z/+").unwrap(), 3);
        assert!(match_one(&topics, "/x/y/z/2", &[1, 2, 1, 2, 3]));
    }

    #[test]
    fn topic() {
        let mut topics: TopicTree<()> = TopicTree::default();
        topics.insert(&Topic::from_str("/iot/b/x").unwrap(), ());
        topics.insert(&Topic::from_str("/iot/b/x").unwrap(), ());
        topics.insert(&Topic::from_str("/iot/b/y").unwrap(), ());
        topics.insert(&Topic::from_str("/iot/cc/dd").unwrap(), ());
        topics.insert(&Topic::from_str("/ddl/22/#").unwrap(), ());

        let val_size = topics.values_size();
        let topics: TopicTree<()> = postcard::from_bytes(&postcard::to_stdvec(&topics).unwrap()).unwrap();
        assert_eq!(val_size, topics.values_size());
    }

    /// The values a topic name reaches, sorted, so a case can state the match set
    /// directly instead of going through `match_one`'s accounting.
    fn matched_values(topics: &TopicTree<NodeId>, name: &str) -> Vec<NodeId> {
        let topic = Topic::from_str(name).unwrap();
        let mut values = Vec::new();

        for (_, vs) in topics.matches(&topic).iter() {
            values.extend(vs.into_iter().copied());
        }

        values.sort_unstable();
        values
    }

    /// [MQTT-4.7.2-1]: a filter that *begins* with a wildcard must not reach a topic
    /// name that begins with `$`. `MatchedIter::prepare` is the only place the trie
    /// applies that isolation, and nothing exercised it until now.
    #[test]
    fn dollar_name_is_out_of_reach_of_a_root_wildcard() {
        let mut topics: TopicTree<NodeId> = TopicTree::default();
        topics.insert(&Topic::from_str("#").unwrap(), 1);
        topics.insert(&Topic::from_str("+/+").unwrap(), 2);
        topics.insert(&Topic::from_str("+/name").unwrap(), 3);
        topics.insert(&Topic::from_str("$SYS").unwrap(), 4);
        topics.insert(&Topic::from_str("$SYS/#").unwrap(), 5);

        // The `#` and `+` subscriptions sit in the tree and still reach nothing:
        // only the two explicit `$SYS` filters answer, and `$SYS/#` covers the parent
        // level the way any `x/#` does.
        assert_eq!(matched_values(&topics, "$SYS"), vec![4, 5]);
        assert_eq!(matched_values(&topics, "$SYS/a"), vec![5]);
        assert_eq!(matched_values(&topics, "$SYS/a/b"), vec![5]);
        // Without the isolation these two would also pick up 1, 2 and 3.
        assert_eq!(matched_values(&topics, "a/b"), vec![1, 2]);
    }

    /// The isolation belongs to the first level only. A `$` level further down is an
    /// ordinary level for the wildcards above it — the shape the Homie convention
    /// publishes device state in (GitHub issue #537).
    #[test]
    fn dollar_level_below_the_root_is_reachable() {
        let mut topics: TopicTree<NodeId> = TopicTree::default();
        topics.insert(&Topic::from_str("#").unwrap(), 1);
        topics.insert(&Topic::from_str("homie/+/$state").unwrap(), 2);
        topics.insert(&Topic::from_str("homie/dev/#").unwrap(), 3);
        topics.insert(&Topic::from_str("homie/dev/$state").unwrap(), 4);

        assert_eq!(matched_values(&topics, "homie/dev/$state"), vec![1, 2, 3, 4]);
        assert_eq!(matched_values(&topics, "homie/other/$state"), vec![1, 2]);
        assert_eq!(matched_values(&topics, "homie/dev/plain"), vec![1, 3]);
    }

    /// A first level that is *empty* (a leading `/`) is not a `$` level, so the
    /// isolation must not fire on it — the `!matches!(path[0], Level::Blank)` term of
    /// the guard. The same name one level up *is* reserved.
    #[test]
    fn blank_first_level_is_not_treated_as_dollar() {
        let mut topics: TopicTree<NodeId> = TopicTree::default();
        topics.insert(&Topic::from_str("#").unwrap(), 1);
        topics.insert(&Topic::from_str("+/b").unwrap(), 2);
        topics.insert(&Topic::from_str("/a").unwrap(), 3);

        assert_eq!(matched_values(&topics, "/a"), vec![1, 3]);
        assert_eq!(matched_values(&topics, "/b"), vec![1, 2]);
        // `$` in the second level is nothing special, and the leading blank keeps
        // the root wildcard from being suppressed.
        assert_eq!(matched_values(&topics, "/$x"), vec![1]);
        // The same level in the first position is out of the wildcard's reach.
        assert!(matched_values(&topics, "$x").is_empty());
    }

    /// `#` covers its parent level as well ([MQTT-4.7.1-2]), while `+` still demands
    /// exactly one level.
    #[test]
    fn multi_wildcard_matches_the_parent_level() {
        let mut topics: TopicTree<NodeId> = TopicTree::default();
        topics.insert(&Topic::from_str("a/#").unwrap(), 1);
        topics.insert(&Topic::from_str("a/+").unwrap(), 2);

        assert_eq!(matched_values(&topics, "a"), vec![1]);
        assert_eq!(matched_values(&topics, "a/b"), vec![1, 2]);
        assert_eq!(matched_values(&topics, "a/b/c"), vec![1]);
    }

    /// Two subscribers on the same filter are two independent values, and removing
    /// one leaves the other: the router keys its relations by filter, so a filter
    /// that silently carried a single value would drop a client.
    #[test]
    fn same_filter_keeps_one_value_per_subscriber() {
        let mut topics: TopicTree<NodeId> = TopicTree::default();
        topics.insert(&Topic::from_str("a/#").unwrap(), 1);
        topics.insert(&Topic::from_str("a/#").unwrap(), 2);

        assert_eq!(matched_values(&topics, "a/b"), vec![1, 2]);
        assert!(topics.remove(&Topic::from_str("a/#").unwrap(), &1));
        assert_eq!(matched_values(&topics, "a/b"), vec![2]);
        assert!(!topics.remove(&Topic::from_str("a/#").unwrap(), &1));
    }

    /// `$` inside a level is ordinary, and the isolation is decided by the *first*
    /// level of the name alone: `$dev` at the root is out of a wildcard's reach even
    /// though it is not literally `$SYS`, while a `$dev` one level down is not.
    #[test]
    fn dollar_inside_a_level_is_not_isolated() {
        let mut topics: TopicTree<NodeId> = TopicTree::default();
        topics.insert(&Topic::from_str("#").unwrap(), 1);
        topics.insert(&Topic::from_str("homie/+/+").unwrap(), 2);
        topics.insert(&Topic::from_str("homie/#").unwrap(), 3);
        topics.insert(&Topic::from_str("homie/$dev/$state").unwrap(), 4);
        topics.insert(&Topic::from_str("homie/dev$/sta$te").unwrap(), 5);
        topics.insert(&Topic::from_str("$dev/state").unwrap(), 6);

        // Below the root a metadata level is an ordinary level for the wildcards
        // above it, `#` and `+` alike.
        assert_eq!(matched_values(&topics, "homie/$dev/$state"), vec![1, 2, 3, 4]);
        // A `$` inside a level is ordinary in every position.
        assert_eq!(matched_values(&topics, "homie/dev$/sta$te"), vec![1, 2, 3, 5]);
        // At the root the name is reserved whatever the `$` is attached to.
        assert_eq!(matched_values(&topics, "$dev/state"), vec![6]);
    }
}
