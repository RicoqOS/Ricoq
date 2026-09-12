//! Pure bounded state used by seL4 IPC capability paths.

const FREE: u8 = 0;
const RESERVED: u8 = 1;
const OCCUPIED: u8 = 2;

/// Monotonic nonzero badge allocation with explicit exhaustion.
pub(crate) struct BadgeSequence {
    next: usize,
}

impl BadgeSequence {
    pub(crate) fn new(first: usize) -> Option<Self> {
        (first != 0).then_some(Self { next: first })
    }

    pub(crate) fn allocate(&mut self) -> Option<usize> {
        let badge = self.next;
        self.next = badge.checked_add(1).filter(|next| *next != 0)?;
        Some(badge)
    }
}

/// Failure to reserve or transition a tracked CSpace slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SlotError {
    /// Slot zero is the null capability and is never a receive destination.
    Reserved,
    /// The slot falls outside the configured CNode.
    OutOfRange,
    /// The fixed bookkeeping representation cannot cover the CNode.
    CapacityTooLarge,
    /// The slot already owns a capability or another receive reservation.
    Occupied,
    /// The reservation was already consumed or belongs to different state.
    InvalidReservation,
}

/// A one-shot reservation for a capability receive destination.
pub(crate) struct SlotReservation<'a> {
    slot: usize,
    state: &'a mut u8,
}

/// Fixed-capacity ownership state for one single-level CSpace.
pub(crate) struct SlotTracker {
    capacity: usize,
    states: [u8; Self::MAX_SLOTS],
}

impl SlotTracker {
    /// Maximum CSpace size supported by the bounded task representation.
    pub(crate) const MAX_SLOTS: usize = 256;

    /// Constructs valid slot bookkeeping for a checked capacity.
    pub(crate) fn try_new(capacity: usize) -> Result<Self, SlotError> {
        if capacity > Self::MAX_SLOTS {
            return Err(SlotError::CapacityTooLarge);
        }
        Ok(Self {
            capacity,
            states: [FREE; Self::MAX_SLOTS],
        })
    }

    #[cfg(test)]
    fn new(capacity: usize) -> Self {
        match Self::try_new(capacity) {
            Ok(slots) => slots,
            Err(error) => panic!("invalid test capacity: {error:?}"),
        }
    }

    /// Records a capability installed during CSpace construction.
    pub(crate) fn mark_occupied(
        &mut self,
        slot: usize,
    ) -> Result<(), SlotError> {
        let state = self.state_mut(slot)?;
        if *state != FREE {
            return Err(SlotError::Occupied);
        }
        *state = OCCUPIED;
        Ok(())
    }

    /// Reserves an empty slot before configuring an IPC receive path.
    pub(crate) fn reserve(
        &mut self,
        slot: usize,
    ) -> Result<SlotReservation<'_>, SlotError> {
        let state = self.state_mut(slot)?;
        if *state != FREE {
            return Err(SlotError::Occupied);
        }
        *state = RESERVED;
        Ok(SlotReservation { slot, state })
    }

    /// Commits a receive destination after seL4 installed a capability.
    pub(crate) fn commit(
        reservation: SlotReservation<'_>,
    ) -> Result<(), SlotError> {
        if *reservation.state != RESERVED {
            return Err(SlotError::InvalidReservation);
        }
        *reservation.state = OCCUPIED;
        Ok(())
    }

    /// Releases a reservation after a receive transferred no capability.
    pub(crate) fn cancel(
        reservation: SlotReservation<'_>,
    ) -> Result<(), SlotError> {
        if *reservation.state != RESERVED {
            return Err(SlotError::InvalidReservation);
        }
        *reservation.state = FREE;
        Ok(())
    }

    /// Returns the reserved slot index.
    pub(crate) fn reservation_slot(
        reservation: &SlotReservation<'_>,
    ) -> usize {
        reservation.slot
    }

    fn state_mut(&mut self, slot: usize) -> Result<&mut u8, SlotError> {
        if slot == 0 {
            return Err(SlotError::Reserved);
        }
        if slot >= self.capacity {
            return Err(SlotError::OutOfRange);
        }
        self.states.get_mut(slot).ok_or(SlotError::OutOfRange)
    }
}

#[cfg(test)]
mod tests {
    use super::{BadgeSequence, SlotError, SlotTracker};

    #[test]
    fn badge_sequence_is_nonzero_unique_and_bounded() {
        assert!(BadgeSequence::new(0).is_none());
        let mut badges =
            BadgeSequence::new(7).expect("nonzero start is valid");
        assert_eq!(badges.allocate(), Some(7));
        assert_eq!(badges.allocate(), Some(8));
        let mut exhausted =
            BadgeSequence::new(usize::MAX).expect("nonzero start is valid");
        assert_eq!(exhausted.allocate(), None);
        assert_eq!(exhausted.allocate(), None);
    }

    #[test]
    fn rejects_invalid_and_occupied_receive_slots() {
        let mut slots = SlotTracker::new(8);
        assert!(matches!(slots.reserve(0), Err(SlotError::Reserved)));
        assert!(matches!(slots.reserve(8), Err(SlotError::OutOfRange)));
        assert_eq!(slots.mark_occupied(3), Ok(()));
        assert!(matches!(slots.reserve(3), Err(SlotError::Occupied)));
    }

    #[test]
    fn failed_receive_keeps_destination_free() {
        let mut slots = SlotTracker::new(8);
        let reservation = slots.reserve(3).expect("slot 3 is valid");
        assert_eq!(SlotTracker::cancel(reservation), Ok(()));
        assert_eq!(slots.reserve(3).map(|_| ()), Ok(()));
    }

    #[test]
    fn successful_receive_commits_destination_ownership() {
        let mut slots = SlotTracker::new(8);
        let reservation = slots.reserve(3).expect("slot 3 is valid");
        assert_eq!(SlotTracker::commit(reservation), Ok(()));
        assert!(matches!(slots.reserve(3), Err(SlotError::Occupied)));
    }

    #[test]
    fn committed_reservations_cannot_be_reused() {
        let mut first = SlotTracker::new(8);
        let reservation = first.reserve(3).expect("slot 3 is valid");
        assert_eq!(SlotTracker::commit(reservation), Ok(()));
        assert!(matches!(first.reserve(3), Err(SlotError::Occupied)));
    }

    #[test]
    fn bounded_tracker_rejects_unrepresentable_capacity() {
        assert!(matches!(
            SlotTracker::try_new(SlotTracker::MAX_SLOTS + 1),
            Err(SlotError::CapacityTooLarge),
        ));
    }
}
