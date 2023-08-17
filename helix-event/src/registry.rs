//! A global registry where events are registered and can be
//! subsribed to by registering hooks. The registry identifies event
//! types using their type name so multiple event with the same type name
//! may not be registered (will cause a panic to ensure soudness)

use std::any::TypeId;

use anyhow::{bail, Result};
use hashbrown::hash_map::Entry;
use hashbrown::HashMap;
use parking_lot::RwLock;

use crate::hook::ErasedHook;
use crate::{runtime_local, Hook};

pub struct Registry {
    events: HashMap<&'static str, TypeId, ahash::RandomState>,
    handlers: HashMap<&'static str, Vec<ErasedHook>, ahash::RandomState>,
}

impl Registry {
    pub fn register_event<E: Event<'static>>(&mut self) {
        let ty = TypeId::of::<E>();
        assert_eq!(ty, TypeId::of::<E::Static>());
        match self.events.entry(E::ID) {
            Entry::Occupied(entry) => {
                if entry.get() == &ty {
                    log::warn!("Event {} was registered multiple times", E::ID);
                } else {
                    panic!("Multiple events with ID {} were registered", E::ID);
                }
            }
            Entry::Vacant(ent) => {
                ent.insert(ty);
                self.handlers.insert(E::ID, Vec::new());
            }
        }
    }

    pub fn register_hook<H: Hook>(&mut self, hook: H) {
        // ensure event type ids match so we can rely on them always matching
        let id = H::Event::ID;
        let Some(&event_id) = self.events.get(id) else {
            panic!("Tried to register handler for unknown event {id}");
        };
        assert!(
            TypeId::of::<H::Event<'static>>() == event_id,
            "Tried to register invalid hook for event {id}"
        );
        let hook = ErasedHook::new(hook);
        self.handlers.get_mut(id).unwrap().push(hook);
    }

    pub fn register_dynamic_hook<H: Fn() + Sync + Send + 'static>(
        &mut self,
        hook: H,
        id: &str,
    ) -> Result<()> {
        // ensure event type ids match so we can rely on them always matching
        if self.events.get(id).is_none() {
            bail!("Tried to register handler for unknown event {id}");
        };
        let hook = ErasedHook::new_dynamic(hook);
        self.handlers.get_mut(id).unwrap().push(hook);
        Ok(())
    }

    pub fn dispatch<'a, E: Event<'a>>(&self, mut event: E) {
        let Some(hooks) = self.handlers.get(E::ID) else {
            panic!("Unknown event {}", E::ID);
        };
        let event_id = self.events[E::ID];

        assert_eq!(
            TypeId::of::<E::Static>(),
            event_id,
            "Tried to dispatch invalid event {}",
            E::ID
        );

        for hook in hooks {
            // safety: event type is the same
            if let Err(err) = unsafe { hook.call(&mut event) } {
                log::error!("{} hook failed: {err:#?}", E::ID);
                crate::status::report_blocking(err);
            }
        }
    }
}

runtime_local! {
    static REGISTRY: RwLock<Registry> = RwLock::new(Registry {
        // hardcoded random number is good enough here we don't care about DOS resistance
        // and avoids the additional complexity of `Option<Registry>`
        events: HashMap::with_hasher(ahash::RandomState::with_seeds(423, 9978, 38322, 3280080)),
        handlers: HashMap::with_hasher(ahash::RandomState::with_seeds(423, 99078, 382322, 3282938)),
    });
}

pub(crate) fn with<T>(f: impl FnOnce(&Registry) -> T) -> T {
    f(&REGISTRY.read())
}

pub(crate) fn with_mut<T>(f: impl FnOnce(&mut Registry) -> T) -> T {
    f(&mut REGISTRY.write())
}

/// # Safety
/// This trait is safe to use but not safe to declare, use the events macro instead
pub unsafe trait Event<'a>: Sized + 'a {
    /// Globally unique (case sensitive)  sting that identifies this type.
    /// A good candidate is the events type name
    const ID: &'static str;
    type Static: Event<'static>;
}
