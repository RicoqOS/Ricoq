//! Bounded wrappers for the pinned seL4 IPC mechanisms.

use core::cell::Cell;
use core::ops::Range;

use crate::boot::Bootstrap;
use crate::cspace;
use crate::errors::BootstrapError;
use crate::ipc_state::{
    BadgeSequence, SlotError, SlotReservation, SlotTracker,
};

/// Maximum message words provided by the pinned seL4 IPC buffer.
pub const MESSAGE_CAPACITY: usize = sel4::NUM_MESSAGE_REGISTERS;
/// Maximum capability words provided by the pinned seL4 IPC buffer.
pub const EXTRA_CAPACITY: usize = 4;
const MAX_DEFERRED_REPLIES: usize = 64;

/// A validated nonzero capability-path badge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Badge(sel4::Word);

/// Failure to construct a capability-path badge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BadgeError {
    /// Badge zero is reserved for unbadged traffic.
    ReservedZero,
}

/// Failure to configure or advance a unique badge sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BadgeAllocationError {
    /// The initial badge is zero or cannot be represented by the target word.
    InvalidStart,
    /// No additional nonzero target-word badge can be allocated.
    Exhausted,
}

/// A monotonic allocator for unambiguous capability-path badges.
pub struct BadgeAllocator(BadgeSequence);

impl BadgeAllocator {
    /// Starts a badge sequence at a validated nonzero value.
    pub fn new(first: sel4::Word) -> Result<Self, BadgeAllocationError> {
        let first = usize::try_from(first)
            .map_err(|_| BadgeAllocationError::InvalidStart)?;
        BadgeSequence::new(first)
            .map(Self)
            .ok_or(BadgeAllocationError::InvalidStart)
    }

    /// Allocates a badge that has not previously been returned by this
    /// allocator.
    pub fn allocate(&mut self) -> Result<Badge, BadgeAllocationError> {
        let raw = self.0.allocate().ok_or(BadgeAllocationError::Exhausted)?;
        let raw = sel4::Word::try_from(raw)
            .map_err(|_| BadgeAllocationError::Exhausted)?;
        Badge::new(raw).map_err(|_| BadgeAllocationError::Exhausted)
    }
}

impl Badge {
    /// Validates a raw badge without assigning it persistent identity
    /// semantics.
    pub fn new(raw: sel4::Word) -> Result<Self, BadgeError> {
        if raw == 0 {
            return Err(BadgeError::ReservedZero);
        }
        Ok(Self(raw))
    }

    /// Returns the raw badge value used by seL4.
    pub fn raw(self) -> sel4::Word {
        self.0
    }
}

/// A fixed-capacity seL4 message and its outgoing capability slots.
#[derive(Clone)]
pub struct Message {
    label: sel4::Word,
    words: [sel4::Word; MESSAGE_CAPACITY],
    length: usize,
    capabilities: [sel4::Word; EXTRA_CAPACITY],
    extra_caps: usize,
    caps_unwrapped: usize,
}

/// Recoverable generic IPC validation or ownership failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpcError {
    /// The payload exceeds the fixed IPC-buffer message-register capacity.
    MessageTooLong { length: usize, capacity: usize },
    /// The capability list exceeds the pinned IPC-buffer capacity.
    TooManyCapabilities { count: usize, capacity: usize },
    /// A message word index is outside the reported payload.
    InvalidMessageIndex { index: usize, length: usize },
    /// A received message reported more capabilities than the configured path
    /// accepts.
    ReceiveCapacityExceeded { count: usize, capacity: usize },
    /// A receive destination is reserved, occupied, or outside its CSpace.
    InvalidReceiveSlot,
    /// A receive-path state transition violated its one-shot invariant.
    InvalidReceivePath,
    /// Another received request still owns the classic kernel's implicit reply
    /// authority.
    ReplyPending,
    /// No bounded slot is available to preserve another deferred reply.
    NoReplySlots,
    /// The received message did not carry reply authority.
    NoReplyAuthority,
    /// A classic reply capability could not be deleted.
    ReplyCapabilityOperationFailed,
}

impl Message {
    /// Copies a bounded word payload into an allocation-free message.
    pub fn new(
        label: sel4::Word,
        words: &[sel4::Word],
    ) -> Result<Self, IpcError> {
        Self::with_capabilities(label, words, &[] as &[sel4::cap::Unspecified])
    }

    /// Adds capabilities that seL4 will transfer from the sender's CSpace.
    pub fn with_capabilities<T: sel4::CapType>(
        label: sel4::Word,
        words: &[sel4::Word],
        capabilities: &[sel4::Cap<T>],
    ) -> Result<Self, IpcError> {
        if words.len() > MESSAGE_CAPACITY {
            return Err(IpcError::MessageTooLong {
                length: words.len(),
                capacity: MESSAGE_CAPACITY,
            });
        }
        if capabilities.len() > EXTRA_CAPACITY {
            return Err(IpcError::TooManyCapabilities {
                count: capabilities.len(),
                capacity: EXTRA_CAPACITY,
            });
        }
        let mut message = Self::empty(label);
        message.words[..words.len()].copy_from_slice(words);
        message.length = words.len();
        for (destination, capability) in
            message.capabilities.iter_mut().zip(capabilities)
        {
            *destination = capability.bits();
        }
        message.extra_caps = capabilities.len();
        Ok(message)
    }

    /// Returns the message label without interpreting it as a protocol tag.
    pub fn label(&self) -> sel4::Word {
        self.label
    }

    /// Returns only the payload length reported for this message.
    pub fn words(&self) -> &[sel4::Word] {
        &self.words[..self.length]
    }

    /// Reads one reported message word after explicit bounds validation.
    pub fn word(&self, index: usize) -> Result<sel4::Word, IpcError> {
        if index >= self.length {
            return Err(IpcError::InvalidMessageIndex {
                index,
                length: self.length,
            });
        }
        self.words
            .get(index)
            .copied()
            .ok_or(IpcError::InvalidMessageIndex {
                index,
                length: self.length,
            })
    }

    /// Returns the number of extra capabilities reported by seL4.
    pub fn extra_caps(&self) -> usize {
        self.extra_caps
    }

    /// Returns the number of capability words unwrapped as badges by seL4.
    pub fn caps_unwrapped(&self) -> usize {
        self.caps_unwrapped
    }

    fn empty(label: sel4::Word) -> Self {
        Self {
            label,
            words: [0; MESSAGE_CAPACITY],
            length: 0,
            capabilities: [0; EXTRA_CAPACITY],
            extra_caps: 0,
            caps_unwrapped: 0,
        }
    }

    pub(crate) fn empty_reply() -> Self {
        Self::empty(0)
    }

    fn prepare(&self) -> sel4::MessageInfo {
        sel4::with_ipc_buffer_mut(|buffer| {
            buffer.msg_regs_mut()[..self.length].copy_from_slice(self.words());
            buffer.caps_or_badges_mut()[..self.extra_caps]
                .copy_from_slice(&self.capabilities[..self.extra_caps]);
        });
        sel4::MessageInfoBuilder::default()
            .label(self.label)
            .extra_caps(self.extra_caps)
            .length(self.length)
            .build()
    }

    fn received(info: &sel4::MessageInfo) -> Result<Self, IpcError> {
        if info.length() > MESSAGE_CAPACITY {
            return Err(IpcError::MessageTooLong {
                length: info.length(),
                capacity: MESSAGE_CAPACITY,
            });
        }
        if info.extra_caps() > EXTRA_CAPACITY {
            return Err(IpcError::TooManyCapabilities {
                count: info.extra_caps(),
                capacity: EXTRA_CAPACITY,
            });
        }
        let mut message = Self::empty(info.label());
        sel4::with_ipc_buffer(|buffer| {
            message.words[..info.length()]
                .copy_from_slice(&buffer.msg_regs()[..info.length()]);
        });
        message.length = info.length();
        message.extra_caps = info.extra_caps();
        message.caps_unwrapped = info.caps_unwrapped();
        Ok(message)
    }
}

/// An owned endpoint used for synchronous seL4 message rendezvous.
pub struct Endpoint {
    cap: sel4::cap::Endpoint,
    implicit_reply_pending: Cell<bool>,
}

impl Endpoint {
    /// Sends one message and blocks until a receiver accepts it.
    pub fn send(&self, message: &Message) {
        self.cap.send(message.prepare());
    }

    /// Attempts delivery without blocking when no receiver is waiting.
    pub fn try_send(&self, message: &Message) {
        self.cap.nb_send(message.prepare());
    }

    /// Performs a synchronous request and copies the bounded reply.
    pub fn call(&self, message: &Message) -> Result<Message, IpcError> {
        let info = self.cap.call(message.prepare());
        Message::received(&info)
    }

    /// Receives one message, optionally accepting one transferred capability.
    pub fn receive<'endpoint>(
        &'endpoint self,
        receive_slot: Option<ReceiveSlot<'_>>,
    ) -> Result<ReceivedMessage<'endpoint>, IpcError> {
        if self.implicit_reply_pending.replace(true) {
            return Err(IpcError::ReplyPending);
        }
        let capacity = usize::from(receive_slot.is_some());
        configure_receive_path(receive_slot.as_ref());
        let (info, raw_badge) = self.cap.recv(());
        if let Some(slot) = receive_slot {
            slot.finish(info.extra_caps() != 0)?;
        }
        if info.extra_caps() > capacity {
            self.clear_implicit_reply();
            return Err(IpcError::ReceiveCapacityExceeded {
                count: info.extra_caps(),
                capacity,
            });
        }
        let message = match Message::received(&info) {
            Ok(message) => message,
            Err(error) => {
                self.clear_implicit_reply();
                return Err(error);
            },
        };
        Ok(ReceivedMessage {
            endpoint: self,
            badge: Badge::new(raw_badge).ok(),
            message,
        })
    }

    /// Receives without blocking when no sender is waiting.
    pub fn try_receive<'endpoint>(
        &'endpoint self,
        receive_slot: Option<ReceiveSlot<'_>>,
    ) -> Result<Option<ReceivedMessage<'endpoint>>, IpcError> {
        if self.implicit_reply_pending.replace(true) {
            return Err(IpcError::ReplyPending);
        }
        let capacity = usize::from(receive_slot.is_some());
        configure_receive_path(receive_slot.as_ref());
        let (info, raw_badge) = self.cap.nb_recv(());
        if raw_badge == 0 && info.length() == 0 && info.extra_caps() == 0 {
            self.implicit_reply_pending.set(false);
            if let Some(slot) = receive_slot {
                slot.finish(false)?;
            }
            return Ok(None);
        }
        if let Some(slot) = receive_slot {
            slot.finish(info.extra_caps() != 0)?;
        }
        if info.extra_caps() > capacity {
            self.clear_implicit_reply();
            return Err(IpcError::ReceiveCapacityExceeded {
                count: info.extra_caps(),
                capacity,
            });
        }
        let message = match Message::received(&info) {
            Ok(message) => message,
            Err(error) => {
                self.clear_implicit_reply();
                return Err(error);
            },
        };
        Ok(Some(ReceivedMessage {
            endpoint: self,
            badge: Badge::new(raw_badge).ok(),
            message,
        }))
    }

    /// Returns the root-held endpoint for capability derivation in
    /// `cspace.rs`.
    pub(crate) fn cap(&self) -> sel4::cap::Endpoint {
        self.cap
    }

    fn clear_implicit_reply(&self) {
        sel4::with_ipc_buffer_mut(|buffer| {
            sel4::reply(buffer, sel4::MessageInfoBuilder::default().build());
        });
        self.implicit_reply_pending.set(false);
    }
}

/// A received message tied to the endpoint's current classic reply authority.
#[must_use = "reply, defer, or finish the received message before receiving again"]
pub struct ReceivedMessage<'a> {
    endpoint: &'a Endpoint,
    badge: Option<Badge>,
    message: Message,
}

impl ReceivedMessage<'_> {
    /// Returns the capability-path badge, or `None` for unbadged traffic.
    pub fn badge(&self) -> Option<Badge> {
        self.badge
    }

    /// Returns the bounded received message.
    pub fn message(&self) -> &Message {
        &self.message
    }

    /// Uses the current implicit authority to reply exactly once.
    pub fn reply(self, message: &Message) {
        let info = message.prepare();
        sel4::with_ipc_buffer_mut(|buffer| sel4::reply(buffer, info));
        self.endpoint.implicit_reply_pending.set(false);
    }

    /// Completes a one-way receive and clears any possible implicit authority.
    pub fn finish(self) {
        self.endpoint.clear_implicit_reply();
    }

    /// Preserves the current classic reply capability in a bounded root slot.
    pub fn defer(
        self,
        pool: &ReplyPool,
    ) -> Result<DeferredReply<'_>, IpcError> {
        let slot = match pool.reserve() {
            Some(slot) => slot,
            None => {
                self.endpoint.clear_implicit_reply();
                return Err(IpcError::NoReplySlots);
            },
        };
        if cspace::save_caller(slot).is_err() {
            pool.release(slot);
            self.endpoint.clear_implicit_reply();
            return Err(IpcError::NoReplyAuthority);
        }
        self.endpoint.implicit_reply_pending.set(false);
        Ok(DeferredReply { pool, slot })
    }
}

/// A bounded pool of root CSpace slots reserved for classic reply caps.
pub struct ReplyPool {
    slots: Range<usize>,
    occupied: Cell<u64>,
}

impl ReplyPool {
    fn reserve(&self) -> Option<usize> {
        let capacity = self.slots.len();
        let occupied = self.occupied.get();
        let available = (!occupied) & low_bits(capacity);
        if available == 0 {
            return None;
        }
        let offset = available.trailing_zeros() as usize;
        self.occupied.set(occupied | (1u64 << offset));
        self.slots.start.checked_add(offset)
    }

    fn release(&self, slot: usize) {
        if let Some(offset) = slot.checked_sub(self.slots.start) &&
            offset < self.slots.len()
        {
            self.occupied.set(self.occupied.get() & !(1u64 << offset));
        }
    }
}

/// One explicitly saved classic-kernel reply capability.
#[must_use = "reply or discard the saved reply authority"]
pub struct DeferredReply<'a> {
    pool: &'a ReplyPool,
    slot: usize,
}

impl DeferredReply<'_> {
    /// Replies to the blocked caller identified by this capability.
    pub fn reply(self, message: &Message) {
        sel4::cap::Unspecified::from_bits(self.slot).send(message.prepare());
        self.pool.release(self.slot);
    }

    /// Deletes an unused reply capability and releases its bounded slot.
    pub fn discard(self) -> Result<(), IpcError> {
        cspace::delete_root_slot(self.slot)
            .map_err(|_| IpcError::ReplyCapabilityOperationFailed)?;
        self.pool.release(self.slot);
        Ok(())
    }
}

/// A validated one-shot destination for at most one transferred capability.
pub struct ReceiveSlot<'a> {
    root: sel4::cap::CNode,
    size_bits: usize,
    reservation: SlotReservation<'a>,
}

impl ReceiveSlot<'_> {
    fn absolute(&self) -> sel4::AbsoluteCPtr {
        let index =
            SlotTracker::reservation_slot(&self.reservation) as sel4::Word;
        self.root
            .absolute_cptr_from_bits_with_depth(index, self.size_bits)
    }

    fn finish(self, received: bool) -> Result<(), IpcError> {
        let result = if received {
            SlotTracker::commit(self.reservation)
        } else {
            SlotTracker::cancel(self.reservation)
        };
        result.map_err(|_| IpcError::InvalidReceivePath)
    }
}

/// An owned asynchronous kernel notification.
pub struct Notification(sel4::cap::Notification);

impl Notification {
    /// Signals this notification without waiting for an observer.
    pub fn signal(&self) {
        self.0.signal();
    }

    /// Blocks until signalled and returns a nonzero observed badge when
    /// present.
    pub fn wait(&self) -> Option<Badge> {
        let (_, raw_badge) = self.0.wait();
        Badge::new(raw_badge).ok()
    }

    /// Polls without blocking and returns a nonzero observed badge when
    /// present.
    pub fn poll(&self) -> Option<Badge> {
        let (_, raw_badge) = self.0.poll();
        Badge::new(raw_badge).ok()
    }

    /// Returns the root-held capability for explicit delegation.
    pub(crate) fn cap(&self) -> sel4::cap::Notification {
        self.0
    }
}

impl Bootstrap<'_> {
    /// Allocates an endpoint through the common kernel-object allocator.
    pub fn allocate_endpoint(&mut self) -> Result<Endpoint, BootstrapError> {
        self.allocate_object::<sel4::cap_type::Endpoint>()
            .map(|cap| Endpoint {
                cap,
                implicit_reply_pending: Cell::new(false),
            })
    }

    /// Allocates a notification through the common kernel-object allocator.
    pub fn allocate_notification(
        &mut self,
    ) -> Result<Notification, BootstrapError> {
        self.allocate_object::<sel4::cap_type::Notification>()
            .map(Notification)
    }

    /// Reserves reusable root slots for independently retained classic
    /// replies.
    pub fn allocate_reply_pool(
        &mut self,
        capacity: usize,
    ) -> Result<ReplyPool, BootstrapError> {
        if capacity == 0 || capacity > MAX_DEFERRED_REPLIES {
            return Err(BootstrapError::InvalidTaskConfiguration);
        }
        let slots = self
            .reserve_empty_slots(capacity)
            .ok_or(BootstrapError::NoFreeSlots)?;
        Ok(ReplyPool {
            slots,
            occupied: Cell::new(0),
        })
    }
}

pub(crate) fn receive_slot<'a>(
    root: sel4::cap::CNode,
    size_bits: usize,
    slots: &'a mut SlotTracker,
    index: usize,
) -> Result<ReceiveSlot<'a>, IpcError> {
    let reservation = slots.reserve(index).map_err(map_slot_error)?;
    Ok(ReceiveSlot {
        root,
        size_bits,
        reservation,
    })
}

fn configure_receive_path(slot: Option<&ReceiveSlot<'_>>) {
    sel4::with_ipc_buffer_mut(|buffer| {
        if let Some(slot) = slot {
            buffer.set_recv_slot(&slot.absolute());
        } else {
            let null = sel4::cap::CNode::from_bits(0)
                .absolute_cptr_from_bits_with_depth(0, 0);
            buffer.set_recv_slot(&null);
        }
    });
}

fn map_slot_error(_error: SlotError) -> IpcError {
    IpcError::InvalidReceiveSlot
}

const fn low_bits(count: usize) -> u64 {
    if count == u64::BITS as usize {
        u64::MAX
    } else {
        (1u64 << count) - 1
    }
}

const _: () = {
    assert!(MESSAGE_CAPACITY > 0);
    assert!(EXTRA_CAPACITY == 4);
    assert!(MAX_DEFERRED_REPLIES <= u64::BITS as usize);
};
