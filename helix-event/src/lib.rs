//! `helix-event` contains systems that allow (often async) communication between
//! different editor components without strongly coupling them. Specifically
//! it allows defining synchronous hooks that run when certain editor events
//! occurs.
//!
//! The core of the event system is the [`Hook`] trait. A hook is essentially
//! just a closure `Fn(event: &mut impl Event) -> Result<()>`. This can currently
//! not be represented in the rust type system with closures (it requires second
//! order generics). Instead we use generic associated types to represent that
//! invariant so a custom type is always required.
//!
//! The [`Event`] trait is unsafe because upon dispatch event lifetimes are
//! essentially erased. To ensure safety all lifetime parameters of the event
//! must oulife the lifetime Parameter of the event trait. To avoid worrying about
//! that (and spreading unsafe everywhere) the [`events`] macro is provided which
//! automatically declares event types.
//!
//! Hooks run synchronously which can be advantageous since they can modify the
//! current editor state right away (for example to immidietly hide the completion
//! popup). However, they can not contain their own state without locking since
//! they only receive immutable references. For handler that want to track state, do
//! expensive background computations or debouncing an [`AsyncHook`] is preferable.
//! Async hooks are based around a channels that receive events specific to
//! that `AsyncHook` (usually an enum). These events can be send by synchronous
//! [`Hook`]s. Due to some limtations around tokio channels the [`send_blocking`]
//! function exported in this crate should be used instead of the builtin
//! `blocking_send`.
//!
//! In addition to the core event system, this crate contains some message queues
//! that allow transfer of data back to the main event loop from async hooks and
//! hooks that may not have access to all application data (for example in helix-view).
//! This include the ability to control rendering ([`lock_frame`], [`request_redraw`]) and
//! display status messages ([`status`]).
//!
//! Hooks declared in helix-term can furthermore dispatch synchronous jobs to be run on the
//! main loop (including access to the compositor). Ideally that queue will be moved
//! to helix-view in the future if we manage to detch the comositor from its rendering backgend.

use anyhow::Result;
pub use cancel::{canceable_future, cancelation, CancelRx, CancelTx};
pub use debounce::{send_blocking, AsyncHook};
pub use redraw::{lock_frame, redraw_requested, request_redraw, start_frame, RenderLockGuard};
pub use registry::Event;

mod cancel;
mod debounce;
mod hook;
mod redraw;
mod registry;
#[doc(hidden)]
pub mod runtime;
pub mod status;

#[cfg(test)]
mod test;

/// A hook is a colsure that will be automatically callen whenever
/// an `Event` of the associated function is [dispatched](crate::dispatch)
/// is called. The closure must be generic over the lifetime of the event.
pub trait Hook: Sized + Sync + Send + 'static {
    type Event<'a>: Event<'a>;
    fn run(&self, _event: &mut Self::Event<'_>) -> Result<()>;
}

pub fn register_event<E: Event<'static>>() {
    registry::with_mut(|registry| registry.register_event::<E>())
}

pub fn register_hook(hook: impl Hook) {
    registry::with_mut(|registry| registry.register_hook(hook))
}

pub fn register_dynamic_hook<H: Fn() + Sync + Send + 'static>(hook: H, id: &str) -> Result<()> {
    registry::with_mut(|reg| reg.register_dynamic_hook(hook, id))
}

pub fn dispatch<'a>(e: impl Event<'a>) {
    registry::with(|registry| registry.dispatch(e));
}

/// Macro to delclare events
///
/// # Examples
///
/// ``` no-compile
/// events! {
///     FileWrite(&Path)
///     ViewScrolled{ view: View, new_pos: ViewOffset }
///     DocumentChanged<'a> { old_doc: &'a Rope, doc: &'a mut Document, changes: &'a ChangSet  }
/// }
///
/// fn init() {
///    register_event::<FileWrite>();
///    register_event::<ViewScrolled>();
///    register_event::<InsertChar>();
///    register_event::<DocumentChanged>();
/// }
///
/// fn save(path: &Path, content: &str){
///     std::fs::write(path, content);
///     dispach(FilWrite(path));
/// }
/// ```
#[macro_export]
macro_rules! events {
    ($name: ident($($data: ty),*) $($rem:tt)*) => {
        pub struct $name($(pub $data),*);
        unsafe impl<'a>  $crate::Event<'a> for $name {
            const ID: &'static str = stringify!($name);
            type Static = Self;
        }
        $crate::events!{ $($rem)* }
    };
    ($name: ident<$lt: lifetime>($(pub $data: ty),*) $($rem:tt)*) => {
        pub struct $name<$lt>($($data),*);
        unsafe impl<$lt> $crate::Event<$lt> for $name<$lt> {
            const ID: &'static str = stringify!($name);
            type Static = $name<'static>;
        }
        $crate::events!{ $($rem)* }
    };
    ($name: ident<$lt: lifetime> { $($data:ident : $data_ty:ty),* } $($rem:tt)*) => {
        pub struct $name<$lt> { $(pub $data: $data_ty),* }
        unsafe impl<$lt> $crate::Event<$lt> for $name<$lt> {
            const ID: &'static str = stringify!($name);
            type Static = $name<'static>;
        }
        $crate::events!{ $($rem)* }
    };
   ($name: ident<$lt1: lifetime, $lt2: lifetime> { $($data:ident : $data_ty:ty),* } $($rem:tt)*) => {
        pub struct $name<$lt1, $lt2> { $(pub $data: $data_ty),* }
        unsafe impl<$lt1, $lt2: $lt1> $crate::Event<$lt1> for $name<$lt1, $lt2> {
            const ID: &'static str = stringify!($name);
            type Static = $name<'static, 'static>;
        }
        $crate::events!{ $($rem)* }
    };
    ($name: ident { $($data:ident : $data_ty:ty),* } $($rem:tt)*) => {
        pub struct $name { $(pub $data: $data_ty),* }
        unsafe impl<'a> $crate::Event<'a> for $name {
            const ID: &'static str = stringify!($name);
            type Static = Self;
        }
        $crate::events!{ $($rem)* }
    };
    () => {};
}
