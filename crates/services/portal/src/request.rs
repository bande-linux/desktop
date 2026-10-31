use std::sync::{Arc, Mutex, PoisonError};

use zbus::object_server::ObjectServer;
use zbus::zvariant::ObjectPath;

pub const SUCCESS: u32 = 0;
pub const CANCELLED: u32 = 1;
pub const OTHER: u32 = 2;

type Callback = Box<dyn FnOnce() + Send>;

#[derive(Default)]
struct State {
    cancelled: bool,
    callbacks: Vec<Callback>,
}

#[derive(Clone, Default)]
pub struct Cancellation(Arc<Mutex<State>>);

impl Cancellation {
    pub fn is_cancelled(&self) -> bool {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .cancelled
    }

    pub fn on_cancel(&self, callback: impl FnOnce() + Send + 'static) {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        if state.cancelled {
            drop(state);
            callback();
        } else {
            state.callbacks.push(Box::new(callback));
        }
    }

    pub fn cancel(&self) {
        let callbacks = {
            let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
            state.cancelled = true;
            std::mem::take(&mut state.callbacks)
        };
        for callback in callbacks {
            callback();
        }
    }
}

struct Request {
    cancellation: Cancellation,
}

#[zbus::interface(name = "org.freedesktop.impl.portal.Request")]
impl Request {
    async fn close(&self) {
        let cancellation = self.cancellation.clone();
        blocking::unblock(move || cancellation.cancel()).await;
    }
}

pub async fn run<T: Send + 'static>(
    server: &ObjectServer,
    handle: &ObjectPath<'_>,
    work: impl FnOnce(&Cancellation) -> T + Send + 'static,
) -> zbus::fdo::Result<Option<T>> {
    let cancellation = Cancellation::default();
    server
        .at(
            handle,
            Request {
                cancellation: cancellation.clone(),
            },
        )
        .await?;
    let worker = cancellation.clone();
    let result = blocking::unblock(move || work(&worker)).await;
    server.remove::<Request, _>(handle).await?;
    Ok((!cancellation.is_cancelled()).then_some(result))
}
