use serde::de::DeserializeOwned;
use serde::Serialize;
use std::future::Future;

use rmqtt::async_trait::async_trait;
use rmqtt::log;

use storage::{Metadata, Result, SledStorageDb, SledStorageTree, StorageDb as _};

use super::config::{PluginConfig, StorageType};

pub(crate) mod storage;

pub(crate) fn init_store_db(cfg: &PluginConfig) -> Result<StorageDb> {
    match cfg.storage_type {
        StorageType::Sled => {
            let sled_cfg = cfg.sled.to_sled_config()?;
            let db = SledStorageDb::new(sled_cfg)?;
            Ok(StorageDb::Sled(db))
        }
    }
}

#[derive(Clone)]
pub(crate) enum StorageDb {
    Sled(SledStorageDb),
}

impl StorageDb {
    pub(crate) fn open<V: AsRef<[u8]>>(&self, name: V) -> Result<Storage> {
        match self {
            StorageDb::Sled(db) => {
                let s = db.open(name)?;
                Ok(Storage::Sled(s))
            }
        }
    }

    pub(crate) fn size_on_disk(&self) -> Result<u64> {
        match self {
            StorageDb::Sled(db) => db.size_on_disk(),
        }
    }
}

#[derive(Clone)]
pub(crate) enum Storage {
    Sled(SledStorageTree),
}

#[async_trait]
impl storage::Storage for Storage {
    #[inline]
    fn insert<K, V>(&self, key: K, val: &V) -> Result<()>
    where
        K: AsRef<[u8]> + Sync + Send,
        V: Serialize + Sync + Send + ?Sized,
    {
        let res = match self {
            Storage::Sled(tree) => tree.insert(key, val),
        };
        if let Err(e) = res {
            log::warn!("Storage::insert error: {:?}", e);
            Err(e)
        } else {
            Ok(())
        }
    }
    #[inline]
    fn get<K, V>(&self, key: K) -> Result<Option<V>>
    where
        K: AsRef<[u8]> + Sync + Send,
        V: DeserializeOwned + Sync + Send,
    {
        let res = match self {
            Storage::Sled(tree) => tree.get(key),
        };
        match res {
            Ok(res) => Ok(res),
            Err(e) => {
                log::warn!("Storage::get error: {:?}", e);
                Err(e)
            }
        }
    }
    #[inline]
    fn metadata<K: AsRef<[u8]> + Sync + Send>(&self, key: K) -> Result<Option<Metadata>> {
        let res = match self {
            Storage::Sled(tree) => tree.metadata(key),
        };
        match res {
            Ok(res) => Ok(res),
            Err(e) => {
                log::warn!("Storage::metadata error: {:?}", e);
                Err(e)
            }
        }
    }
    #[inline]
    fn len(&self) -> usize {
        match self {
            Storage::Sled(tree) => tree.len(),
        }
    }
    #[inline]
    fn is_empty(&self) -> bool {
        match self {
            Storage::Sled(tree) => tree.is_empty(),
        }
    }
    #[inline]
    fn contains_key<K: AsRef<[u8]> + Sync + Send>(&self, key: K) -> Result<bool> {
        match self {
            Storage::Sled(tree) => tree.contains_key(key),
        }
    }
    #[inline]
    fn remove<K: AsRef<[u8]> + Sync + Send>(&self, key: K) -> Result<()> {
        let res = match self {
            Storage::Sled(tree) => tree.remove(key),
        };
        match res {
            Ok(res) => Ok(res),
            Err(e) => {
                log::warn!("Storage::remove error: {:?}", e);
                Err(e)
            }
        }
    }
    #[inline]
    fn clear(&self) -> Result<()> {
        match self {
            Storage::Sled(tree) => tree.clear(),
        }
    }
    #[inline]
    async fn flush(&self) -> Result<usize> {
        match self {
            Storage::Sled(tree) => tree.flush().await,
        }
    }
    #[inline]
    fn iter<'a, V>(&'a self) -> Box<dyn Iterator<Item = Result<(Metadata, V)>> + 'a>
    where
        V: DeserializeOwned + Sync + Send + 'a,
    {
        match self {
            Storage::Sled(tree) => tree.iter(),
        }
    }

    #[inline]
    fn prefix_iter<'a, P, V>(&'a self, prefix: P) -> Box<dyn Iterator<Item = Result<(Metadata, V)>> + 'a>
    where
        P: AsRef<[u8]>,
        V: serde::de::DeserializeOwned + Sync + Send + 'a,
    {
        match self {
            Storage::Sled(tree) => tree.prefix_iter(prefix),
        }
    }

    #[inline]
    async fn retain<'a, F, Out, V>(&'a self, f: F)
    where
        F: Fn(Result<(Metadata<'a>, V)>) -> Out + Send + Sync,
        Out: Future<Output = bool> + Send + 'a,
        V: serde::de::DeserializeOwned + Sync + Send + 'a,
    {
        match self {
            Storage::Sled(tree) => tree.retain(f).await,
        }
    }

    #[inline]
    async fn retain_with_meta<'a, F, Out>(&'a self, f: F)
    where
        F: Fn(Result<Metadata<'a>>) -> Out + Send + Sync,
        Out: Future<Output = bool> + Send + 'a,
    {
        match self {
            Storage::Sled(tree) => tree.retain_with_meta(f).await,
        }
    }
}
