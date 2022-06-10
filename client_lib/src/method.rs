use std::marker::PhantomData;

use tokio::sync::mpsc::{self, Receiver, Sender};
use tokio::sync::oneshot::{self, Receiver as OneReceiver, Sender as OneSender};

pub trait MethodTrait {}

pub struct Method<ParamsType, ResponseType> {
    params: ParamsType,
    callback_send: Sender<ParamsType>,
    callback_rcv: Receiver<ParamsType>,
    stup: PhantomData<ResponseType>,
}

impl<ParamsType, ResponseType> Method<ParamsType, ResponseType> {
    pub fn get() -> Option<(ParamsType, OneSender<ResponseType>)> {
        None
    }
}
