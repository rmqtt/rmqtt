use std::borrow::Cow;
use std::future::Future;

use rmqtt::{anyhow, async_trait::async_trait, bincode, chrono, log};

pub(crate) type Result<T> = anyhow::Result<T>;

const MAX_KEY_LEN: usize = u8::MAX as usize;

pub trait StorageDB: Send + Sync {
    type StorageType: Storage;

    fn open<V: AsRef<[u8]>>(&self, name: V) -> Result<Self::StorageType>;

    fn drop<V: AsRef<[u8]>>(&self, name: V) -> Result<bool>;

    fn size_on_disk(&self) -> Result<u64>;
}

#[async_trait]
pub trait Storage: Sync + Send {
    fn insert<K, V>(&self, key: K, val: &V) -> Result<()>
    where
        K: AsRef<[u8]> + Sync + Send,
        V: serde::ser::Serialize + Sync + Send + ?Sized;

    fn get<K, V>(&self, key: K) -> Result<Option<V>>
    where
        K: AsRef<[u8]> + Sync + Send,
        V: serde::de::DeserializeOwned + Sync + Send;

    fn metadata<K: AsRef<[u8]> + Sync + Send>(&self, key: K) -> Result<Option<Metadata>>;

    fn len(&self) -> usize;

    fn is_empty(&self) -> bool;

    fn contains_key<K: AsRef<[u8]> + Sync + Send>(&self, key: K) -> Result<bool>;

    fn remove<K: AsRef<[u8]> + Sync + Send>(&self, key: K) -> Result<()>;

    fn clear(&self) -> Result<()>;

    async fn flush(&self) -> Result<usize>;

    fn iter<'a, V>(&'a self) -> Box<dyn Iterator<Item = Result<(Metadata, V)>> + 'a>
    where
        V: serde::de::DeserializeOwned + Sync + Send + 'a;

    fn prefix_iter<'a, P, V>(&'a self, prefix: P) -> Box<dyn Iterator<Item = Result<(Metadata, V)>> + 'a>
    where
        P: AsRef<[u8]>,
        V: serde::de::DeserializeOwned + Sync + Send + 'a;

    async fn retain<'a, F, Out, V>(&'a self, f: F)
    where
        F: Fn(Result<(Metadata<'a>, V)>) -> Out + Send + Sync,
        Out: Future<Output = bool> + Send + 'a,
        V: serde::de::DeserializeOwned + Sync + Send + 'a;

    async fn retain_with_meta<'a, F, Out>(&'a self, f: F)
    where
        F: Fn(Result<Metadata<'a>>) -> Out + Send + Sync,
        Out: Future<Output = bool> + Send + 'a;
}

#[derive(Serialize, Deserialize, PartialEq, Debug)]
pub struct Metadata<'a> {
    pub key: Cow<'a, [u8]>,
    /// Timestamp in unix milliseconds when this entry was written.
    pub time: i64,
    /// Size of data associated with this entry.
    pub size: usize,
}

#[derive(Clone)]
pub struct SledStorageDB {
    db: sled::Db,
}

impl SledStorageDB {
    #[inline]
    pub fn new(cfg: sled::Config) -> Result<Self> {
        let db = cfg.open()?;
        Ok(Self { db })
    }
}

impl StorageDB for SledStorageDB {
    type StorageType = SledStorageTree;

    fn open<V: AsRef<[u8]>>(&self, name: V) -> Result<Self::StorageType> {
        let tree = self.db.open_tree(name)?;
        Ok(SledStorageTree::new(tree))
    }

    fn drop<V: AsRef<[u8]>>(&self, name: V) -> Result<bool> {
        Ok(self.db.drop_tree(name)?)
    }

    fn size_on_disk(&self) -> Result<u64> {
        Ok(self.db.size_on_disk()?)
    }
}

#[derive(Clone)]
pub struct SledStorageTree {
    tree: sled::Tree,
}

impl SledStorageTree {
    #[inline]
    fn new(tree: sled::Tree) -> Self {
        Self { tree }
    }
}

#[async_trait]
impl Storage for SledStorageTree {
    #[inline]
    fn insert<K, V>(&self, key: K, val: &V) -> Result<()>
    where
        K: AsRef<[u8]> + Sync + Send,
        V: serde::ser::Serialize + Sync + Send + ?Sized,
    {
        self.remove(key.as_ref())?;

        let data = bincode::serialize(val)?;
        let m = Metadata {
            key: Cow::Borrowed(key.as_ref()),
            time: chrono::Local::now().timestamp_millis(),
            size: data.len(),
        };
        let mut buffer = Vec::with_capacity(key.as_ref().len() * 2 + 8 + 8 + 8 + 1);
        buffer.extend(key.as_ref());
        bincode::serialize_into(&mut buffer, &m)?;
        buffer.push(key.as_ref().len() as u8);
        self.tree.insert(buffer, data)?;
        Ok(())
    }

    #[inline]
    fn get<K, V>(&self, key: K) -> Result<Option<V>>
    where
        K: AsRef<[u8]> + Sync + Send,
        V: serde::de::DeserializeOwned + Sync + Send,
    {
        if key.as_ref().len() > MAX_KEY_LEN {
            return Err(anyhow::Error::msg("Key too long"));
        }
        for item in self.tree.scan_prefix(key.as_ref()) {
            match item {
                Ok((k, v)) => {
                    if key.as_ref().len() as u8 == k[k.len() - 1] {
                        return Ok(Some(bincode::deserialize::<V>(v.as_ref())?));
                    }
                }
                Err(e) => return Err(anyhow::Error::new(e)),
            }
        }
        Ok(None)
    }

    #[inline]
    fn metadata<K: AsRef<[u8]> + Sync + Send>(&self, key: K) -> Result<Option<Metadata>> {
        if key.as_ref().len() > MAX_KEY_LEN {
            return Err(anyhow::Error::msg("Key too long"));
        }

        for item in self.tree.scan_prefix(key.as_ref()) {
            match item {
                Ok((k, _v)) => {
                    if key.as_ref().len() as u8 == k[k.len() - 1] {
                        let m = bincode::deserialize::<Metadata>(&k[key.as_ref().len()..(k.len() - 1)])?;
                        return Ok(Some(m));
                    }
                }
                Err(e) => {
                    log::warn!("{:?}", e);
                    return Err(anyhow::Error::new(e));
                }
            }
        }
        Ok(None)
    }

    #[inline]
    fn len(&self) -> usize {
        self.tree.len()
    }

    #[inline]
    fn is_empty(&self) -> bool {
        self.tree.is_empty()
    }

    #[inline]
    fn contains_key<K: AsRef<[u8]> + Sync + Send>(&self, key: K) -> Result<bool> {
        if key.as_ref().len() > MAX_KEY_LEN {
            return Err(anyhow::Error::msg("Key too long"));
        }

        for item in self.tree.scan_prefix(key.as_ref()) {
            match item {
                Ok((k, _v)) => {
                    if key.as_ref().len() as u8 == k[k.len() - 1] {
                        return Ok(true);
                    }
                }
                Err(e) => {
                    log::warn!("{:?}", e);
                    return Err(anyhow::Error::new(e));
                }
            };
        }
        Ok(false)
    }

    #[inline]
    fn remove<K: AsRef<[u8]> + Sync + Send>(&self, key: K) -> Result<()> {
        if key.as_ref().len() > MAX_KEY_LEN {
            return Err(anyhow::Error::msg("Key too long"));
        }

        for item in self.tree.scan_prefix(key.as_ref()) {
            match item {
                Ok((k, _v)) => {
                    if key.as_ref().len() as u8 == k[k.len() - 1] {
                        if let Err(e) = self.tree.remove(k) {
                            log::warn!("{:?}", e);
                        }
                    }
                }
                Err(e) => {
                    log::warn!("{:?}", e);
                }
            }
        }
        Ok(())
    }

    #[inline]
    fn clear(&self) -> Result<()> {
        self.tree.clear()?;
        Ok(())
    }

    #[inline]
    async fn flush(&self) -> Result<usize> {
        Ok(self.tree.flush_async().await?)
    }

    #[inline]
    fn iter<'a, V>(&'a self) -> Box<dyn Iterator<Item = Result<(Metadata, V)>> + 'a>
    where
        V: serde::de::DeserializeOwned + Sync + Send + 'a,
    {
        Box::new(Iter { _map: self, iter: self.tree.iter(), _m: std::marker::PhantomData })
    }

    #[inline]
    fn prefix_iter<'a, P, V>(&'a self, prefix: P) -> Box<dyn Iterator<Item = Result<(Metadata, V)>> + 'a>
    where
        P: AsRef<[u8]>,
        V: serde::de::DeserializeOwned + Sync + Send + 'a,
    {
        Box::new(Iter {
            _map: self,
            iter: self.tree.scan_prefix(prefix.as_ref()),
            _m: std::marker::PhantomData,
        })
    }

    #[inline]
    async fn retain<'a, F, Out, V>(&'a self, f: F)
    where
        F: Fn(Result<(Metadata<'a>, V)>) -> Out + Send + Sync,
        Out: Future<Output = bool> + Send + 'a,
        V: serde::de::DeserializeOwned + Sync + Send + 'a,
    {
        for item in self.tree.iter() {
            match item {
                Ok((k, v)) => {
                    let key_len = k[k.len() - 1] as usize;
                    match bincode::deserialize::<Metadata>(&k[key_len..(k.len() - 1)]) {
                        Ok(m) => match bincode::deserialize::<V>(v.as_ref()) {
                            Ok(v) => {
                                if !f(Ok((m, v))).await {
                                    if let Err(e) = self.tree.remove(k) {
                                        log::warn!("{:?}", e);
                                    }
                                }
                            }
                            Err(e) => {
                                if !f(Err(anyhow::Error::new(e))).await {
                                    if let Err(e) = self.tree.remove(k) {
                                        log::warn!("{:?}", e);
                                    }
                                }
                            }
                        },
                        Err(e) => {
                            if !f(Err(anyhow::Error::new(e))).await {
                                if let Err(e) = self.tree.remove(k) {
                                    log::warn!("{:?}", e);
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    log::warn!("{:?}", e);
                }
            }
        }
    }

    async fn retain_with_meta<'a, F, Out>(&'a self, f: F)
    where
        F: Fn(Result<Metadata<'a>>) -> Out + Send + Sync,
        Out: Future<Output = bool> + Send + 'a,
    {
        for item in self.tree.iter().keys() {
            match item {
                Ok(k) => {
                    let key_len = k[k.len() - 1] as usize;
                    match bincode::deserialize::<Metadata>(&k[key_len..(k.len() - 1)]) {
                        Ok(m) => {
                            if !f(Ok(m)).await {
                                if let Err(e) = self.tree.remove(k) {
                                    log::warn!("{:?}", e);
                                }
                            }
                        }
                        Err(e) => {
                            if !f(Err(anyhow::Error::new(e))).await {
                                if let Err(e) = self.tree.remove(k) {
                                    log::warn!("{:?}", e);
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    log::warn!("{:?}", e);
                }
            }
        }
    }
}

pub struct Iter<'a, V> {
    _map: &'a SledStorageTree,
    iter: sled::Iter,
    _m: std::marker::PhantomData<V>,
}

impl<'a, V> Iterator for Iter<'a, V>
where
    V: serde::de::DeserializeOwned + Sync + Send,
{
    type Item = Result<(Metadata<'a>, V)>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.iter.next() {
            None => None,
            Some(Err(e)) => Some(Err(anyhow::Error::new(e))),
            Some(Ok((k, v))) => match bincode::deserialize::<V>(v.as_ref()) {
                Ok(v) => {
                    let key_len = k[k.len() - 1] as usize;
                    match bincode::deserialize::<Metadata>(&k[key_len..(k.len() - 1)]) {
                        Ok(m) => Some(Ok((m, v))),
                        Err(e) => Some(Err(anyhow::Error::new(e))),
                    }
                }
                Err(e) => Some(Err(anyhow::Error::new(e))),
            },
        }
    }
}
