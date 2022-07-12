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

    async fn register_method<P, R, Ret>(
        method_name: &str,
        context: SelfMethod<Self, P, R, Ret>,
    ) -> crate::Result<()>
    where
        P: DeserializeOwned + Send + 'static,
        R: Serialize + Send + 'static,
        Ret: Future<Output = R> + Send + 'static,
    {
        match *SERVICE_BUS.lock().unwrap() {
            Some(ref mut bus) => {
                bus.register_method(method_name, move |p| async move { context.exec(p).await })?;
            }
            _ => return Err(Box::new(BusError::NotRegistered)),
        }

        Ok(())
    }
}

pub struct SelfMethod<
    T: Send + Sync + 'static,
    P: 'static,
    R: 'static,
    Ret: Future<Output = R> + 'static,
> {
    pointer: *mut T,
    method: &'static (dyn Fn(&mut T, P) -> Ret + Send + Sync),
}

impl<T: Send + Sync, P, R, Ret: Future<Output = R>> SelfMethod<T, P, R, Ret> {
    pub async fn exec(&self, parameter: P) -> R {
        unsafe {
            let reference = self.pointer.as_mut().unwrap();
            (self.method)(reference, parameter).await
        }
    }
}

impl<T: Send + Sync, P, R, Ret: Future<Output = R>> Copy for SelfMethod<T, P, R, Ret> {}
impl<T: Send + Sync, P, R, Ret: Future<Output = R>> Clone for SelfMethod<T, P, R, Ret> {
    fn clone(&self) -> Self {
        *self
    }
}

unsafe impl<T: Send + Sync, P, R, Ret: Future<Output = R>> Send for SelfMethod<T, P, R, Ret> {}
unsafe impl<T: Send + Sync, P, R, Ret: Future<Output = R>> Sync for SelfMethod<T, P, R, Ret> {}

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
            _ => return Err(Box::new(BusError::NotRegistered)),
        }

        Ok(())
    }

    pub fn emit(&self, value: T) -> crate::Result<()> {
        match self.internal {
            None => Err(Box::new(BusError::NotRegistered)),
            Some(ref internal) => Ok(internal.emit(value)),
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
            _ => return Err(Box::new(BusError::NotRegistered)),
        }

        Ok(())
    }

    pub fn set(&mut self, value: T) -> crate::Result<()> {
        match self.internal {
            None => Err(Box::new(BusError::NotRegistered)),
            Some(ref mut internal) => Ok(internal.set(value)),
        }
    }

    pub fn get(&self) -> crate::Result<&T> {
        match self.internal {
            None => Err(Box::new(BusError::NotRegistered)),
            Some(ref internal) => Ok(internal.get()),
        }
    }
}
