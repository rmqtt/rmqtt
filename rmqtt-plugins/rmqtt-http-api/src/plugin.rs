use rmqtt::plugin::PluginInfo;
use rmqtt::serde_json;
use rmqtt::{Result, Runtime};

#[inline]
pub(crate) async fn get_plugins() -> Result<Vec<PluginInfo>> {
    let mut plugins = Vec::new();
    for entry in Runtime::instance().plugins.iter() {
        let info = entry.to_info(entry.key()).await?;
        plugins.push(info);
    }
    Ok(plugins)
}

#[inline]
pub(crate) async fn get_plugin(name: &str) -> Result<Option<PluginInfo>> {
    if let Some(entry) = Runtime::instance().plugins.get(name) {
        match entry.to_info(entry.key()).await {
            Ok(p) => Ok(Some(p)),
            Err(e) => Err(e),
        }
    } else {
        Ok(None)
    }
}

#[inline]
pub(crate) async fn get_plugin_config(name: &str) -> Result<Vec<u8>> {
    let data = Runtime::instance().plugins.get_config(name).await.map(|cfg| serde_json::to_vec(&cfg))??;
    Ok(data)
}
