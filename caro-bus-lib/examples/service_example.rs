use async_trait::async_trait;
use caro_bus_lib::service::*;

struct Service {
    pub signal: Signal<String>,
    pub state: State<i32>,
}

impl Service {
    pub async fn new() -> caro_bus_lib::Result<Self> {
        let mut this = Self {
            signal: Signal::new(),
            state: State::new(),
        };

        this.register_service().await?;
        Ok(this)
    }

    async fn hello_method(&mut self, value: i32) -> String {
        format!("Hello, {}", value)
    }
}

#[async_trait]
impl MacroService for Service {
    async fn register_service(&mut self) -> caro_bus_lib::Result<()> {
        Self::register_bus("caro.service.example").await?;
        self.signal.register("signal")?;
        self.state.register("state", 0)?;
        Ok(())
    }
}

#[async_trait]
impl RegisterMethods for Service {
    async fn register_methods(&mut self) -> caro_bus_lib::Result<()> {
        let context = SelfMethod { pointer: self };

        Self::register_method("method", move |p| {
            Box::pin(async move { context.get().hello_method(p).await })
        })?;
        Ok(())
    }
}

#[tokio::main]
async fn main() {
    let mut service = Service::new().await.unwrap();
    service.signal.emit("Hello".into()).unwrap();
    service.state.set(42).unwrap();
}
