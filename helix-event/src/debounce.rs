//! Utilities for declaring an async (usually debounced) hook

use std::time::Duration;

use futures_executor::block_on;
use tokio::sync::mpsc::{self, error::TrySendError, Sender};
use tokio::time::Instant;

/// Async hooks are the core of the event system, the m
pub trait AsyncHook: Sync + Send + 'static + Sized {
    type Event: Sync + Send + 'static;
    /// Called immidietly whenever an event is received, this function can
    /// consume the event immidietly or debounce it. In case of debouncing
    /// it can either define a new debounce timeout or continue the current
    fn handle_event(&mut self, event: Self::Event, timeout: Option<Instant>) -> Option<Instant>;

    /// Called whenever the debounce timeline is searched
    fn finish_debounce(&mut self);

    fn spawn(self) -> mpsc::Sender<Self::Event> {
        // the capaicity doesn't matter too much here, unless the cpu is totally overwhelmed
        // the cap will never be reached sine we awalys immidietly drain the channel
        // so is should only be reached in case of total CPU overload
        // However, a bounded channel is much more efficient so its nice to use here
        let (tx, rx) = mpsc::channel(128);
        tokio::spawn(run(self, rx));
        tx
    }
}

async fn run<Hook: AsyncHook>(mut hook: Hook, mut rx: mpsc::Receiver<Hook::Event>) {
    let mut deadline = None;
    loop {
        let event = match deadline {
            Some(deadline_) => {
                let res = tokio::time::timeout_at(deadline_, rx.recv()).await;
                match res {
                    Ok(event) => event,
                    Err(_) => {
                        hook.finish_debounce();
                        deadline = None;
                        continue;
                    }
                }
            }
            None => rx.recv().await,
        };
        let Some(event) = event else {
            break;
        };
        deadline = hook.handle_event(event, deadline);
    }
}

pub fn send_blocking<T>(tx: &Sender<T>, data: T) {
    // block_on has some ovherhead and in practice the channel should basically
    // never be full anyway so first try sending without blocking
    if let Err(TrySendError::Full(data)) = tx.try_send(data) {
        // set a timeout so that we just drop a message instead of freezing the editor in the worst case
        block_on(tx.send_timeout(data, Duration::from_millis(10))).unwrap();
    }
}
