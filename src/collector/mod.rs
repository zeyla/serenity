//! Collectors will receive events from the contextual shard, check if the
//! filter lets them pass, and collects if the receive, collect, or time limits
//! are not reached yet.

use std::sync::Arc;

mod error;
pub use error::Error as CollectorError;

#[cfg(feature = "unstable_discord_api")]
pub mod component_interaction_collector;
pub mod event_collector;
pub mod message_collector;
pub mod reaction_collector;

#[cfg(feature = "unstable_discord_api")]
pub use component_interaction_collector::*;
pub use event_collector::*;
pub use message_collector::*;
pub use reaction_collector::*;

/// Wraps a &T and clones the value into an Arc<T> lazily. Used with collectors to allow inspecting
/// the value in filters while only cloning values that actually match.
#[derive(Debug)]
pub(crate) struct LazyArc<'a, T> {
    value: &'a T,
    arc: Option<Arc<T>>,
}

impl<'a, T: Clone> LazyArc<'a, T> {
    pub fn new(value: &'a T) -> Self {
        LazyArc {
            value,
            arc: None,
        }
    }

    pub fn as_arc(&mut self) -> Arc<T> {
        let value = self.value;
        self.arc.get_or_insert_with(|| Arc::new(value.clone())).clone()
    }
}

impl<'a, T> std::ops::Deref for LazyArc<'a, T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.value
    }
}
