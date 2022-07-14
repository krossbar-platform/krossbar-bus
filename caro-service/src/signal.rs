use serde::Serialize;

use caro_bus_common::errors::Error as BusError;
use caro_bus_lib::{signal::Signal as BusSignal, Result as BusResult};

pub struct Signal<T: Serialize> {
    internal: Option<BusSignal<T>>,
}

impl<T: Serialize + 'static> Signal<T> {
    pub fn new() -> Self {
        Self { internal: None }
    }

    pub fn register(&mut self, signal_name: &str) -> BusResult<()> {
        if self.internal.is_some() {
            return Err(Box::new(BusError::AlreadyRegistered));
        }

        match *crate::service::SERVICE_BUS.lock().unwrap() {
            Some(ref mut bus) => self.internal = Some(bus.register_signal(signal_name)?),
            _ => panic!("Not registered"),
        }

        Ok(())
    }

    pub fn emit(&self, value: T) {
        match self.internal {
            None => panic!("Not registered"),
            Some(ref internal) => internal.emit(value),
        }
    }
}
