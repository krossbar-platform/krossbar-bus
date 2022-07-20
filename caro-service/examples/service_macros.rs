use std::{pin::Pin, time::Duration};

use caro_service::{service::ServiceMethods, Service as CaroService};
use log::LevelFilter;

use async_trait::async_trait;
use caro_macros::{method, service_impl, Service};
use caro_service::Signal;
use caro_service::State;

#[derive(Service)]
#[service_name("com.examples.service")]
struct ServiceExample {
    //peer: Pin<Box<PeerExample>>,
    #[signal]
    signal: Signal<String>,
    #[state(0)]
    state: State<i32>,
    counter: i32,
}

#[service_impl]
impl ServiceExample {
    pub fn new() -> Self {
        Self {
            //peer: Box::pin(PeerExample::new()),
            signal: Signal::new(),
            state: State::new(),
            counter: 0,
        }
    }

    #[method]
    async fn hello_method(&mut self, value: i32) -> String {
        self.counter += 1;
        format!("Hello, {}", value + self.counter)
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