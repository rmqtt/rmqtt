use async_trait::async_trait;
use rmqtt::{plugin::Plugin, Result, Runtime};

#[inline]
pub async fn init<N: Into<String>, D: Into<String>>(
    runtime: &'static Runtime,
    name: N,
    descr: D,
    default_startup: bool,
) -> Result<()> {
    runtime
        .plugins
        .register(
            Box::new(Helloworld::new(runtime, name.into(), descr.into())),
            default_startup,
        )
        .await?;
    Ok(())
}

struct Helloworld {
    _runtime: &'static Runtime,
    name: String,
    descr: String,
}

impl Helloworld {
    #[inline]
    fn new(runtime: &'static Runtime, name: String, descr: String) -> Self {
        Self {
            _runtime: runtime,
            name,
            descr,
        }
    }
}

#[async_trait]
impl Plugin for Helloworld {
    #[inline]
    async fn init(&mut self) -> Result<()> {
        log::info!("{} init", self.name);
        Ok(())
    }

    #[inline]
    async fn start(&mut self) -> Result<()> {
        log::info!("{} start", self.name);
        Ok(())
    }

    #[inline]
    async fn stop(&mut self) -> Result<bool> {
        log::info!("{} stop", self.name);
        Ok(true)
    }

    #[inline]
    fn version(&self) -> &str {
        "0.1.1"
    }

    #[inline]
    fn name(&self) -> &str {
        &self.name
    }

    #[inline]
    fn descr(&self) -> &str {
        &self.descr
    }
}
