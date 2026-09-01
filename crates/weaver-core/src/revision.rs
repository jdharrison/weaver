//! Monotonically increasing world revisions.

/// A monotonically increasing world revision number.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Revision(u64);

impl Revision {
    /// The first revision.
    pub const ZERO: Self = Self(0);

    /// Create a revision from a raw value.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Raw revision number.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Advance to the next revision.
    #[must_use]
    pub const fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }
}

impl From<u64> for Revision {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

/// A value tagged with the world revision at which it was committed.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Versioned<T> {
    /// The committed value.
    pub value: T,
    /// The world revision at which the value was committed.
    pub revision: Revision,
}

impl<T> Versioned<T> {
    /// Tag a value with a revision.
    #[must_use]
    pub const fn new(value: T, revision: Revision) -> Self {
        Self { value, revision }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revision_monotonicity() {
        let mut rev = Revision::ZERO;
        for _ in 0..10 {
            rev = rev.next();
        }
        assert_eq!(rev.get(), 10);
    }
}
