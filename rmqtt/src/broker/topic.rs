use std::fmt;
use std::fmt::Debug;
use std::hash::Hash;

use serde::de::Deserialize;
use serde::ser::Serialize;

use crate::broker::types::TopicFilter;

type HashMap<K, V> = std::collections::HashMap<K, V, ahash::RandomState>;
type ValueSet<K> = std::collections::BTreeSet<K>;

pub type Level = ntex_mqtt::TopicLevel;
pub type Topic = ntex_mqtt::Topic;
pub type TopicTree<V> = Node<V>;

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

    // #[inline]
    // pub fn old_matches(&self, topic: &Topic) -> HashMap<Topic, Vec<V>> {
    //     let mut out = HashMap::default();
    //     self._matches(topic.levels(), Vec::new(), &mut out);
    //     out
    // }

    // #[inline]
    // fn _matches(&self, path: &[Level], mut sub_path: Vec<Level>, out: &mut HashMap<Topic, Vec<V>>) {
    //     let mut add_to_out = |levels: Vec<Level>, v_set: &ValueSet<V>| {
    //         if !v_set.is_empty() {
    //             out.entry(Topic::from(levels))
    //                 .or_default()
    //                 .extend(v_set.iter().map(|v| (*v).clone()).collect::<Vec<V>>().into_iter());
    //         }
    //     };
    //
    //     if path.is_empty() {
    //         //Match parent #
    //         if let Some(n) = self.branches.get(&Level::MultiWildcard) {
    //             if !n.values.is_empty() {
    //                 let mut sub_path = sub_path.clone();
    //                 sub_path.push(Level::MultiWildcard);
    //                 add_to_out(sub_path, &n.values);
    //             }
    //         }
    //         add_to_out(sub_path, &self.values);
    //     } else {
    //         //Topic names starting with the $character cannot be matched with topic
    //         //filters starting with wildcards (# or +)
    //         if !(sub_path.is_empty()
    //             && !matches!(path[0], Level::Blank)
    //             && path[0].is_metadata()
    //             && (self.branches.contains_key(&Level::MultiWildcard)
    //                 || self.branches.contains_key(&Level::SingleWildcard)))
    //         {
    //             //Multilayer matching
    //             if let Some(n) = self.branches.get(&Level::MultiWildcard) {
    //                 if !n.values.is_empty() {
    //                     let mut sub_path = sub_path.clone();
    //                     sub_path.push(Level::MultiWildcard);
    //                     add_to_out(sub_path, &n.values);
    //                 }
    //             }
    //
    //             //Single layer matching
    //             if let Some(n) = self.branches.get(&Level::SingleWildcard) {
    //                 let mut sub_path = sub_path.clone();
    //                 sub_path.push(Level::SingleWildcard);
    //                 n._matches(&path[1..], sub_path, out);
    //             }
    //         }
    //
    //         //Precise matching
    //         if let Some(n) = self.branches.get(&path[0]) {
    //             sub_path.push(path[0].clone());
    //             n._matches(&path[1..], sub_path, out);
    //         }
    //     }
    // }

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
    use std::str::FromStr;

    use super::super::NodeId;
    use super::{Topic, TopicTree, VecToString};

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
            bincode::deserialize(&bincode::serialize(&topics).unwrap()).unwrap();
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
        let topics: TopicTree<()> = bincode::deserialize(&bincode::serialize(&topics).unwrap()).unwrap();
        assert_eq!(val_size, topics.values_size());
    }
}
