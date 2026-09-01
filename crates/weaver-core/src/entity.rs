//! Entity identifiers and a minimal registry.

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

/// A Weaver entity identifier.
///
/// This is intentionally separate from any backing runtime (Signalweave,
/// renderer, physics, etc.) so that Weaver can map between them without
/// leaking internal IDs across crate boundaries.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EntityId(u64);

impl EntityId {
    /// Allocate a fresh entity identifier.
    #[must_use]
    pub fn new() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        Self(COUNTER.fetch_add(1, Ordering::Relaxed))
    }

    /// Raw numeric value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Create an identifier from a raw value.
    ///
    /// # Safety
    ///
    /// The caller must ensure the value corresponds to an existing entity
    /// returned by [`EntityId::new`]. Arbitrary values may collide with future
    /// allocated identifiers.
    #[must_use]
    pub const fn from_raw(id: u64) -> Self {
        Self(id)
    }
}

impl Default for EntityId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for EntityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "E{}", self.0)
    }
}

/// Minimal storage for entities before a full ECS is selected.
///
/// The registry is intentionally generic: it maps an [`EntityId`] to an
/// application-defined component bag. Systems that need typed access can
/// downcast or replace this with a real ECS later.
pub struct EntityRegistry<T> {
    entities: Vec<(EntityId, T)>,
}

impl<T> Default for EntityRegistry<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> EntityRegistry<T> {
    /// Create an empty registry.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            entities: Vec::new(),
        }
    }

    /// Spawn a new entity with the given components.
    pub fn spawn(&mut self, components: T) -> EntityId {
        let id = EntityId::new();
        self.entities.push((id, components));
        id
    }

    /// Remove an entity, returning its components if it existed.
    pub fn remove(&mut self, id: EntityId) -> Option<T> {
        let index = self.entities.iter().position(|(e, _)| *e == id)?;
        Some(self.entities.swap_remove(index).1)
    }

    /// Borrow an entity's components.
    #[must_use]
    pub fn get(&self, id: EntityId) -> Option<&T> {
        self.entities.iter().find(|(e, _)| *e == id).map(|(_, c)| c)
    }

    /// Mutably borrow an entity's components.
    #[must_use]
    pub fn get_mut(&mut self, id: EntityId) -> Option<&mut T> {
        self.entities
            .iter_mut()
            .find(|(e, _)| *e == id)
            .map(|(_, c)| c)
    }

    /// Iterate over all entities.
    pub fn iter(&self) -> impl Iterator<Item = (EntityId, &T)> {
        self.entities.iter().map(|(id, c)| (*id, c))
    }

    /// Iterate mutably over all entities.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (EntityId, &mut T)> {
        self.entities.iter_mut().map(|(id, c)| (*id, c))
    }

    /// Number of live entities.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entities.len()
    }

    /// Returns `true` if the registry contains no entities.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entities.is_empty()
    }

    /// Clear all entities.
    pub fn clear(&mut self) {
        self.entities.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_and_remove() {
        let mut registry = EntityRegistry::<i32>::new();
        let id = registry.spawn(42);
        assert_eq!(registry.get(id), Some(&42));
        assert_eq!(registry.remove(id), Some(42));
        assert!(registry.get(id).is_none());
    }

    #[test]
    fn entity_ids_are_unique() {
        let id_a = EntityId::new();
        let id_b = EntityId::new();
        assert_ne!(id_a, id_b);
    }
}
