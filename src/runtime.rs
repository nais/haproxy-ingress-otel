use std::fmt::Debug;
use std::future::Future;
use std::time::Duration;

use opentelemetry_sdk::runtime::{Runtime, RuntimeChannel, Tokio};

#[derive(Debug, Clone)]
pub(crate) struct HaproxyTokio(Tokio);

impl HaproxyTokio {
    pub(crate) fn new() -> Self {
        Self(Tokio)
    }
}

impl Runtime for HaproxyTokio {
    fn spawn<F: Future<Output = ()> + Send + 'static>(&self, future: F) {
        let _guard = haproxy_api::runtime().enter();
        self.0.spawn(future);
    }

    fn delay(&self, duration: Duration) -> impl Future<Output = ()> + Send + 'static {
        let _guard = haproxy_api::runtime().enter();
        self.0.delay(duration)
    }
}

impl RuntimeChannel for HaproxyTokio {
    type Receiver<T: Debug + Send> = <Tokio as RuntimeChannel>::Receiver<T>;
    type Sender<T: Debug + Send> = <Tokio as RuntimeChannel>::Sender<T>;

    fn batch_message_channel<T: Debug + Send>(
        &self,
        capacity: usize,
    ) -> (Self::Sender<T>, Self::Receiver<T>) {
        self.0.batch_message_channel(capacity)
    }
}
