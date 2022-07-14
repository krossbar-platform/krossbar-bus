use std::{future::Future, sync::Mutex};

use async_trait::async_trait;
use lazy_static::lazy_static;
use serde::{de::DeserializeOwned, Serialize};

use caro_bus_lib::{Bus, Result as BusResult};

lazy_static! {
    pub(crate) static ref SERVICE_BUS: Mutex<Option<Bus>> = Mutex::new(None);
}

#[async_trait]
pub trait Service {
    async fn register_bus(service_name: &str) -> BusResult<()> {
        let bus = Bus::register(service_name).await?;

        *SERVICE_BUS.lock().unwrap() = Some(bus);

        Ok(())
    }

    async fn register_service(&mut self) -> BusResult<()>;
}

#[async_trait]
pub trait ServiceMethods: Send + Sync + Sized {
    async fn register_methods(&mut self) -> BusResult<()>;

    fn register_method<P, R, Ret>(
        method_name: &str,
        callback: impl Fn(P) -> Ret + Send + Sync + 'static,
    ) -> BusResult<()>
    where
        P: DeserializeOwned + Send + 'static,
        R: Serialize + Send + 'static,
        Ret: Future<Output = R> + Send,
    {
        match *SERVICE_BUS.lock().unwrap() {
            Some(ref mut bus) => {
                bus.register_method::<P, R, Ret>(method_name, callback)?;
            }
            _ => panic!("Not registered"),
        }

        Ok(())
    }
}
