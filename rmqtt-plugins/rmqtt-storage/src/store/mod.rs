use serde::de::DeserializeOwned;
use serde::Serialize;
use std::future::Future;

use rmqtt::{async_trait::async_trait, log};

use storage::{Metadata, Result, SledStorageDB, SledStorageTree, StorageDB as _};

use super::config::{PluginConfig, StorageType};

pub(crate) mod storage;

pub(crate) fn init_store_db(cfg: &PluginConfig) -> Result<StorageDB> {
    match cfg.storage_type {
        StorageType::Sled => {
            let sled_cfg = cfg.sled.to_sled_config()?;
            let db = SledStorageDB::new(sled_cfg)?;
            Ok(StorageDB::Sled(db))
        }
    }
}

#[derive(Clone)]
pub(crate) enum StorageDB {
    Sled(SledStorageDB),
}

impl StorageDB {
    #[inline]
    pub(crate) fn open<V: AsRef<[u8]>>(&self, name: V) -> Result<StorageKV> {
        match self {
            StorageDB::Sled(db) => {
                let s = db.open(name)?;
                Ok(StorageKV::Sled(s))
            }
        }
    }

    #[inline]
    pub(crate) fn size_on_disk(&self) -> Result<u64> {
        match self {
            StorageDB::Sled(db) => db.size_on_disk(),
        }
    }

    #[inline]
    pub(crate) fn generate_id(&self) -> Result<u64> {
        match self {
            StorageDB::Sled(db) => db.generate_id(),
        }
    }

    #[inline]
    pub(crate) fn counter_inc(&self, key: &str) -> Result<usize> {
        match self {
            StorageDB::Sled(db) => db.counter_inc(key),
        }
    }

    #[inline]
    pub(crate) fn counter_get(&self, key: &str) -> Result<usize> {
        match self {
            StorageDB::Sled(db) => db.counter_get(key),
        }
    }
}

#[derive(Clone)]
pub(crate) enum StorageKV {
    Sled(SledStorageTree),
}

#[async_trait]
impl storage::Storage for StorageKV {
    #[inline]
    fn batch_remove<K>(&self, keys: Vec<K>) -> Result<()>
    where
        K: AsRef<[u8]> + Sync + Send,
    {
        let res = match self {
            StorageKV::Sled(tree) => tree.batch_remove(keys),
        };
        match res {
            Ok(()) => Ok(()),
            Err(e) => {
                log::warn!("Storage::batch_remove error: {:?}", e);
                Err(e)
            }
        }
    }

    #[inline]
    fn batch_remove_with_prefix<K>(&self, prefixs: Vec<K>) -> Result<()>
    where
        K: AsRef<[u8]> + Sync + Send,
    {
        let res = match self {
            StorageKV::Sled(tree) => tree.batch_remove_with_prefix(prefixs),
        };
        match res {
            Ok(()) => Ok(()),
            Err(e) => {
                log::warn!("Storage::batch_remove_with_prefix error: {:?}", e);
                Err(e)
            }
        }
    }

    #[inline]
    fn batch_insert<K, V>(&self, key_vals: Vec<(K, V)>) -> Result<()>
    where
        K: AsRef<[u8]> + Sync + Send,
        V: serde::ser::Serialize + Sync + Send,
    {
        let res = match self {
            StorageKV::Sled(tree) => tree.batch_insert(key_vals),
        };
        if let Err(e) = res {
            log::warn!("Storage::batch_insert error: {:?}", e);
            Err(e)
        } else {
            Ok(())
        }
    }

    #[inline]
    fn insert<K, V>(&self, key: K, val: &V) -> Result<()>
    where
        K: AsRef<[u8]> + Sync + Send,
        V: Serialize + Sync + Send + ?Sized,
    {
        let res = match self {
            StorageKV::Sled(tree) => tree.insert(key, val),
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
            StorageKV::Sled(tree) => tree.get(key),
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
    fn remove_and_fetch<K, V>(&self, key: K) -> Result<Option<V>>
    where
        K: AsRef<[u8]> + Sync + Send,
        V: serde::de::DeserializeOwned + Sync + Send,
    {
        let res = match self {
            StorageKV::Sled(tree) => tree.remove_and_fetch(key),
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
    fn remove<K>(&self, key: K) -> Result<()>
    where
        K: AsRef<[u8]> + Sync + Send,
    {
        let res = match self {
            StorageKV::Sled(tree) => tree.remove(key),
        };
        match res {
            Ok(()) => Ok(()),
            Err(e) => {
                log::warn!("Storage::remove error: {:?}", e);
                Err(e)
            }
        }
    }

    #[inline]
    fn remove_with_prefix<K>(&self, prefix: K) -> Result<()>
    where
        K: AsRef<[u8]> + Sync + Send,
    {
        let res = match self {
            StorageKV::Sled(tree) => tree.remove_with_prefix(prefix),
        };
        match res {
            Ok(()) => Ok(()),
            Err(e) => {
                log::warn!("Storage::remove error: {:?}", e);
                Err(e)
            }
        }
    }

    #[inline]
    fn push_array<K, V>(&self, key: K, val: &V) -> Result<()>
    where
        K: AsRef<[u8]> + Sync + Send,
        V: serde::ser::Serialize + Sync + Send + ?Sized,
    {
        let res = match self {
            StorageKV::Sled(tree) => tree.push_array(key, val),
        };
        match res {
            Ok(()) => Ok(()),
            Err(e) => {
                log::warn!("Storage::push_array error: {:?}", e);
                Err(e)
            }
        }
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
        let res = match self {
            StorageKV::Sled(tree) => tree.push_array_limit(key, val, limit, pop_front_if_limited),
        };
        match res {
            Ok(res) => Ok(res),
            Err(e) => {
                log::warn!("Storage::push_array_limit error: {:?}", e);
                Err(e)
            }
        }
    }

    #[inline]
    fn get_array<K, V>(&self, key: K) -> Result<Vec<V>>
    where
        K: AsRef<[u8]> + Sync + Send,
        V: serde::de::DeserializeOwned + Sync + Send,
    {
        let res = match self {
            StorageKV::Sled(tree) => tree.get_array(key),
        };
        match res {
            Ok(res) => Ok(res),
            Err(e) => {
                log::warn!("Storage::get_array error: {:?}", e);
                Err(e)
            }
        }
    }

    #[inline]
    fn get_array_item<K, V>(&self, key: K, idx: usize) -> Result<Option<V>>
    where
        K: AsRef<[u8]> + Sync + Send,
        V: serde::de::DeserializeOwned + Sync + Send,
    {
        let res = match self {
            StorageKV::Sled(tree) => tree.get_array_item(key, idx),
        };
        match res {
            Ok(res) => Ok(res),
            Err(e) => {
                log::warn!("Storage::get_array_item error: {:?}", e);
                Err(e)
            }
        }
    }

    #[inline]
    fn get_array_len<K: AsRef<[u8]> + Sync + Send>(&self, key: K) -> Result<usize> {
        let res = match self {
            StorageKV::Sled(tree) => tree.get_array_len(key),
        };
        match res {
            Ok(res) => Ok(res),
            Err(e) => {
                log::warn!("Storage::get_array_len error: {:?}", e);
                Err(e)
            }
        }
    }

    #[inline]
    fn remove_array<K: AsRef<[u8]> + Sync + Send>(&self, key: K) -> Result<()> {
        let res = match self {
            StorageKV::Sled(tree) => tree.remove_array(key),
        };
        match res {
            Ok(()) => Ok(()),
            Err(e) => {
                log::warn!("Storage::remove_array error: {:?}", e);
                Err(e)
            }
        }
    }

    #[inline]
    fn pop_array<K, V>(&self, key: K) -> Result<Option<V>>
    where
        K: AsRef<[u8]> + Sync + Send,
        V: serde::de::DeserializeOwned + Sync + Send,
    {
        let res = match self {
            StorageKV::Sled(tree) => tree.pop_array(key),
        };
        match res {
            Ok(res) => Ok(res),
            Err(e) => {
                log::warn!("Storage::pop_array error: {:?}", e);
                Err(e)
            }
        }
    }

    #[inline]
    fn array_iter<'a, V>(&'a self) -> Box<dyn Iterator<Item = Result<(Vec<u8>, Vec<V>)>> + 'a>
    where
        V: serde::de::DeserializeOwned + Sync + Send + 'a,
    {
        match self {
            StorageKV::Sled(tree) => tree.array_iter(),
        }
    }

    #[inline]
    fn array_key_iter(&self) -> Box<dyn Iterator<Item = Result<Vec<u8>>>> {
        match self {
            StorageKV::Sled(tree) => tree.array_key_iter(),
        }
    }

    #[inline]
    fn metadata<K: AsRef<[u8]> + Sync + Send>(&self, key: K) -> Result<Option<Metadata>> {
        let res = match self {
            StorageKV::Sled(tree) => tree.metadata(key),
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
            StorageKV::Sled(tree) => tree.len(),
        }
    }
    #[inline]
    fn is_empty(&self) -> bool {
        match self {
            StorageKV::Sled(tree) => tree.is_empty(),
        }
    }
    #[inline]
    fn contains_key<K: AsRef<[u8]> + Sync + Send>(&self, key: K) -> Result<bool> {
        match self {
            StorageKV::Sled(tree) => tree.contains_key(key),
        }
    }

    #[inline]
    fn clear(&self) -> Result<()> {
        match self {
            StorageKV::Sled(tree) => tree.clear(),
        }
    }
    #[inline]
    async fn flush(&self) -> Result<usize> {
        match self {
            StorageKV::Sled(tree) => tree.flush().await,
        }
    }
    #[inline]
    fn iter<'a, V>(&'a self) -> Box<dyn Iterator<Item = Result<(Metadata, V)>> + 'a>
    where
        V: DeserializeOwned + Sync + Send + 'a,
    {
        match self {
            StorageKV::Sled(tree) => tree.iter(),
        }
    }

    #[inline]
    fn iter_meta<'a>(&'a self) -> Box<dyn Iterator<Item = Result<Metadata>> + 'a> {
        match self {
            StorageKV::Sled(tree) => tree.iter_meta(),
        }
    }

    #[inline]
    fn prefix_iter<'a, P, V>(&'a self, prefix: P) -> Box<dyn Iterator<Item = Result<(Metadata, V)>> + 'a>
    where
        P: AsRef<[u8]>,
        V: serde::de::DeserializeOwned + Sync + Send + 'a,
    {
        match self {
            StorageKV::Sled(tree) => tree.prefix_iter(prefix),
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
            StorageKV::Sled(tree) => tree.retain(f).await,
        }
    }

    #[inline]
    async fn retain_with_meta<'a, F, Out>(&'a self, f: F)
    where
        F: Fn(Result<Metadata<'a>>) -> Out + Send + Sync,
        Out: Future<Output = bool> + Send + 'a,
    {
        match self {
            StorageKV::Sled(tree) => tree.retain_with_meta(f).await,
        }
    }
}
