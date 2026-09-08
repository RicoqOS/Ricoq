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

    /// Commits a slot only when creation succeeds; errors leave it available.
    /// The callback must leave the slot empty on error.
    pub(crate) fn try_allocate<T, E>(
        &mut self,
        create: impl FnOnce(usize) -> Result<T, E>,
    ) -> Result<Option<T>, E> {
        if self.remaining.is_empty() {
            return Ok(None);
        }
        let value = create(self.remaining.start)?;
        // A nonempty range guarantees start < end <= usize::MAX.
        self.remaining.start += 1;
        Ok(Some(value))
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
}
