use std::{collections::HashMap, pin::Pin, sync::Arc};

use bson::Bson;
use futures::{future, lock::Mutex, Future};
use log::{debug, warn};
use serde::{de::DeserializeOwned, Serialize};

use krossbar_common_rpc::request::{Body, RpcRequest};

use self::{signal::Signal, state::State};

pub mod signal;
pub mod state;

type MethodFunctionType =
    Box<dyn FnMut(Bson) -> Pin<Box<dyn Future<Output = crate::Result<Bson>> + Send>> + Send>;

pub struct Endpoints {
    methods: HashMap<String, MethodFunctionType>,
    signals: HashMap<String, signal::Handle>,
    states: HashMap<String, state::Handle>,
}

impl Endpoints {
    pub fn new() -> Self {
        Self {
            methods: HashMap::new(),
            signals: HashMap::new(),
            states: HashMap::new(),
        }
    }

    pub async fn handle_call(&mut self, mut request: RpcRequest) {
        match request.take_body().unwrap() {
            Body::Call(params) => {
                if let Some(method) = self.methods.get_mut(request.endpoint()) {
                    debug!(
                        "Method call. Name: {}. Params: {params:?}",
                        request.endpoint()
                    );

                    let result = method(params).await;
                    request.respond(result).await;
                } else if let Some(state) = self.states.get_mut(request.endpoint()) {
                    debug!("State subscription. Name: {}", request.endpoint());

                    request.respond(Ok(state.value().await)).await;
                } else {
                    warn!("Unknown call endpoint requested: {}", request.endpoint());
                    request.respond::<()>(Err(crate::Error::NoEndpoint)).await;
                }
            }
            Body::Subscription => {
                if let Some(signal) = self.signals.get_mut(request.endpoint()) {
                    debug!("Signal subscription. Name: {}", request.endpoint());

                    signal
                        .add_client(request.message_id(), request.writer().clone())
                        .await;
                } else if let Some(state) = self.states.get_mut(request.endpoint()) {
                    debug!("State subscription. Name: {}", request.endpoint());

                    state
                        .add_client(request.message_id(), request.writer().clone())
                        .await;
                } else {
                    warn!("Unknown endpoint requested: {}", request.endpoint());
                    request.respond::<()>(Err(crate::Error::NoEndpoint)).await;
                }
            }
            _ => {
                warn!("Unknown endpoint requested: {}", request.endpoint());
                request.respond::<()>(Err(crate::Error::NotAllowed)).await;
            }
        }
    }

    pub fn register_method<P, R, Fr, F>(&mut self, name: &str, func: F) -> crate::Result<()>
    where
        P: DeserializeOwned + 'static + Send,
        R: Serialize,
        Fr: Future<Output = R> + Send,
        F: FnMut(P) -> Fr + 'static + Send,
    {
        if self.methods.contains_key(name) {
            return Err(crate::Error::AlreadyRegistered);
        }

        let function_mutex = Arc::new(Mutex::new(func));
        let internal_method: MethodFunctionType = Box::new(move |param: Bson| {
            let param = match bson::from_bson::<P>(param) {
                Ok(value) => value,
                Err(e) => {
                    return Box::pin(future::err(crate::Error::ParamsTypeError(e.to_string())))
                }
            };

            let fn_clone = function_mutex.clone();
            Box::pin(async move {
                let result = fn_clone.lock().await(param).await;

                match bson::to_bson(&result) {
                    Ok(bson) => Ok(bson),
                    Err(e) => Err(crate::Error::ResultTypeError(e.to_string())),
                }
            })
        });

        self.methods
            .insert(name.to_owned(), Box::new(internal_method));

        Ok(())
    }

    pub fn register_signal<T: Serialize>(&mut self, name: &str) -> crate::Result<Signal<T>> {
        if self.signals.contains_key(name) {
            return Err(crate::Error::AlreadyRegistered);
        }

        let result = Signal::new();

        self.signals.insert(name.to_owned(), result.handle());

        Ok(result)
    }

    pub fn register_state<T: Serialize>(
        &mut self,
        name: &str,
        value: T,
    ) -> crate::Result<State<T>> {
        if self.states.contains_key(name) {
            return Err(crate::Error::AlreadyRegistered);
        }

        let result = State::new(value)?;

        self.states.insert(name.to_owned(), result.handle());

        Ok(result)
    }
}
