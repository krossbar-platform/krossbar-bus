use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use caro_bus_lib::{self, Result as BusResult};
use lazy_static::lazy_static;
use serde::de::DeserializeOwned;

lazy_static! {
    pub(crate) static ref PEERS_CONNECTIONS: Mutex<HashMap<String, Arc<caro_bus_lib::peer::Peer>>> =
        Mutex::new(HashMap::new());
}

#[async_trait]
pub trait Peer {
    async fn register(&mut self) -> caro_bus_lib::Result<()>;

    async fn register_peer(peer_name: &str) -> BusResult<Arc<caro_bus_lib::peer::Peer>> {
        if PEERS_CONNECTIONS.lock().unwrap().contains_key(peer_name) {
            panic!("Peer already registered")
        }

        match *crate::service::SERVICE_BUS.lock().await {
            Some(ref mut bus) => {
                let peer_internal = Arc::new(bus.connect_await(peer_name).await?);
                PEERS_CONNECTIONS
                    .lock()
                    .unwrap()
                    .insert(peer_name.into(), peer_internal.clone());

                Ok(peer_internal)
            }
            _ => panic!("Not registered"),
        }
    }
}

#[async_trait]
pub trait PeerSignalsAndStates {
    async fn register_callbacks(&mut self) -> BusResult<()>;

    async fn subscribe_on_signal<T, Ret>(
        peer_name: &str,
        signal_name: &str,
        callback: impl Fn(T) -> Ret + Send + Sync + 'static,
    ) -> BusResult<()>
    where
        T: DeserializeOwned + Send + 'static,
        Ret: Future<Output = ()> + Send,
    {
        let maybe_peer = PEERS_CONNECTIONS
            .lock()
            .unwrap()
            .get(peer_name.into())
            .cloned();

        match maybe_peer {
            Some(peer) => {
                peer.subscribe::<T, Ret>(signal_name, callback).await?;
            }
            _ => panic!("Not registered"),
        }

        Ok(())
    }

    async fn watch_state<T, Ret>(
        peer_name: &str,
        state_name: &str,
        callback: impl Fn(T) -> Ret + Send + Sync + 'static,
    ) -> BusResult<T>
    where
        T: DeserializeOwned + Send + 'static,
        Ret: Future<Output = ()> + Send,
    {
        let maybe_peer = PEERS_CONNECTIONS
            .lock()
            .unwrap()
            .get(peer_name.into())
            .cloned();

        match maybe_peer {
            Some(peer) => {
                return Ok(peer.watch::<T, Ret>(state_name, callback).await?);
            }
            _ => panic!("Not registered"),
        }
    }
}
