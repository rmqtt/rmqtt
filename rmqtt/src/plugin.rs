//! MQTT Broker Plugin Management System
//!
//! Provides a robust plugin architecture with:
//! - Dynamic loading/unloading
//! - Lifecycle management
//! - Configuration handling
//! - Inter-plugin communication
//!
//! ## Core Functionality
//! 1. ​**​Plugin Lifecycle​**​:
//!    - Registration and initialization
//!    - Startup/shutdown sequencing
//!    - Immutable plugin support
//!    - State tracking (active/inactive)
//!
//! 2. ​**​Configuration Management​**​:
//!    - File-based configuration
//!    - Environment variable overrides
//!    - Default value handling
//!    - Runtime reload capability
//!
//! 3. ​**​Plugin Operations​**​:
//!    - Metadata inspection
//!    - Message passing
//!    - Thread-safe access
//!    - Dependency management
//!
//! ## Key Features
//! - Async-friendly interface
//! - Atomic state transitions
//! - Flexible configuration system
//! - Plugin isolation
//! - Comprehensive metadata
//!
//! ## Implementation Details
//! - DashMap for concurrent storage
//! - Async trait patterns
//! - Type-erased plugin instances
//! - JSON-based configuration
//! - Environment-aware config loading
//!
//! Usage Patterns:
//! 1. Implement `Plugin` trait for custom functionality
//! 2. Register with `register!` macro
//! 3. Manage via `Manager` interface:
//!    - `start()`/`stop()`
//!    - `load_config()`
//!    - `send()` messages
//! 4. Query plugin info/metadata
//!
//! Note: Plugins can be marked immutable to prevent
//! runtime modifications for critical components.

use std::future::Future;
use std::path::Path;
use std::pin::Pin;

use anyhow::anyhow;
use async_trait::async_trait;
use config::FileFormat::Toml;
use config::{Config, File, Source};
use dashmap::iter::Iter;
use dashmap::mapref::one::{Ref, RefMut};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::types::{DashMap, HashMap};
use crate::Result;

/// Reference to a plugin entry in the manager's concurrent map.
pub type EntryRef<'a> = Ref<'a, String, Entry>;
/// Mutable reference to a plugin entry.
pub type EntryRefMut<'a> = RefMut<'a, String, Entry>;
/// Iterator over all registered plugin entries.
pub type EntryIter<'a> = Iter<'a, String, Entry, ahash::RandomState, DashMap<String, Entry>>;

/// Registers a plugin with the broker's plugin manager.
///
/// This macro generates two functions:
/// - `register(scx, default_startup, immutable)` — registers using
///   the crate name from `CARGO_PKG_NAME`.
/// - `register_named(scx, name, default_startup, immutable)` — registers
///   with an explicit name.
///
/// # Usage
///
/// ```ignore
/// register!(my_plugin::new);
/// ```
///
/// The provided closure receives `(ServerContext, &str)` and must
/// return `Result<DynPlugin>`.
///
/// # Arguments
///
/// * `$name` — A factory function that creates the plugin instance.
///   Signature: `async fn(ServerContext, &str) -> Result<impl Plugin>`
#[macro_export]
macro_rules! register {
    ($name:path) => {
        #[inline]
        pub async fn register_named(
            scx: &rmqtt::context::ServerContext,
            name: &'static str,
            default_startup: bool,
            immutable: bool,
        ) -> rmqtt::Result<()> {
            let scx1 = scx.clone();
            scx.plugins
                .register(name, default_startup, immutable, move || -> rmqtt::plugin::DynPluginResult {
                    let scx1 = scx1.clone();
                    Box::pin(async move {
                        $name(scx1.clone(), name).await.map(|p| -> rmqtt::plugin::DynPlugin { Box::new(p) })
                    })
                })
                .await?;
            Ok(())
        }

        #[inline]
        pub async fn register(
            scx: &rmqtt::context::ServerContext,
            default_startup: bool,
            immutable: bool,
        ) -> rmqtt::Result<()> {
            let name = env!("CARGO_PKG_NAME");
            register_named(scx, env!("CARGO_PKG_NAME"), default_startup, immutable).await
        }
    };
}

/// Core trait for all RMQTT plugins.
///
/// Every plugin must implement this trait along with [`PackageInfo`].
/// The trait defines the plugin lifecycle: initialization, configuration
/// loading, start, and stop.
///
/// # Lifecycle
///
/// 1. **Construction** — The plugin factory function is called.
/// 2. [`init`](Plugin::init) — Called once after construction.
///    Default implementation is a no-op.
/// 3. [`load_config`](Plugin::load_config) — Load plugin-specific configuration.
///    Default returns "unimplemented!" error.
/// 4. [`start`](Plugin::start) — Activate the plugin. Default is a no-op.
/// 5. [`stop`](Plugin::stop) — Deactivate the plugin. Returns `true` if
///    the plugin fully stopped, `false` if it declined.
///
/// # Message Passing
///
/// Plugins can receive ad-hoc messages via [`send`](Plugin::send),
/// enabling plugin-to-plugin communication and runtime management.
///
/// # Default Implementations
///
/// Most methods have default no-op implementations so plugins only
/// need to override the relevant lifecycle hooks.
#[async_trait]
pub trait Plugin: PackageInfo + Send + Sync {
    /// Initialize the plugin after construction.
    ///
    /// Called once before `start`. Use this for one-time setup
    /// that does not depend on other plugins being active.
    async fn init(&mut self) -> Result<()> {
        Ok(())
    }

    /// Return the plugin's current configuration as a JSON value.
    ///
    /// Used by the management API to expose plugin configuration.
    async fn get_config(&self) -> Result<serde_json::Value> {
        Ok(json!({}))
    }

    /// Load and apply the plugin's configuration from its config source.
    ///
    /// # Errors
    ///
    /// Returns an error if configuration loading or parsing fails.
    /// The default implementation returns "unimplemented!".
    async fn load_config(&mut self) -> Result<()> {
        Err(anyhow!("unimplemented!"))
    }

    /// Start the plugin.
    ///
    /// Called after `init`. The plugin should begin its active
    /// processing (e.g., register hooks, start background tasks).
    async fn start(&mut self) -> Result<()> {
        Ok(())
    }

    /// Stop the plugin.
    ///
    /// # Returns
    ///
    /// * `true` — Plugin stopped successfully and can be considered inactive.
    /// * `false` — Plugin declined to stop (e.g., it is essential for operation).
    async fn stop(&mut self) -> Result<bool> {
        Ok(true)
    }

    /// Return plugin-specific runtime attributes as JSON.
    ///
    /// Used for exposing dynamic state via the management API.
    async fn attrs(&self) -> serde_json::Value {
        serde_json::Value::Null
    }

    /// Handle an incoming message for plugin-to-plugin communication.
    ///
    /// # Returns
    ///
    /// A JSON value as the response. The default returns `null`.
    async fn send(&self, _msg: serde_json::Value) -> Result<serde_json::Value> {
        Ok(serde_json::Value::Null)
    }
}

/// Provides package metadata for a plugin.
///
/// Plugins implement this trait (often via a blanket derive or manual
/// implementation) to expose their identity and provenance information.
///
/// # Required
///
/// Only `name()` is mandatory. All other methods return `None` or
/// default values if not overridden.
pub trait PackageInfo {
    /// Unique name of the plugin (e.g., `"rmqtt-acl"`).
    fn name(&self) -> &str;

    /// Semantic version string (e.g., `"0.22.0"`).
    fn version(&self) -> &str {
        "0.0.0"
    }

    /// Short description of the plugin's purpose.
    fn descr(&self) -> Option<&str> {
        None
    }

    /// List of plugin authors.
    fn authors(&self) -> Option<Vec<&str>> {
        None
    }

    /// Plugin project homepage URL.
    fn homepage(&self) -> Option<&str> {
        None
    }

    /// Plugin license identifier.
    fn license(&self) -> Option<&str> {
        None
    }

    /// Plugin source repository URL.
    fn repository(&self) -> Option<&str> {
        None
    }
}

/// Boxed, pinned, and sendable future produced by a plugin factory.
type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// Trait for plugin factory functions.
///
/// Implemented automatically for closures matching the signature
/// `Fn() -> BoxFuture<Result<DynPlugin>>`.
pub trait PluginFn: 'static + Sync + Send + Fn() -> BoxFuture<Result<DynPlugin>> {}

impl<T> PluginFn for T where T: 'static + Sync + Send + ?Sized + Fn() -> BoxFuture<Result<DynPlugin>> {}

/// Boxed future returning a dynamically typed plugin instance.
pub type DynPluginResult = BoxFuture<Result<DynPlugin>>;
/// Heap-allocated, type-erased plugin instance.
pub type DynPlugin = Box<dyn Plugin>;
/// Heap-allocated, type-erased plugin factory function.
pub type DynPluginFn = Box<dyn PluginFn>;

/// A registered plugin entry in the manager.
///
/// Tracks the plugin instance, lifecycle state, and whether the
/// plugin can be modified at runtime.
///
/// # States
///
/// * **Immutable** — Plugin cannot be stopped, started, or have
///   its configuration loaded at runtime. Used for critical plugins.
/// * **Inited** — Plugin has been initialized but not yet started.
/// * **Active** — Plugin is running and processing events.
///
/// A plugin may exist without an instance (`plugin: None`) if it
/// was registered but not started (lazy initialization via `plugin_f`).
pub struct Entry {
    /// Whether [`Plugin::init`] has been called.
    inited: bool,
    /// Whether [`Plugin::start`] has been called and the plugin is active.
    active: bool,
    /// Whether runtime lifecycle operations are blocked.
    immutable: bool,
    /// The initialized plugin instance, if any.
    plugin: Option<DynPlugin>,
    /// Factory function for lazy plugin creation.
    plugin_f: Option<DynPluginFn>,
}

impl Entry {
    /// Whether the plugin has been initialized.
    #[inline]
    pub fn inited(&self) -> bool {
        self.inited
    }

    /// Whether the plugin is currently active (started).
    #[inline]
    pub fn active(&self) -> bool {
        self.active
    }

    /// Whether the plugin is immutable (cannot be modified at runtime).
    #[inline]
    pub fn immutable(&self) -> bool {
        self.immutable
    }

    #[inline]
    async fn plugin(&self) -> Result<&dyn Plugin> {
        if let Some(plugin) = &self.plugin {
            Ok(plugin.as_ref())
        } else {
            Err(anyhow!("the plug-in is not initialized"))
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
            Err(anyhow!("the plug-in is not initialized"))
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

/// Serializable snapshot of a plugin's metadata and runtime state.
///
/// Used by the management API to expose plugin information
/// to operators and external systems.
#[derive(Deserialize, Serialize, Debug, Clone, Default)]
pub struct PluginInfo {
    /// Plugin name (e.g., `"rmqtt-acl"`).
    pub name: String,
    /// Semantic version, if available.
    pub version: Option<String>,
    /// Human-readable description.
    pub descr: Option<String>,
    /// List of authors.
    pub authors: Option<Vec<String>>,
    /// Project homepage URL.
    pub homepage: Option<String>,
    /// License identifier.
    pub license: Option<String>,
    /// Source repository URL.
    pub repository: Option<String>,

    /// Whether the plugin has been initialized.
    pub inited: bool,
    /// Whether the plugin is currently active.
    pub active: bool,
    /// Whether runtime modifications are blocked.
    pub immutable: bool,
    /// JSON-encoded runtime attributes.
    pub attrs: Vec<u8>,
}

impl PluginInfo {
    #[inline]
    /// Serialize this plugin info to a JSON value for the management API.
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

/// Configuration for the plugin [`Manager`].
///
/// Specifies where to load plugin configuration files and allows
/// inline config overrides by plugin name.
///
/// # Configuration Sources
///
/// Plugin configs are loaded in order of precedence:
/// 1. Environment variables with prefix `rmqtt_plugin_{name}`.
/// 2. Inline config strings added via [`add`](PluginManagerConfig::add).
/// 3. TOML files in the configured [`path`](PluginManagerConfig::path).
#[derive(Default)]
pub struct PluginManagerConfig {
    pub(crate) path: Option<String>,
    pub(crate) map: HashMap<String, String>,
}

impl PluginManagerConfig {
    /// Set the directory path for plugin configuration files.
    ///
    /// The manager will look for `{path}/{plugin_name}.toml` files.
    pub fn path(mut self, path: String) -> Self {
        self.path = Some(path);
        self
    }

    /// Merge a map of inline config strings by plugin name.
    ///
    /// These override file-based configuration for the specified plugins.
    pub fn map(mut self, map: HashMap<String, String>) -> Self {
        self.map.extend(map);
        self
    }

    /// Add a single inline config string for a plugin.
    pub fn add(mut self, name: String, cfg: String) -> Self {
        self.map.insert(name, cfg);
        self
    }
}

/// Manages all registered plugins in the broker.
///
/// Provides CRUD-style operations for plugin lifecycle:
/// registration, start, stop, configuration loading, and inspection.
///
/// # Thread Safety
///
/// Uses [`DashMap`] internally for lock-free concurrent access.
/// The `read_config*` methods are synchronous, while lifecycle
/// operations are async.
///
/// # Immutable Plugins
///
/// Some plugins (e.g., storage backends) are marked immutable
/// to prevent accidental runtime modification. Attempting to
/// call `get_mut`, `stop`, or `load_config` on an immutable
/// plugin returns an error.
pub struct Manager {
    plugins: DashMap<String, Entry>,
    config: PluginManagerConfig,
}

impl Manager {
    pub(crate) fn new(config: PluginManagerConfig) -> Self {
        Self { plugins: DashMap::default(), config }
    }

    /// Register a plugin with the manager.
    ///
    /// If a plugin with the same name already exists and is active,
    /// it will be stopped before replacement.
    ///
    /// # Arguments
    ///
    /// * `name` — Plugin name (e.g., `"rmqtt-acl"`).
    /// * `default_startup` — If true, the plugin is initialized and
    ///   started immediately. If false, the factory is stored for
    ///   lazy initialization on first use.
    /// * `immutable` — If true, runtime lifecycle operations
    ///   (stop, load config) are blocked.
    /// * `plugin_f` — Factory closure that creates the plugin instance.
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

    /// Return the current configuration of a plugin.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin does not exist.
    pub async fn get_config(&self, name: &str) -> Result<serde_json::Value> {
        if let Some(entry) = self.get(name) {
            entry.plugin().await?.get_config().await
        } else {
            Err(anyhow!(format!("{} the plug-in does not exist", name)))
        }
    }

    /// Load and apply configuration for a plugin.
    ///
    /// The plugin must be initialized before configuration
    /// can be loaded. Immutable plugins are rejected.
    pub async fn load_config(&self, name: &str) -> Result<()> {
        if let Some(mut entry) = self.get_mut(name)? {
            if entry.inited {
                entry.plugin_mut().await?.load_config().await?;
                Ok(())
            } else {
                Err(anyhow!("the plug-in is not initialized"))
            }
        } else {
            Err(anyhow!(format!("{} the plug-in does not exist", name)))
        }
    }

    /// Start a plugin.
    ///
    /// If the plugin has not been initialized yet, it will be
    /// initialized first. Has no effect if the plugin is already active.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin does not exist, is immutable,
    /// or if initialization/startup fails.
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
            Err(anyhow!(format!("{} the plug-in does not exist", name)))
        }
    }

    /// Stop a plugin.
    ///
    /// # Returns
    ///
    /// `true` if the plugin fully stopped. `false` if the plugin
    /// declined to stop.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin does not exist, is immutable,
    /// or is not currently active.
    pub async fn stop(&self, name: &str) -> Result<bool> {
        if let Some(mut entry) = self.get_mut(name)? {
            if entry.active {
                let stopped = entry.plugin_mut().await?.stop().await?;
                entry.active = !stopped;
                Ok(stopped)
            } else {
                Err(anyhow!(format!("{} the plug-in is not started", name)))
            }
        } else {
            Err(anyhow!(format!("{} the plug-in does not exist", name)))
        }
    }

    /// Check whether a plugin is currently active.
    pub fn is_active(&self, name: &str) -> bool {
        if let Some(entry) = self.plugins.get(name) {
            entry.active()
        } else {
            false
        }
    }

    /// Get a reference to a plugin entry by name.
    pub fn get(&self, name: &str) -> Option<EntryRef<'_>> {
        self.plugins.get(name)
    }

    /// Get a mutable reference to a plugin entry by name.
    ///
    /// # Errors
    ///
    /// Returns an error if the plugin is immutable.
    pub fn get_mut(&self, name: &str) -> Result<Option<EntryRefMut<'_>>> {
        if let Some(entry) = self.plugins.get_mut(name) {
            if entry.immutable {
                Err(anyhow!("the plug-in is immutable"))
            } else {
                Ok(Some(entry))
            }
        } else {
            Ok(None)
        }
    }

    /// Send a message to a plugin and receive a response.
    pub async fn send(&self, name: &str, msg: serde_json::Value) -> Result<serde_json::Value> {
        if let Some(entry) = self.plugins.get(name) {
            entry.plugin().await?.send(msg).await
        } else {
            Err(anyhow!(format!("{} the plug-in does not exist", name)))
        }
    }

    /// Iterate over all registered plugins.
    pub fn iter(&self) -> EntryIter<'_> {
        self.plugins.iter()
    }

    /// Read a plugin's configuration and deserialize it.
    ///
    /// Configuration is loaded from (in order of precedence):
    /// 1. Environment variables `rmqtt_plugin_{name}_*`
    /// 2. Inline config strings
    /// 3. `{config_path}/{name}.toml` files
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration is missing or invalid.
    pub fn read_config<'de, T: serde::Deserialize<'de>>(&self, name: &str) -> Result<T> {
        let (cfg, _) = self.read_config_with_required(name, true, &[])?;
        Ok(cfg)
    }

    /// Read a plugin's configuration, using defaults if no config exists.
    ///
    /// Logs a warning when falling back to defaults.
    pub fn read_config_default<'de, T: serde::Deserialize<'de>>(&self, name: &str) -> Result<T> {
        let (cfg, def) = self.read_config_with_required(name, false, &[])?;
        if def {
            log::warn!("The configuration for plugin '{name}' does not exist, default values will be used!");
        }
        Ok(cfg)
    }

    /// Read a plugin's configuration with environment variable list key support.
    ///
    /// `env_list_keys` specifies which configuration keys should be parsed
    /// as space-separated lists from environment variables.
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration is missing or invalid.
    pub fn read_config_with<'de, T: serde::Deserialize<'de>>(
        &self,
        name: &str,
        env_list_keys: &[&str],
    ) -> Result<T> {
        let (cfg, _) = self.read_config_with_required(name, true, env_list_keys)?;
        Ok(cfg)
    }

    /// Read a plugin's configuration with list key support, using defaults if missing.
    ///
    /// Like [`read_config_default`](Self::read_config_default), but supports
    /// environment variable list parsing for the specified keys.
    pub fn read_config_default_with<'de, T: serde::Deserialize<'de>>(
        &self,
        name: &str,
        env_list_keys: &[&str],
    ) -> Result<T> {
        let (cfg, def) = self.read_config_with_required(name, false, env_list_keys)?;
        if def {
            log::warn!("The configuration for plugin '{name}' does not exist, default values will be used!");
        }
        Ok(cfg)
    }

    /// Core configuration reader with required flag and environment list key support.
    ///
    /// # Returns
    ///
    /// A tuple of `(deserialized_config, is_default)` where `is_default`
    /// indicates whether no configuration file was found and defaults were used.
    ///
    /// # Configuration Sources (in order)
    ///
    /// 1. `{config_path}/{name}.toml` file
    /// 2. Inline config string in the config map
    /// 3. Environment variables with prefix `rmqtt_plugin_{name}`
    ///
    /// If `required` is true, at least one source must exist.
    /// If `env_list_keys` is non-empty, those keys are parsed as
    /// space-separated lists from environment variables.
    pub fn read_config_with_required<'de, T: serde::Deserialize<'de>>(
        &self,
        name: &str,
        required: bool,
        env_list_keys: &[&str],
    ) -> Result<(T, bool)> {
        let builder = if let Some(path) = &self.config.path {
            let path = path.trim_end_matches(['/', '\\']);
            let path = format!("{path}/{name}.toml");
            let path = Path::new(path.as_str());
            if path.is_file() {
                Some(Config::builder().add_source(File::from(path).required(required)))
            } else {
                None
            }
        } else {
            None
        };

        let builder = match builder {
            Some(builder) => Some(builder),
            None => self.config.map.get(name).map(|config_string| {
                Config::builder().add_source(File::from_str(config_string, Toml).required(required))
            }),
        };

        let mut builder = if required {
            builder.ok_or_else(|| {
                anyhow!(format!("plugin configuration not found, the plugin name is: {name}"))
            })?
        } else {
            builder.unwrap_or_default()
        };

        let mut env = config::Environment::with_prefix(&format!("rmqtt_plugin_{}", name.replace('-', "_")));
        if !env_list_keys.is_empty() {
            env = env.try_parsing(true).list_separator(" ");
            for key in env_list_keys {
                env = env.with_list_parse_key(key);
            }
        }
        builder = builder.add_source(env);

        let s = builder.build()?;
        let count = s.collect()?.len();
        Ok((s.try_deserialize::<T>()?, count == 0))
    }
}
