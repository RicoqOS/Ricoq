use core::ops::Range;

/// Owned only by the initial thread; allocated slots are never returned.
pub(crate) struct FreeSlots {
    remaining: Range<usize>,
}

impl FreeSlots {
    pub(crate) fn new(remaining: Range<usize>) -> Self {
        Self { remaining }
    }

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
