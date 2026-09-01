//! Frame identifiers and parent/child relationships.

use std::collections::HashMap;
use thiserror::Error;

/// A coordinate frame identifier.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FrameId(u64);

impl FrameId {
    /// The implicit root / world frame.
    pub const ROOT: Self = Self(0);

    /// Create a frame identifier from a raw value.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Raw frame value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl Default for FrameId {
    fn default() -> Self {
        Self::ROOT
    }
}

impl From<u64> for FrameId {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

/// Errors that can occur when manipulating the frame tree.
#[derive(Debug, Error, PartialEq)]
pub enum FrameError {
    /// A frame was added with itself as an ancestor.
    #[error("frame cycle detected")]
    Cycle,
    /// The referenced parent frame does not exist.
    #[error("parent frame not found")]
    ParentNotFound,
    /// The frame already exists.
    #[error("frame already exists")]
    AlreadyExists,
}

/// A tree of coordinate frames.
#[derive(Debug, Default)]
pub struct FrameRegistry {
    parents: HashMap<FrameId, FrameId>,
    next_id: u64,
}

impl FrameRegistry {
    /// Create an empty registry containing only the root frame.
    #[must_use]
    pub fn new() -> Self {
        Self {
            parents: HashMap::new(),
            next_id: 1,
        }
    }

    /// Create a new frame parented under `parent`.
    ///
    /// # Errors
    ///
    /// Returns [`FrameError::ParentNotFound`] if the parent is not in the tree,
    /// or [`FrameError::Cycle`] if the new frame would create a cycle.
    pub fn create(&mut self, parent: FrameId) -> Result<FrameId, FrameError> {
        if parent != FrameId::ROOT && !self.parents.contains_key(&parent) {
            return Err(FrameError::ParentNotFound);
        }
        let id = FrameId::new(self.next_id);
        self.next_id += 1;
        self.parents.insert(id, parent);
        Ok(id)
    }

    /// Insert a frame with a specific id and parent.
    ///
    /// # Errors
    ///
    /// Returns [`FrameError::AlreadyExists`] if the id is already present,
    /// [`FrameError::ParentNotFound`] if the parent is missing, or
    /// [`FrameError::Cycle`] if the insertion would create a cycle.
    pub fn insert(&mut self, id: FrameId, parent: FrameId) -> Result<(), FrameError> {
        if id == FrameId::ROOT {
            return Ok(());
        }
        if parent != FrameId::ROOT && !self.parents.contains_key(&parent) {
            return Err(FrameError::ParentNotFound);
        }
        if self.is_ancestor(parent, id) {
            return Err(FrameError::Cycle);
        }
        if self.parents.contains_key(&id) {
            return Err(FrameError::AlreadyExists);
        }
        self.parents.insert(id, parent);
        self.next_id = self.next_id.max(id.get() + 1);
        Ok(())
    }

    /// Return the parent of a frame, or `None` for the root.
    #[must_use]
    pub fn parent(&self, frame: FrameId) -> Option<FrameId> {
        self.parents.get(&frame).copied()
    }

    /// Return `true` if `ancestor` is on the path from `frame` to the root.
    #[must_use]
    pub fn is_ancestor(&self, frame: FrameId, ancestor: FrameId) -> bool {
        if frame == ancestor {
            return true;
        }
        let mut current = frame;
        while let Some(parent) = self.parents.get(&current).copied() {
            if parent == ancestor {
                return true;
            }
            current = parent;
        }
        false
    }

    /// Returns `true` if the frame exists in the registry.
    #[must_use]
    pub fn contains(&self, frame: FrameId) -> bool {
        frame == FrameId::ROOT || self.parents.contains_key(&frame)
    }

    /// Remove a frame and all of its descendants.
    pub fn remove(&mut self, frame: FrameId) {
        let mut to_remove = vec![frame];
        while let Some(current) = to_remove.pop() {
            self.parents.remove(&current);
            for (child, parent) in &self.parents {
                if *parent == current {
                    to_remove.push(*child);
                }
            }
        }
    }

    /// Iterate over all known frame ids.
    pub fn frames(&self) -> impl Iterator<Item = FrameId> + use<'_> {
        std::iter::once(FrameId::ROOT).chain(self.parents.keys().copied())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_always_exists() {
        let registry = FrameRegistry::new();
        assert!(registry.contains(FrameId::ROOT));
    }

    #[test]
    fn create_child() {
        let mut registry = FrameRegistry::new();
        let child = registry.create(FrameId::ROOT).unwrap();
        assert!(registry.is_ancestor(child, FrameId::ROOT));
    }

    #[test]
    fn reject_cycle() {
        let mut registry = FrameRegistry::new();
        let a = registry.create(FrameId::ROOT).unwrap();
        let b = registry.create(a).unwrap();
        assert_eq!(registry.insert(a, b), Err(FrameError::Cycle));
    }

    #[test]
    fn reject_self_parent() {
        let mut registry = FrameRegistry::new();
        let a = registry.create(FrameId::ROOT).unwrap();
        assert_eq!(registry.insert(a, a), Err(FrameError::Cycle));
    }
}
