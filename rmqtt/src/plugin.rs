use core::pin::Pin;
use std::future::Future;

use dashmap::iter::Iter;
use dashmap::mapref::one::{Ref, RefMut};

use crate::{MqttError, Result};

type DashMap<K, V> = dashmap::DashMap<K, V, ahash::RandomState>;
pub type EntryRef<'a> = Ref<'a, String, Entry, ahash::RandomState>;
pub type EntryRefMut<'a> = RefMut<'a, String, Entry, ahash::RandomState>;
pub type EntryIter<'a> = Iter<'a, String, Entry, ahash::RandomState, DashMap<String, Entry>>;

#[macro_export]
macro_rules! register {
    ($name:path) => {
        #[inline]
        pub async fn register(
            runtime: &'static rmqtt::Runtime,
            name: &'static str,
            default_startup: bool,
            immutable: bool,
        ) -> Result<()> {
            runtime
                .plugins
                .register(name, default_startup, immutable, move || -> rmqtt::plugin::DynPluginResult {
                    Box::pin(async move {
                        $name(runtime, name).await.map(|p| -> rmqtt::plugin::DynPlugin { Box::new(p) })
                    })
                })
                .await?;
            Ok(())
        }
    };
}

#[async_trait]
pub trait Plugin: PackageInfo + Send + Sync {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        Ok(())
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
    async fn attrs(&self) -> serde_json::Value {
        serde_json::Value::Null
    }

    #[inline]
    async fn send(&self, _msg: serde_json::Value) -> Result<serde_json::Value> {
        Ok(serde_json::Value::Null)
    }
}

pub trait PackageInfo {
    fn name(&self) -> &str;

    #[inline]
    fn version(&self) -> &str {
        "0.0.0"
    }

    #[inline]
    fn descr(&self) -> Option<&str> {
        None
    }

    #[inline]
    fn authors(&self) -> Option<Vec<&str>> {
        None
    }

    #[inline]
    fn homepage(&self) -> Option<&str> {
        None
    }

    #[inline]
    fn license(&self) -> Option<&str> {
        None
    }

    #[inline]
    fn repository(&self) -> Option<&str> {
        None
    }
}

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;
// type LocalBoxFuture<T> = Pin<Box<dyn Future<Output = T>>>;

pub trait PluginFn: 'static + Sync + Send + Fn() -> BoxFuture<Result<DynPlugin>> {}

impl<T> PluginFn for T where T: 'static + Sync + Send + ?Sized + Fn() -> BoxFuture<Result<DynPlugin>> {}

pub type DynPluginResult = BoxFuture<Result<DynPlugin>>;
pub type DynPlugin = Box<dyn Plugin>;
pub type DynPluginFn = Box<dyn PluginFn>;

pub struct Entry {
    inited: bool,
    active: bool,
    //will reject start, stop, and load config operations
    immutable: bool,
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
    pub fn immutable(&self) -> bool {
        self.immutable
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
    pub async fn to_info(&self, name: &str) -> Result<PluginInfo> {
        if let Ok(plugin) = self.plugin().await {
            let attrs = serde_json::to_vec(&plugin.attrs().await)?;
            Ok(PluginInfo {
                name: plugin.name().to_owned(),
                version: Some(plugin.version().to_owned()),
                descr: plugin.descr().map(String::from),
                authors: plugin.authors().map(|authors| authors.into_iter().map(String::from).collect()),
                homepage: plugin.homepage().map(String::from),
                license: plugin.license().map(String::from),
                repository: plugin.repository().map(String::from),

                inited: self.inited,
                active: self.active,
                immutable: self.immutable,
                attrs,
            })
        } else {
            Ok(PluginInfo {
                name: name.to_owned(),
                inited: self.inited,
                active: self.active,
                immutable: self.immutable,
                ..Default::default()
            })
        }
    }
}

#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct PluginInfo {
    pub name: String,
    pub version: Option<String>,
    pub descr: Option<String>,
    pub authors: Option<Vec<String>>,
    pub homepage: Option<String>,
    pub license: Option<String>,
    pub repository: Option<String>,

    pub inited: bool,
    pub active: bool,
    pub immutable: bool,
    pub attrs: Vec<u8>, //json data
}

impl PluginInfo {
    #[inline]
    pub fn to_json(&self) -> Result<serde_json::Value> {
        let attrs = if self.attrs.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&self.attrs)?
        };
        Ok(json!({
            "name": self.name,
            "version": self.version,
            "descr": self.descr,
            "authors": self.authors,
            "homepage": self.homepage,
            "license": self.license,
            "repository": self.repository,

            "inited": self.inited,
            "active": self.active,
            "immutable": self.immutable,
            "attrs": attrs,
        }))
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
        immutable: bool,
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

        let entry = Entry { inited: default_startup, active: default_startup, immutable, plugin, plugin_f };
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
        if let Some(mut entry) = self.get_mut(name)? {
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
        if let Some(mut entry) = self.get_mut(name)? {
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
        if let Some(mut entry) = self.get_mut(name)? {
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
    pub fn get_mut(&self, name: &str) -> Result<Option<EntryRefMut>> {
        if let Some(entry) = self.plugins.get_mut(name) {
            if entry.immutable {
                Err(MqttError::from("the plug-in is immutable"))
            } else {
                Ok(Some(entry))
            }
        } else {
            Ok(None)
        }
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
