use dashmap::iter::Iter;
use dashmap::mapref::one::{Ref, RefMut};

use crate::{MqttError, Result};

type DashMap<K, V> = dashmap::DashMap<K, V, ahash::RandomState>;
pub type EntryRef<'a> = Ref<'a, String, Entry, ahash::RandomState>;
pub type EntryRefMut<'a> = RefMut<'a, String, Entry, ahash::RandomState>;
pub type EntryIter<'a> = Iter<'a, String, Entry, ahash::RandomState, DashMap<String, Entry>>;

#[async_trait]
pub trait Plugin: Send + Sync {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        Ok(())
    }

    #[inline]
    fn name(&self) -> &str {
        ""
    }

    #[inline]
    fn get_config(&self) -> Result<serde_json::Value> {
        Ok(json!({}))
    }

    #[inline]
    async fn load_config(&mut self) -> Result<()> {
        Ok(())
    }

    #[inline]
    async fn start(&mut self) -> Result<()> {
        Ok(())
    }

    #[inline]
    async fn stop(&mut self) -> Result<bool> {
        Ok(true)
    }

    #[inline]
    fn version(&self) -> &str {
        "0.0.0"
    }

    #[inline]
    fn descr(&self) -> &str {
        ""
    }

    #[inline]
    fn attrs(&self) -> serde_json::Value {
        serde_json::Value::Null
    }
}

pub struct Entry {
    active: bool,
    pub plugin: Box<dyn Plugin>,
}

impl Entry {
    #[inline]
    pub fn active(&self) -> bool {
        self.active
    }

    #[inline]
    pub fn to_json(&self) -> serde_json::Value {
        json!({
            "name": self.plugin.name(),
            "version": self.plugin.version(),
            "descr": self.plugin.descr(),
            "active": self.active,
            "attrs": self.plugin.attrs(),
        })
    }
}

pub struct Manager {
    plugins: DashMap<String, Entry>,
}

impl Manager {
    pub(crate) fn new() -> Self {
        Self {
            plugins: DashMap::default(),
        }
    }

    ///Register a Plugin
    pub async fn register(&self, mut plugin: Box<dyn Plugin>, default_startup: bool) -> Result<()> {
        if let Some((_, mut entry)) = self.plugins.remove(plugin.name()) {
            if entry.active {
                entry.plugin.stop().await?;
            }
        }
        plugin.init().await?;
        if default_startup {
            plugin.start().await?;
        }
        let name = plugin.name().into();
        self.plugins.insert(
            name,
            Entry {
                active: default_startup,
                plugin,
            },
        );
        Ok(())
    }

    ///Return Config
    pub fn get_config(&self, name: &str) -> Result<serde_json::Value> {
        if let Some(entry) = self.get(name) {
            entry.plugin.get_config()
        } else {
            Err(MqttError::from(format!(
                "{} the plug-in does not exist",
                name
            )))
        }
    }

    ///Load Config
    pub async fn load_config(&self, name: &str) -> Result<()> {
        if let Some(mut entry) = self.get_mut(name) {
            entry.plugin.load_config().await?;
            Ok(())
        } else {
            Err(MqttError::from(format!(
                "{} the plug-in does not exist",
                name
            )))
        }
    }

    ///Start a Plugin
    pub async fn start(&self, name: &str) -> Result<()> {
        if let Some(mut entry) = self.get_mut(name) {
            if !entry.active {
                entry.plugin.start().await?;
                entry.active = true;
            }
            Ok(())
        } else {
            Err(MqttError::from(format!(
                "{} the plug-in does not exist",
                name
            )))
        }
    }

    ///Stop a Plugin
    pub async fn stop(&self, name: &str) -> Result<bool> {
        if let Some(mut entry) = self.get_mut(name) {
            if entry.active {
                let stopped = entry.plugin.stop().await?;
                entry.active = !stopped;
                Ok(stopped)
            } else {
                Err(MqttError::from(format!(
                    "{} the plug-in is not started",
                    name
                )))
            }
        } else {
            Err(MqttError::from(format!(
                "{} the plug-in does not exist",
                name
            )))
        }
    }

    ///Plugin is active
    pub fn is_active(&self, name: &str) -> bool {
        if let Some(entry) = self.plugins.get(name) {
            entry.active()
        } else {
            false
        }
    }

    ///Get a Plugin
    pub fn get(&self, name: &str) -> Option<EntryRef> {
        self.plugins.get(name)
    }

    ///Get a mut Plugin
    pub fn get_mut(&self, name: &str) -> Option<EntryRefMut> {
        self.plugins.get_mut(name)
    }

    ///List Plugins
    pub fn iter(&self) -> EntryIter {
        self.plugins.iter()
    }
}
