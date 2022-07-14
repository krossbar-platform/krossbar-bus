use std::{pin::Pin, time::Duration};

use async_trait::async_trait;
use caro_service::{
    service::{Service, ServiceMethods},
    signal::Signal,
    state::State,
};
use log::LevelFilter;

struct ServiceExample {
    signal: Signal<String>,
    state: State<i32>,
    counter: i32,
}

impl ServiceExample {
    pub fn new() -> Self {
        Self {
            signal: Signal::new(),
            state: State::new(),
            counter: 0,
        }
    }

    async fn hello_method(&mut self, value: i32) -> String {
        self.counter += 1;
        format!("Hello, {}", value + self.counter)
    }
}

#[async_trait]
impl Service for ServiceExample {
    async fn register_service(&mut self) -> caro_bus_lib::Result<()> {
        Self::register_bus("com.examples.register_state").await?;
        self.signal.register("signal")?;
        self.state.register("state", 0)?;
        Ok(())
    }
}

#[async_trait]
impl ServiceMethods for Pin<Box<ServiceExample>> {
    async fn register_methods(&mut self) -> caro_bus_lib::Result<()> {
        let context = caro_service::this::This { pointer: self };

        Self::register_method("method", move |p| async move {
            context.get().hello_method(p).await
        })?;
        Ok(())
    }
}

#[tokio::main]
async fn main() {
    pretty_env_logger::formatted_builder()
        .filter_level(LevelFilter::Trace)
        .init();

    let mut service = Box::pin(ServiceExample::new());

    service.register_service().await.unwrap();
    service.register_methods().await.unwrap();

    loop {
        service.signal.emit("Hello".into());
        service.state.set(42);
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
