//! Slot allocation management for single-threaded contexts.

use core::ops::Range;

/// Manages a contiguous range of indices allocated on demand.
///
/// `FreeSlots` provides sequential index allocation over a specified `usize`
/// range. It is intended to be owned by a single thread. Only a fully emptied
/// allocation suffix may be returned for reuse.
pub(crate) struct FreeSlots {
    first: usize,
    remaining: Range<usize>,
}

impl FreeSlots {
    /// Creates a new [`FreeSlots`] allocator with a provided index range.
    pub(crate) fn new(remaining: Range<usize>) -> Self {
        Self {
            first: remaining.start,
            remaining,
        }
    }

    /// Commits a slot only when creation succeeds; errors leave it available.
    /// The callback must leave the slot empty on error.
    pub(crate) fn try_allocate<T, E>(
        &mut self,
        create: impl FnOnce(usize) -> Result<T, E>,
    ) -> Result<Option<T>, E> {
        if self.remaining.is_empty() {
            return Ok(None);
        }
        let Some(next) = self.remaining.start.checked_add(1) else {
            return Ok(None);
        };
        let value = create(self.remaining.start)?;
        self.remaining.start = next;
        Ok(Some(value))
    }

    /// Captures the next slot so a multi-object operation can be rolled back.
    pub(crate) fn checkpoint(&self) -> usize {
        self.remaining.start
    }

    /// Returns slots committed after a valid checkpoint.
    pub(crate) fn allocated_since(
        &self,
        checkpoint: usize,
    ) -> Option<Range<usize>> {
        (self.first <= checkpoint && checkpoint <= self.remaining.start)
            .then_some(checkpoint..self.remaining.start)
    }

    /// Restores a checkpoint after the caller has emptied every later slot.
    pub(crate) fn rewind(&mut self, checkpoint: usize) -> bool {
        if self.allocated_since(checkpoint).is_none() {
            return false;
        }
        self.remaining.start = checkpoint;
        true
    }
}

#[cfg(test)]
mod tests {
    use super::FreeSlots;

    #[test]
    fn slots_are_unique_and_exhaustion_is_permanent() {
        let mut slots = FreeSlots::new(7..9);
        assert_eq!(slots.try_allocate(Ok::<_, ()>), Ok(Some(7)));
        assert_eq!(slots.try_allocate(Ok::<_, ()>), Ok(Some(8)));
        assert_eq!(slots.try_allocate(Ok::<_, ()>), Ok(None));
        assert_eq!(slots.try_allocate(Ok::<_, ()>), Ok(None));
    }

    #[test]
    fn empty_range_has_no_slots() {
        assert_eq!(FreeSlots::new(4..4).try_allocate(Ok::<_, ()>), Ok(None));
    }

    #[test]
    fn upper_bound_does_not_overflow() {
        let mut slots = FreeSlots::new(usize::MAX - 1..usize::MAX);
        assert_eq!(slots.try_allocate(Ok::<_, ()>), Ok(Some(usize::MAX - 1)));
        assert_eq!(slots.try_allocate(Ok::<_, ()>), Ok(None));
    }

    #[test]
    fn failed_creation_preserves_the_slot() {
        let mut slots = FreeSlots::new(7..8);
        for failure in ["no candidates", "all exhausted", "kernel error"] {
            let result: Result<Option<usize>, &str> =
                slots.try_allocate(|slot| {
                    assert_eq!(slot, 7);
                    Err(failure)
                });
            assert_eq!(result, Err(failure));
        }
        assert_eq!(slots.try_allocate(Ok::<_, &str>), Ok(Some(7)));
        assert_eq!(slots.try_allocate(Ok::<_, &str>), Ok(None));
    }

    #[test]
    fn empty_slots_do_not_attempt_creation() {
        let mut slots = FreeSlots::new(4..4);
        let result: Result<Option<usize>, ()> = slots.try_allocate(|_| {
            panic!("creation must not run without an empty slot");
        });
        assert_eq!(result, Ok(None));
    }

    #[test]
    fn checkpoint_rewinds_only_a_valid_committed_suffix() {
        let mut slots = FreeSlots::new(7..10);
        let checkpoint = slots.checkpoint();
        assert_eq!(slots.try_allocate(Ok::<_, ()>), Ok(Some(7)));
        assert_eq!(slots.try_allocate(Ok::<_, ()>), Ok(Some(8)));
        assert_eq!(slots.allocated_since(checkpoint), Some(7..9));
        assert!(slots.rewind(checkpoint));
        assert_eq!(slots.try_allocate(Ok::<_, ()>), Ok(Some(7)));
        assert!(!slots.rewind(6));
        assert!(!slots.rewind(9));
    }
}
