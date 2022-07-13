use std::{future::Future, sync::Mutex};

use async_trait::async_trait;
use lazy_static::lazy_static;
use serde::{de::DeserializeOwned, Serialize};

use crate::bus::Bus;
use caro_bus_common::errors::Error as BusError;

lazy_static! {
    pub static ref SERVICE_BUS: Mutex<Option<Bus>> = Mutex::new(None);
}

pub trait Service {
    fn init(&mut self);
    fn print(&self);
}

#[async_trait]
pub trait MacroService {
    async fn register_bus(service_name: &str) -> crate::Result<()> {
        let bus = Bus::register(service_name).await?;

        *SERVICE_BUS.lock().unwrap() = Some(bus);

        Ok(())
    }

    async fn register_service(&mut self) -> crate::Result<()>;
}

#[async_trait]
pub trait RegisterMethods: Send + Sync + Sized {
    async fn register_methods(&mut self) -> crate::Result<()>;

    fn register_method<P, R, Ret>(
        method_name: &str,
        callback: impl Fn(P) -> Ret + Send + Sync + 'static,
    ) -> crate::Result<()>
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

pub struct SelfMethod<T: Send + Sync + 'static> {
    pub pointer: *mut T,
}

impl<T: Send + Sync> SelfMethod<T> {
    pub fn get(&self) -> &mut T {
        unsafe { self.pointer.as_mut().unwrap() }
    }
}

impl<T: Send + Sync> Copy for SelfMethod<T> {}
impl<T: Send + Sync> Clone for SelfMethod<T> {
    fn clone(&self) -> Self {
        *self
    }
}

unsafe impl<T: Send + Sync> Send for SelfMethod<T> {}
unsafe impl<T: Send + Sync> Sync for SelfMethod<T> {}

pub struct Signal<T: Serialize> {
    internal: Option<crate::signal::Signal<T>>,
}

impl<T: Serialize + 'static> Signal<T> {
    pub fn new() -> Self {
        Self { internal: None }
    }

    pub fn register(&mut self, signal_name: &str) -> crate::Result<()> {
        if self.internal.is_some() {
            return Err(Box::new(BusError::AlreadyRegistered));
        }

        match *SERVICE_BUS.lock().unwrap() {
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

pub struct State<T: Serialize> {
    internal: Option<crate::state::State<T>>,
}

impl<T: Serialize + 'static> State<T> {
    pub fn new() -> Self {
        Self { internal: None }
    }

    pub fn register(&mut self, state_name: &str, initial_value: T) -> crate::Result<()> {
        if self.internal.is_some() {
            return Err(Box::new(BusError::AlreadyRegistered));
        }

        match *SERVICE_BUS.lock().unwrap() {
            Some(ref mut bus) => {
                self.internal = Some(bus.register_state(state_name, initial_value)?)
            }
            _ => panic!("Not registered"),
        }

        Ok(())
    }

    pub fn set(&mut self, value: T) {
        match self.internal {
            None => panic!("Not registered"),
            Some(ref mut internal) => internal.set(value),
        }
    }

    pub fn get(&self) -> &T {
        match self.internal {
            None => panic!("Not registered"),
            Some(ref internal) => internal.get(),
        }
    }
}
