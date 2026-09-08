//! Slot allocation management for single-threaded contexts.

use core::ops::Range;

/// Manages a contiguous range of indices allocated on demand.
///
/// `FreeSlots` provides sequential index allocation over a specified `usize`
/// range. It is intended to be owned by a single thread; slots allocated from
/// this struct are never returned or reused.
pub(crate) struct FreeSlots {
    remaining: Range<usize>,
}

impl FreeSlots {
    /// Creates a new [`FreeSlots`] allocator with a provided index range.
    pub(crate) fn new(remaining: Range<usize>) -> Self {
        Self { remaining }
    }

    /// Allocates and returns the next available slot index, if any remain.
    ///
    /// Returns `Some(index)` containing the next available index in sequence,
    /// or `None` if all slots in the range have been exhausted.
    ///
    /// # Examples
    ///
    /// ```
    /// let mut slots = FreeSlots::new(1..3);
    /// assert_eq!(slots.allocate(), Some(1));
    /// assert_eq!(slots.allocate(), Some(2));
    /// assert_eq!(slots.allocate(), None);
    /// ```
    pub(crate) fn allocate(&mut self) -> Option<usize> {
        self.remaining.next()
    }
}

#[cfg(test)]
mod tests {
    use super::FreeSlots;

    #[test]
    fn slots_are_unique_and_exhaustion_is_permanent() {
        let mut slots = FreeSlots::new(7..9);
        assert_eq!(slots.allocate(), Some(7));
        assert_eq!(slots.allocate(), Some(8));
        assert_eq!(slots.allocate(), None);
        assert_eq!(slots.allocate(), None);
    }

    #[test]
    fn empty_range_has_no_slots() {
        assert_eq!(FreeSlots::new(4..4).allocate(), None);
    }

    #[test]
    fn upper_bound_does_not_overflow() {
        let mut slots = FreeSlots::new(usize::MAX - 1..usize::MAX);
        assert_eq!(slots.allocate(), Some(usize::MAX - 1));
        assert_eq!(slots.allocate(), None);
    }
}
