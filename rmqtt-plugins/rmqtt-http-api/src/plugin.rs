use rmqtt::context::ServerContext;
use rmqtt::plugin::PluginInfo;
use rmqtt::Result;

#[inline]
pub(crate) async fn get_plugins(scx: &ServerContext) -> Result<Vec<PluginInfo>> {
    let mut plugins = Vec::new();
    for entry in scx.plugins.iter() {
        let info = entry.to_info(entry.key()).await?;
        plugins.push(info);
    }
    Ok(plugins)
}

#[inline]
pub(crate) async fn get_plugin(scx: &ServerContext, name: &str) -> Result<Option<PluginInfo>> {
    if let Some(entry) = scx.plugins.get(name) {
        match entry.to_info(entry.key()).await {
            Ok(p) => Ok(Some(p)),
            Err(e) => Err(e),
        }
    } else {
        Ok(None)
    }
}

#[inline]
pub(crate) async fn get_plugin_config(scx: &ServerContext, name: &str) -> Result<Vec<u8>> {
    let data = scx.plugins.get_config(name).await.map(|cfg| serde_json::to_vec(&cfg))??;
    Ok(data)
}
