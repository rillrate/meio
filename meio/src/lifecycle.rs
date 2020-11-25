//! Contains message of the `Actor`'s lifecycle.

use crate::Action;
use anyhow::Error;

pub(crate) trait LifecycleNotifier: Send {
    fn notify(&mut self) -> Result<(), Error>;
}

impl<T> LifecycleNotifier for T
where
    T: FnMut() -> Result<(), Error>,
    T: Send,
{
    fn notify(&mut self) -> Result<(), Error> {
        (self)()
    }
}

/// This message sent by a `Supervisor` to a spawned child actor.
pub struct Awake /* TODO: Add `Supervisor` type parameter to support different spawners */ {
    // TODO: Add `Supervisor`
}

impl Awake {
    pub(crate) fn new() -> Self {
        Self {}
    }
}

impl Action for Awake {
    fn is_high_priority(&self) -> bool {
        true
    }
}

/// The event to ask an `Actor` to interrupt its activity.
pub struct Interrupt {}

impl Interrupt {
    pub(crate) fn new() -> Self {
        Self {}
    }
}

impl Action for Interrupt {
    fn is_high_priority(&self) -> bool {
        true
    }
}

/// Notifies when `Actor`'s activity is completed.
pub struct Done {}

impl Done {
    pub(crate) fn new() -> Self {
        Self {}
    }
}

impl Action for Done {
    fn is_high_priority(&self) -> bool {
        true
    }
}
/*
 * struct Supervisor {
 *   address?
 * }
 *
 * impl Supervisor {
 *   /// The method that allow a child to ask the supervisor to shutdown.
 *   /// It sends `Shutdown` message, the supervisor can ignore it.
 *   fn shutdown();
 * }
*/