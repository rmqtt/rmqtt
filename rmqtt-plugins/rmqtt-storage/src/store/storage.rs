use std::borrow::Cow;
use std::future::Future;
use std::io;
use std::io::ErrorKind;

use sled::transaction::{ConflictableTransactionError, ConflictableTransactionResult, TransactionalTree};

use rmqtt::{anyhow, async_trait::async_trait, bincode, chrono, log};

pub(crate) type Result<T> = anyhow::Result<T>;

const MAX_KEY_LEN: usize = u8::MAX as usize;
const KEY_COUNT_PREFIX: &[u8] = b"@count@";
const KEY_CONTENT_PREFIX: &[u8] = b"@content@";

pub trait StorageDB: Send + Sync {
    type StorageType: Storage;

    fn open<V: AsRef<[u8]>>(&self, name: V) -> Result<Self::StorageType>;

    fn drop<V: AsRef<[u8]>>(&self, name: V) -> Result<bool>;

    fn size_on_disk(&self) -> Result<u64>;

    fn generate_id(&self) -> Result<u64>;

    fn counter_inc(&self, key: &str) -> Result<usize>;
    fn counter_set(&self, key: &str, val: usize) -> Result<()>;
    fn counter_get(&self, key: &str) -> Result<usize>;
}

#[async_trait]
pub trait Storage: Sync + Send {
    fn batch_remove<K>(&self, keys: Vec<K>) -> Result<()>
    where
        K: AsRef<[u8]> + Sync + Send;

    fn batch_remove_with_prefix<K>(&self, prefixs: Vec<K>) -> Result<()>
    where
        K: AsRef<[u8]> + Sync + Send;

    fn batch_insert<K, V>(&self, key_vals: Vec<(K, V)>) -> Result<()>
    where
        K: AsRef<[u8]> + Sync + Send,
        V: serde::ser::Serialize + Sync + Send;

    fn insert<K, V>(&self, key: K, val: &V) -> Result<()>
    where
        K: AsRef<[u8]> + Sync + Send,
        V: serde::ser::Serialize + Sync + Send + ?Sized;

    fn get<K, V>(&self, key: K) -> Result<Option<V>>
    where
        K: AsRef<[u8]> + Sync + Send,
        V: serde::de::DeserializeOwned + Sync + Send;

    fn remove<K>(&self, key: K) -> Result<()>
    where
        K: AsRef<[u8]> + Sync + Send;

    fn remove_and_fetch<K, V>(&self, key: K) -> Result<Option<V>>
    where
        K: AsRef<[u8]> + Sync + Send,
        V: serde::de::DeserializeOwned + Sync + Send;

    fn remove_with_prefix<K>(&self, prefix: K) -> Result<()>
    where
        K: AsRef<[u8]> + Sync + Send;

    fn push_array<K, V>(&self, key: K, val: &V) -> Result<()>
    where
        K: AsRef<[u8]> + Sync + Send,
        V: serde::ser::Serialize + Sync + Send + ?Sized;

    fn push_array_limit<K, V>(
        &self,
        key: K,
        val: &V,
        limit: usize,
        pop_front_if_limited: bool,
    ) -> Result<Option<V>>
    where
        K: AsRef<[u8]> + Sync + Send,
        V: serde::ser::Serialize + Sync + Send + ?Sized,
        V: serde::de::DeserializeOwned;

    fn get_array<K, V>(&self, key: K) -> Result<Vec<V>>
    where
        K: AsRef<[u8]> + Sync + Send,
        V: serde::de::DeserializeOwned + Sync + Send;

    fn get_array_item<K, V>(&self, key: K, idx: usize) -> Result<Option<V>>
    where
        K: AsRef<[u8]> + Sync + Send,
        V: serde::de::DeserializeOwned + Sync + Send;

    fn get_array_len<K: AsRef<[u8]> + Sync + Send>(&self, key: K) -> Result<usize>;

    fn remove_array<K: AsRef<[u8]> + Sync + Send>(&self, key: K) -> Result<()>;

    fn pop_array<K, V>(&self, key: K) -> Result<Option<V>>
    where
        K: AsRef<[u8]> + Sync + Send,
        V: serde::de::DeserializeOwned + Sync + Send;

    #[allow(clippy::type_complexity)]
    fn array_iter<'a, V>(&'a self) -> Box<dyn Iterator<Item = Result<(Vec<u8>, Vec<V>)>> + 'a>
    where
        V: serde::de::DeserializeOwned + Sync + Send + 'a;

    fn array_key_iter(&self) -> Box<dyn Iterator<Item = Result<Vec<u8>>>>;

    fn metadata<K: AsRef<[u8]> + Sync + Send>(&self, key: K) -> Result<Option<Metadata>>;

    fn len(&self) -> usize;

    fn is_empty(&self) -> bool;

    fn contains_key<K: AsRef<[u8]> + Sync + Send>(&self, key: K) -> Result<bool>;

    fn clear(&self) -> Result<()>;

    async fn flush(&self) -> Result<usize>;

    fn iter<'a, V>(&'a self) -> Box<dyn Iterator<Item = Result<(Metadata, V)>> + 'a>
    where
        V: serde::de::DeserializeOwned + Sync + Send + 'a;

    fn iter_meta<'a>(&'a self) -> Box<dyn Iterator<Item = Result<Metadata>> + 'a>;

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

    fn increment(old: Option<&[u8]>) -> Option<Vec<u8>> {
        let number = match old {
            Some(bytes) => {
                if let Ok(array) = bytes.try_into() {
                    let number = usize::from_be_bytes(array);
                    number + 1
                } else {
                    1
                }
            }
            None => 1,
        };

        Some(number.to_be_bytes().to_vec())
    }
}

impl StorageDB for SledStorageDB {
    type StorageType = SledStorageTree;

    #[inline]
    fn open<V: AsRef<[u8]>>(&self, name: V) -> Result<Self::StorageType> {
        let tree = self.db.open_tree(name)?;
        Ok(SledStorageTree::new(tree))
    }

    #[inline]
    fn drop<V: AsRef<[u8]>>(&self, name: V) -> Result<bool> {
        Ok(self.db.drop_tree(name)?)
    }

    #[inline]
    fn generate_id(&self) -> Result<u64> {
        Ok(self.db.generate_id()?)
    }

    #[inline]
    fn size_on_disk(&self) -> Result<u64> {
        Ok(self.db.size_on_disk()?)
    }

    #[inline]
    fn counter_inc(&self, key: &str) -> Result<usize> {
        let res = if let Some(res) = self.db.update_and_fetch(key.as_bytes(), Self::increment)? {
            usize::from_be_bytes(res.as_ref().try_into()?)
        } else {
            0
        };
        Ok(res)
    }

    #[inline]
    fn counter_set(&self, key: &str, val: usize) -> Result<()> {
        self.db.insert(key, val.to_be_bytes().as_slice())?;
        Ok(())
    }

    #[inline]
    fn counter_get(&self, key: &str) -> Result<usize> {
        let res = if let Some(res) = self.db.get(key)? {
            usize::from_be_bytes(res.as_ref().try_into()?)
        } else {
            0
        };
        Ok(res)
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

    #[inline]
    fn array_key_count<K>(key: K) -> Vec<u8>
    where
        K: AsRef<[u8]>,
    {
        let mut key_count = KEY_COUNT_PREFIX.to_vec();
        key_count.extend_from_slice(key.as_ref());
        key_count
    }

    #[inline]
    fn array_key_content<K>(key: K, idx: usize) -> Vec<u8>
    where
        K: AsRef<[u8]>,
    {
        let mut key_content = Self::array_key_content_prefix(key);
        key_content.extend_from_slice(idx.to_be_bytes().as_slice());
        key_content
    }

    #[inline]
    fn array_key_content_prefix<K>(key: K) -> Vec<u8>
    where
        K: AsRef<[u8]>,
    {
        let mut key_content = KEY_CONTENT_PREFIX.to_vec();
        key_content.extend_from_slice(key.as_ref());
        key_content
    }

    #[inline]
    fn array_tx_count<K, E>(
        tx: &TransactionalTree,
        key_count: K,
    ) -> ConflictableTransactionResult<(usize, usize), E>
    where
        K: AsRef<[u8]>,
    {
        if let Some(v) = tx.get(key_count.as_ref())? {
            let (start, end) = bincode::deserialize::<(usize, usize)>(v.as_ref()).map_err(|e| {
                ConflictableTransactionError::Storage(sled::Error::Io(io::Error::new(
                    ErrorKind::InvalidData,
                    e,
                )))
            })?;
            Ok((start, end))
        } else {
            Ok((0, 0))
        }
    }

    #[inline]
    fn array_tx_set_count<K, E>(
        tx: &TransactionalTree,
        key_count: K,
        start: usize,
        end: usize,
    ) -> ConflictableTransactionResult<(), E>
    where
        K: AsRef<[u8]>,
    {
        let count_bytes = bincode::serialize(&(start, end)).map_err(|e| {
            ConflictableTransactionError::Storage(sled::Error::Io(io::Error::new(ErrorKind::InvalidData, e)))
        })?;
        tx.insert(key_count.as_ref(), count_bytes.as_slice())?;
        Ok(())
    }

    #[inline]
    fn array_tx_set_content<K, V, E>(
        tx: &TransactionalTree,
        key_content: K,
        data: V,
    ) -> ConflictableTransactionResult<(), E>
    where
        K: AsRef<[u8]>,
        V: AsRef<[u8]>,
    {
        tx.insert(key_content.as_ref(), data.as_ref())?;
        Ok(())
    }
}

#[async_trait]
impl Storage for SledStorageTree {
    #[inline]
    fn batch_remove<K>(&self, keys: Vec<K>) -> Result<()>
    where
        K: AsRef<[u8]> + Sync + Send,
    {
        let mut batch = sled::Batch::default();
        for key in keys {
            for item in self.tree.scan_prefix(key.as_ref()) {
                match item {
                    Ok((k, _v)) => {
                        if key.as_ref().len() as u8 == k[k.len() - 1] {
                            batch.remove(k);
                        }
                    }
                    Err(e) => {
                        log::warn!("{:?}", e);
                    }
                }
            }
        }
        self.tree.apply_batch(batch)?;
        Ok(())
    }

    #[inline]
    fn batch_remove_with_prefix<K>(&self, prefixs: Vec<K>) -> Result<()>
    where
        K: AsRef<[u8]> + Sync + Send,
    {
        let mut batch = sled::Batch::default();
        for prefix in prefixs {
            for item in self.tree.scan_prefix(prefix.as_ref()) {
                match item {
                    Ok((k, _v)) => {
                        batch.remove(k);
                    }
                    Err(e) => {
                        log::warn!("{:?}", e);
                    }
                }
            }
        }
        self.tree.apply_batch(batch)?;
        Ok(())
    }

    #[inline]
    fn batch_insert<K, V>(&self, key_vals: Vec<(K, V)>) -> Result<()>
    where
        K: AsRef<[u8]> + Sync + Send,
        V: serde::ser::Serialize + Sync + Send,
    {
        let mut removeds = Vec::new();
        let mut batch = sled::Batch::default();
        for (key, val) in key_vals {
            removeds.push(key.as_ref().to_vec());

            let data = bincode::serialize(&val)?;
            let m = Metadata {
                key: Cow::Borrowed(key.as_ref()),
                time: chrono::Local::now().timestamp_millis(),
                size: data.len(),
            };
            let mut buffer = Vec::with_capacity(key.as_ref().len() * 2 + 8 + 8 + 8 + 1);
            buffer.extend(key.as_ref());
            bincode::serialize_into(&mut buffer, &m)?;
            buffer.push(key.as_ref().len() as u8);
            batch.insert(buffer, data);
        }
        self.batch_remove(removeds)?;
        self.tree.apply_batch(batch)?;
        Ok(())
    }

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
    fn remove<K>(&self, key: K) -> Result<()>
    where
        K: AsRef<[u8]> + Sync + Send,
    {
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
    fn remove_and_fetch<K, V>(&self, key: K) -> Result<Option<V>>
    where
        K: AsRef<[u8]> + Sync + Send,
        V: serde::de::DeserializeOwned + Sync + Send,
    {
        if key.as_ref().len() > MAX_KEY_LEN {
            return Err(anyhow::Error::msg("Key too long"));
        }

        for item in self.tree.scan_prefix(key.as_ref()) {
            match item {
                Ok((k, _v)) => {
                    if key.as_ref().len() as u8 == k[k.len() - 1] {
                        match self.tree.remove(k) {
                            Err(e) => {
                                log::warn!("{:?}", e);
                            }
                            Ok(Some(v)) => {
                                return Ok(Some(bincode::deserialize::<V>(v.as_ref())?));
                            }
                            Ok(None) => {}
                        }
                    }
                }
                Err(e) => {
                    log::warn!("{:?}", e);
                }
            }
        }
        Ok(None)
    }

    #[inline]
    fn remove_with_prefix<K>(&self, prefix: K) -> Result<()>
    where
        K: AsRef<[u8]> + Sync + Send,
    {
        if prefix.as_ref().len() > MAX_KEY_LEN {
            return Err(anyhow::Error::msg("Prefix too long"));
        }

        for item in self.tree.scan_prefix(prefix.as_ref()) {
            match item {
                Ok((k, _v)) => {
                    if let Err(e) = self.tree.remove(k) {
                        log::warn!("{:?}", e);
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
    fn push_array<K, V>(&self, key: K, val: &V) -> Result<()>
    where
        K: AsRef<[u8]> + Sync + Send,
        V: serde::ser::Serialize + Sync + Send + ?Sized,
    {
        let data = bincode::serialize(val)?;
        let key_count = Self::array_key_count(key.as_ref());

        let _ = self.tree.transaction(move |tx| {
            let (start, mut end) = Self::array_tx_count(tx, key_count.as_slice())?;
            end += 1;
            Self::array_tx_set_count(tx, key_count.as_slice(), start, end)?;
            let key_content = Self::array_key_content(key.as_ref(), end);
            Self::array_tx_set_content(tx, key_content.as_slice(), &data)?;
            Ok::<(), ConflictableTransactionError<sled::Error>>(())
        });

        Ok(())
    }

    #[inline]
    fn push_array_limit<K, V>(
        &self,
        key: K,
        val: &V,
        limit: usize,
        pop_front_if_limited: bool,
    ) -> Result<Option<V>>
    where
        K: AsRef<[u8]> + Sync + Send,
        V: serde::ser::Serialize + Sync + Send + ?Sized,
        V: serde::de::DeserializeOwned,
    {
        let data = bincode::serialize(val)?;
        let key_count = Self::array_key_count(key.as_ref());

        let res = self.tree.transaction(move |tx| {
            let (mut start, mut end) = Self::array_tx_count::<_, ConflictableTransactionError<sled::Error>>(
                tx,
                key_count.as_slice(),
            )?;
            let count = end - start;

            let res = if count < limit {
                end += 1;
                Self::array_tx_set_count(tx, key_count.as_slice(), start, end)?;
                let key_content = Self::array_key_content(key.as_ref(), end);
                Self::array_tx_set_content(tx, key_content.as_slice(), &data)?;
                Ok(None)
            } else if pop_front_if_limited {
                let mut removed = None;
                let removed_key_content = Self::array_key_content(key.as_ref(), start + 1);
                if let Some(v) = tx.remove(removed_key_content)? {
                    removed = Some(
                        bincode::deserialize::<V>(v.as_ref())
                            .map_err(|e| sled::Error::Io(io::Error::new(ErrorKind::InvalidData, e)))?,
                    );
                    start += 1;
                }
                end += 1;
                Self::array_tx_set_count(tx, key_count.as_slice(), start, end)?;
                let key_content = Self::array_key_content(key.as_ref(), end);
                Self::array_tx_set_content(tx, key_content.as_slice(), &data)?;
                Ok(removed)
            } else {
                Err(ConflictableTransactionError::Storage(sled::Error::Io(io::Error::new(
                    ErrorKind::InvalidData,
                    "Is full",
                ))))
            };
            res
        })?;

        Ok(res)
    }

    #[inline]
    fn get_array_len<K: AsRef<[u8]> + Sync + Send>(&self, key: K) -> Result<usize> {
        let key_count = Self::array_key_count(key.as_ref());
        if let Some(v) = self.tree.get(key_count.as_slice())? {
            let (start, end) = bincode::deserialize::<(usize, usize)>(v.as_ref())?;
            Ok(end - start)
        } else {
            Ok(0)
        }
    }

    #[inline]
    fn get_array<K, V>(&self, key: K) -> Result<Vec<V>>
    where
        K: AsRef<[u8]> + Sync + Send,
        V: serde::de::DeserializeOwned + Sync + Send,
    {
        let key_content_prefix = Self::array_key_content_prefix(key.as_ref());

        let res = self
            .tree
            .scan_prefix(key_content_prefix)
            .values()
            .map(|item| {
                item.and_then(|v| {
                    bincode::deserialize::<V>(v.as_ref())
                        .map_err(|e| sled::Error::Io(io::Error::new(ErrorKind::InvalidData, e)))
                })
                .map_err(anyhow::Error::new)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(res)
    }

    #[inline]
    fn get_array_item<K, V>(&self, key: K, idx: usize) -> Result<Option<V>>
    where
        K: AsRef<[u8]> + Sync + Send,
        V: serde::de::DeserializeOwned + Sync + Send,
    {
        let key_count = Self::array_key_count(key.as_ref());

        let res = self.tree.transaction(move |tx| {
            let (start, end) = Self::array_tx_count::<_, ConflictableTransactionError<sled::Error>>(
                tx,
                key_count.as_slice(),
            )?;
            let res = if idx < (end - start) {
                let key_content = Self::array_key_content(key.as_ref(), start + idx + 1);
                if let Some(v) = tx.get(key_content)? {
                    Ok(Some(
                        bincode::deserialize::<V>(v.as_ref())
                            .map_err(|e| sled::Error::Io(io::Error::new(ErrorKind::InvalidData, e)))?,
                    ))
                } else {
                    Err(ConflictableTransactionError::Storage(sled::Error::Io(io::Error::new(
                        ErrorKind::InvalidData,
                        "Data Disorder",
                    ))))
                }
            } else {
                Err(ConflictableTransactionError::Storage(sled::Error::Io(io::Error::new(
                    ErrorKind::InvalidData,
                    "Index Out of Bounds",
                ))))
            };
            res
        })?;
        Ok(res)
    }

    #[inline]
    fn remove_array<K: AsRef<[u8]> + Sync + Send>(&self, key: K) -> Result<()> {
        let key_count = Self::array_key_count(key.as_ref());
        let key_content_prefix = Self::array_key_content_prefix(key.as_ref());
        for item in self.tree.scan_prefix(key_count).keys() {
            match item {
                Ok(k) => {
                    self.tree.remove(k)?;
                }
                Err(e) => return Err(anyhow::Error::new(e)),
            }
        }

        for item in self.tree.scan_prefix(key_content_prefix).keys() {
            match item {
                Ok(k) => {
                    self.tree.remove(k)?;
                }
                Err(e) => return Err(anyhow::Error::new(e)),
            }
        }

        Ok(())
    }

    #[inline]
    fn pop_array<K, V>(&self, key: K) -> Result<Option<V>>
    where
        K: AsRef<[u8]> + Sync + Send,
        V: serde::de::DeserializeOwned + Sync + Send,
    {
        let key_count = Self::array_key_count(key.as_ref());

        let removed = self.tree.transaction(move |tx| {
            let (start, end) = Self::array_tx_count(tx, key_count.as_slice())?;

            let mut removed = None;
            if (end - start) > 0 {
                let removed_key_content = Self::array_key_content(key.as_ref(), start + 1);
                if let Some(v) = tx.remove(removed_key_content)? {
                    removed = Some(
                        bincode::deserialize::<V>(v.as_ref())
                            .map_err(|e| sled::Error::Io(io::Error::new(ErrorKind::InvalidData, e)))?,
                    );
                    Self::array_tx_set_count(tx, key_count.as_slice(), start + 1, end)?;
                }
            }
            Ok::<_, ConflictableTransactionError<sled::Error>>(removed)
        })?;

        Ok(removed)
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
    fn iter_meta<'a>(&'a self) -> Box<dyn Iterator<Item = Result<Metadata>> + 'a> {
        Box::new(IterMeta { _map: self, iter: self.tree.iter() })
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

    #[inline]
    fn array_iter<'a, V>(&'a self) -> Box<dyn Iterator<Item = Result<(Vec<u8>, Vec<V>)>> + 'a>
    where
        V: serde::de::DeserializeOwned + Sync + Send + 'a,
    {
        Box::new(ArrayIter {
            tree: self,
            iter: self.tree.scan_prefix(KEY_COUNT_PREFIX),
            _m: std::marker::PhantomData,
        })
    }

    #[inline]
    fn array_key_iter(&self) -> Box<dyn Iterator<Item = Result<Vec<u8>>>> {
        Box::new(ArrayKeyIter { iter: self.tree.scan_prefix(KEY_COUNT_PREFIX) })
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

pub struct IterMeta<'a> {
    _map: &'a SledStorageTree,
    iter: sled::Iter,
}

impl<'a> Iterator for IterMeta<'a> {
    type Item = Result<Metadata<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.iter.next() {
            None => None,
            Some(Err(e)) => Some(Err(anyhow::Error::new(e))),
            Some(Ok((k, _))) => {
                let key_len = k[k.len() - 1] as usize;
                match bincode::deserialize::<Metadata>(&k[key_len..(k.len() - 1)]) {
                    Ok(m) => Some(Ok(m)),
                    Err(e) => Some(Err(anyhow::Error::new(e))),
                }
            }
        }
    }
}

pub struct ArrayIter<'a, V> {
    tree: &'a SledStorageTree,
    iter: sled::Iter,
    _m: std::marker::PhantomData<V>,
}

impl<'a, V> Iterator for ArrayIter<'a, V>
where
    V: serde::de::DeserializeOwned + Sync + Send,
{
    type Item = Result<(Vec<u8>, Vec<V>)>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.iter.next() {
            None => None,
            Some(Err(e)) => Some(Err(anyhow::Error::new(e))),
            Some(Ok((k, _v))) => {
                let key = k[KEY_COUNT_PREFIX.len()..].as_ref();
                let arrays = self.tree.get_array::<_, V>(key);
                let arrays = arrays.map(|arrays| (key.to_vec(), arrays));
                Some(arrays)
            }
        }
    }
}

pub struct ArrayKeyIter {
    iter: sled::Iter,
}

impl Iterator for ArrayKeyIter {
    type Item = Result<Vec<u8>>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.iter.next() {
            None => None,
            Some(Err(e)) => Some(Err(anyhow::Error::new(e))),
            Some(Ok((k, _v))) => {
                let key = k[KEY_COUNT_PREFIX.len()..].to_vec();
                Some(Ok(key))
            }
        }
    }
}
