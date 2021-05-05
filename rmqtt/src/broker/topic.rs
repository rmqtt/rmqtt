use serde::de::{self, Deserialize, Deserializer, Error, SeqAccess, Visitor};
use serde::ser::{Serialize, SerializeTuple, Serializer};
use std::clone::Clone;
use std::cmp::Eq;
use std::default::Default;
use std::fmt;
use std::fmt::Debug;
use std::hash::Hash;

type HashMap<K, V> = std::collections::HashMap<K, V, ahash::RandomState>;
type HashSet<K> = std::collections::HashSet<K, ahash::RandomState>;

pub type Level = ntex_mqtt::TopicLevel;
pub type Topic = ntex_mqtt::Topic;
pub type TopicTree<V> = Node<V>;

pub struct Node<V> {
    values: HashSet<V>,
    branches: HashMap<Level, Node<V>>,
}

impl<V> Default for Node<V> {
    #[inline]
    fn default() -> Node<V> {
        Self {
            values: HashSet::default(),
            branches: HashMap::default(),
        }
    }
}

impl<V> Node<V>
where
    V: Hash + Eq + Clone + Debug,
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
    pub fn matches(&self, topic: &Topic) -> HashMap<Topic, Vec<V>> {
        let mut out = HashMap::default();
        self._matches(topic.levels(), Vec::new(), &mut out);
        out
    }

    #[inline]
    fn _matches(&self, path: &[Level], mut sub_path: Vec<Level>, out: &mut HashMap<Topic, Vec<V>>) {
        let mut add_to_out = |levels: Vec<Level>, v_set: &HashSet<V>| {
            if !v_set.is_empty() {
                out.entry(Topic::from(levels)).or_default().extend(
                    v_set
                        .iter()
                        .map(|v| (*v).clone())
                        .collect::<Vec<V>>()
                        .into_iter(),
                );
            }
        };

        if path.is_empty() {
            //Match parent #
            if let Some(n) = self.branches.get(&Level::MultiWildcard) {
                if !n.values.is_empty() {
                    let mut sub_path = sub_path.clone();
                    sub_path.push(Level::MultiWildcard);
                    add_to_out(sub_path, &n.values);
                }
            }
            add_to_out(sub_path, &self.values);
        } else {
            //Topic names starting with the $character cannot be matched with topic
            //filters starting with wildcards (# or +)
            if !(sub_path.is_empty()
                && !matches!(path[0], Level::Blank)
                && path[0].is_metadata()
                && (self.branches.contains_key(&Level::MultiWildcard)
                    || self.branches.contains_key(&Level::SingleWildcard)))
            {
                //Multilayer matching
                if let Some(n) = self.branches.get(&Level::MultiWildcard) {
                    if !n.values.is_empty() {
                        let mut sub_path = sub_path.clone();
                        sub_path.push(Level::MultiWildcard);
                        add_to_out(sub_path, &n.values);
                    }
                }

                //Single layer matching
                if let Some(n) = self.branches.get(&Level::SingleWildcard) {
                    let mut sub_path = sub_path.clone();
                    sub_path.push(Level::SingleWildcard);
                    n._matches(&path[1..], sub_path, out);
                }
            }

            //Precise matching
            if let Some(n) = self.branches.get(&path[0]) {
                sub_path.push(path[0].clone());
                n._matches(&path[1..], sub_path, out);
            }
        }
    }

    #[inline]
    pub fn values_size(&self) -> usize {
        let len: usize = self.branches.iter().map(|(_, n)| n.values_size()).sum();
        self.values.len() + len
    }

    #[inline]
    pub fn nodes_size(&self) -> usize {
        let len: usize = self.branches.iter().map(|(_, n)| n.nodes_size()).sum();
        self.branches.len() + len
    }

    #[inline]
    pub fn values(&self) -> &HashSet<V> {
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
            out.push(format!(
                //"{} {:?} => {:?}, values: {:?}",
                "{} {:?}, values: {:?}",
                " ".repeat(depth * 3),
                //parent.to_string(),
                l.to_string(),
                n.values
            ));
            n._list(out, l, top - 1, depth + 1);
        }
    }
}

impl<V> Debug for Node<V>
where
    V: Hash + Eq + Clone + Debug,
{
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Node {{ nodes_size: {}, values_size: {} }}",
            self.nodes_size(),
            self.values_size()
        )
    }
}

use crate::NodeId;
impl Serialize for Node<NodeId> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut s = serializer.serialize_tuple(2)?;
        s.serialize_element(&self.values.iter().collect::<Vec<&NodeId>>())?;
        s.serialize_element(
            &self
                .branches
                .iter()
                .map(|(k, v)| (k, v))
                .collect::<Vec<(&Level, &Node<NodeId>)>>(),
        )?;
        s.end()
    }
}

impl<'de> Deserialize<'de> for Node<NodeId> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct NodeVisitor;

        impl<'de> Visitor<'de> for NodeVisitor {
            type Value = Node<NodeId>;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("struct Node<NodeId>")
            }

            fn visit_seq<A>(self, mut seq: A) -> Result<Self::Value, A::Error>
            where
                A: SeqAccess<'de>,
            {
                if seq.size_hint() != Some(2) {
                    return Err(Error::invalid_type(serde::de::Unexpected::Seq, &self));
                }

                let values = seq
                    .next_element::<HashSet<NodeId>>()?
                    .ok_or_else(|| de::Error::missing_field("values"))?;
                let branches = seq
                    .next_element::<HashMap<Level, Node<NodeId>>>()?
                    .ok_or_else(|| de::Error::missing_field("branches"))?;

                Ok(Node { values, branches })
            }
        }
        deserializer.deserialize_tuple(2, NodeVisitor)
    }
}

#[cfg(test)]
mod tests {
    use super::super::NodeId;
    use super::{Topic, TopicTree};
    use std::str::FromStr;

    fn match_one(topics: &TopicTree<NodeId>, topic: &str, vs: &[NodeId]) -> bool {
        let mut matcheds = 0;
        let t = Topic::from_str(topic).unwrap();
        for (topic_filter, matched) in topics.matches(&t).iter() {
            println!(
                "[topic] {}({}) => {:?}, {:?}",
                topic,
                topic_filter.to_string(),
                matched,
                vs
            );
            let matched_len = matched
                .iter()
                .filter_map(|v| if vs.contains(v) { Some(v) } else { None })
                .collect::<Vec<&NodeId>>()
                .len();

            if matched_len != matched.len() {
                return false;
            }

            matcheds += matched.len();
        }
        matcheds == vs.len()
    }

    #[test]
    fn topic() {
        let mut topics: TopicTree<NodeId> = TopicTree::default();
        topics.insert(&Topic::from_str("/iot/b/x").unwrap(), 1);
        topics.insert(&Topic::from_str("/iot/b/x").unwrap(), 2);
        topics.insert(&Topic::from_str("/iot/b/y").unwrap(), 3);
        topics.insert(&Topic::from_str("/iot/cc/dd").unwrap(), 4);
        topics.insert(&Topic::from_str("/ddl/22/#").unwrap(), 5);
        topics.insert(&Topic::from_str("/ddl/+/+").unwrap(), 6);
        topics.insert(&Topic::from_str("/xyz/yy/zz").unwrap(), 7);
        topics.insert(&Topic::from_str("/xyz").unwrap(), 8);

        assert!(match_one(&topics, "/iot/b/x", &[1, 2]));
        assert!(match_one(&topics, "/iot/b/y", &[3]));
        assert!(match_one(&topics, "/iot/cc/dd", &[4]));
        assert!(!match_one(&topics, "/iot/cc/dd", &[0]));
        assert!(match_one(&topics, "/ddl/a/b", &[6]));
        assert!(match_one(&topics, "/xyz/yy/zz", &[7]));
        assert!(match_one(&topics, "/ddl/22/1/2", &[5]));
        assert!(match_one(&topics, "/ddl/22/1", &[5, 6]));
        assert!(match_one(&topics, "/ddl/22/", &[5, 6]));
        assert!(match_one(&topics, "/ddl/22", &[5]));

        assert!(topics.remove(&Topic::from_str("/iot/b/x").unwrap(), &2));
        assert!(topics.remove(&Topic::from_str("/xyz/yy/zz").unwrap(), &7));
        assert!(!topics.remove(&Topic::from_str("/xyz").unwrap(), &123));

        assert!(!match_one(&topics, "/xyz/yy/zz", &[7]));

        //------------------------------------------------------
        let mut topics: TopicTree<NodeId> = TopicTree::default();
        topics.insert(&Topic::from_str("/a/b/c").unwrap(), 1);
        topics.insert(&Topic::from_str("/a/+").unwrap(), 2);

        let topics: TopicTree<NodeId> =
            bincode::deserialize(&bincode::serialize(&topics).unwrap()).unwrap();

        assert!(match_one(&topics, "/a/b/c", &[1]));
        assert!(match_one(&topics, "/a/b", &[2]));
        assert!(match_one(&topics, "/a/1", &[2]));
    }
}
