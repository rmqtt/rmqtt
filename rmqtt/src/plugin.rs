use core::pin::Pin;
use std::future::Future;

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
    async fn get_config(&self) -> Result<serde_json::Value> {
        Ok(json!({}))
    }

    #[inline]
    async fn load_config(&mut self) -> Result<()> {
        Err(MqttError::from("unimplemented!"))
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
    async fn attrs(&self) -> serde_json::Value {
        serde_json::Value::Null
    }

    #[inline]
    async fn send(&self, _msg: serde_json::Value) -> Result<serde_json::Value> {
        Ok(serde_json::Value::Null)
    }
}

type BoxFuture<T> = Pin<Box<dyn Future<Output=T> + Send>>;
// type LocalBoxFuture<T> = Pin<Box<dyn Future<Output = T>>>;

pub trait PluginFn: 'static + Sync + Send + Fn() -> BoxFuture<Result<DynPlugin>> {}

impl<T> PluginFn for T where T: 'static + Sync + Send + ?Sized + Fn() -> BoxFuture<Result<DynPlugin>> {}

pub type DynPluginResult = BoxFuture<Result<DynPlugin>>;
pub type DynPlugin = Box<dyn Plugin>;
pub type DynPluginFn = Box<dyn PluginFn>;

pub struct Entry {
    inited: bool,
    active: bool,
    //immutable: bool, //will reject start, stop, and load config operations
    plugin: Option<DynPlugin>,
    plugin_f: Option<DynPluginFn>,
}

impl Entry {
    #[inline]
    pub fn inited(&self) -> bool {
        self.inited
    }

    #[inline]
    pub fn active(&self) -> bool {
        self.active
    }

    #[inline]
    async fn plugin(&self) -> Result<&dyn Plugin> {
        if let Some(plugin) = &self.plugin {
            Ok(plugin.as_ref())
        } else {
            Err(MqttError::from("the plug-in is not initialized"))
        }
    }

    #[inline]
    async fn plugin_mut(&mut self) -> Result<&mut dyn Plugin> {
        if let Some(plugin_f) = self.plugin_f.take() {
            self.plugin.replace(plugin_f().await?);
        }

        if let Some(plugin) = self.plugin.as_mut() {
            Ok(plugin.as_mut())
        } else {
            Err(MqttError::from("the plug-in is not initialized"))
        }
    }

    #[inline]
    pub async fn to_json(&self, name: &str) -> Result<serde_json::Value> {
        if let Ok(plugin) = self.plugin().await {
            Ok(json!({
                "name": plugin.name(),
                "version": plugin.version(),
                "descr": plugin.descr(),
                "inited": self.inited,
                "active": self.active,
                "attrs": plugin.attrs().await,
            }))
        } else {
            Ok(json!({
                "name": name,
                "inited": self.inited,
                "active": self.active,
            }))
        }
    }
}

pub struct Manager {
    plugins: DashMap<String, Entry>,
}

impl Manager {
    pub(crate) fn new() -> Self {
        Self { plugins: DashMap::default() }
    }

    ///Register a Plugin
    pub async fn register<N: Into<String>, F: PluginFn>(
        &self,
        name: N,
        default_startup: bool,
        plugin_f: F,
    ) -> Result<()> {
        let name = name.into();

        if let Some((_, mut entry)) = self.plugins.remove(&name) {
            if entry.active {
                entry.plugin_mut().await?.stop().await?;
            }
        }

        let (plugin, plugin_f) = if default_startup {
            let mut plugin = plugin_f().await?;
            plugin.init().await?;
            plugin.start().await?;
            (Some(plugin), None)
        } else {
            let boxed_f: Box<dyn PluginFn> = Box::new(plugin_f);
            (None, Some(boxed_f))
        };

        let entry = Entry { inited: default_startup, active: default_startup, plugin, plugin_f };
        self.plugins.insert(name, entry);
        Ok(())
    }

    ///Return Config
    pub async fn get_config(&self, name: &str) -> Result<serde_json::Value> {
        if let Some(entry) = self.get(name) {
            entry.plugin().await?.get_config().await
        } else {
            Err(MqttError::from(format!("{} the plug-in does not exist", name)))
        }
    }

    ///Load Config
    pub async fn load_config(&self, name: &str) -> Result<()> {
        if let Some(mut entry) = self.get_mut(name) {
            if entry.inited {
                entry.plugin_mut().await?.load_config().await?;
                Ok(())
            } else {
                Err(MqttError::from("the plug-in is not initialized"))
            }
        } else {
            Err(MqttError::from(format!("{} the plug-in does not exist", name)))
        }
    }

    ///Start a Plugin
    pub async fn start(&self, name: &str) -> Result<()> {
        if let Some(mut entry) = self.get_mut(name) {
            if !entry.inited {
                entry.plugin_mut().await?.init().await?;
                entry.inited = true;
            }
            if !entry.active {
                entry.plugin_mut().await?.start().await?;
                entry.active = true;
            }
            Ok(())
        } else {
            Err(MqttError::from(format!("{} the plug-in does not exist", name)))
        }
    }

    ///Stop a Plugin
    pub async fn stop(&self, name: &str) -> Result<bool> {
        if let Some(mut entry) = self.get_mut(name) {
            if entry.active {
                let stopped = entry.plugin_mut().await?.stop().await?;
                entry.active = !stopped;
                Ok(stopped)
            } else {
                Err(MqttError::from(format!("{} the plug-in is not started", name)))
            }
        } else {
            Err(MqttError::from(format!("{} the plug-in does not exist", name)))
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

    ///Sending messages to plug-in
    pub async fn send(&self, name: &str, msg: serde_json::Value) -> Result<serde_json::Value> {
        if let Some(entry) = self.plugins.get(name) {
            entry.plugin().await?.send(msg).await
        } else {
            Err(MqttError::from(format!("{} the plug-in does not exist", name)))
        }
    }

    ///List Plugins
    pub fn iter(&self) -> EntryIter {
        self.plugins.iter()
    }
}
