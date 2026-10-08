use tokio::sync::broadcast;

/// Marker for an application's event enum.
///
/// An app defines exactly one event type — a single enum — and implements this
/// marker for it. One enum (not one type per event) is what lets a subscriber
/// `match` exhaustively and lets an agent see every event in one place.
///
/// ```ignore
/// #[derive(Debug, Clone)]
/// pub enum AppEvent {
///     UserCreated { id: Uuid },
///     UserDeleted { id: Uuid },
/// }
/// impl Event for AppEvent {}
/// ```
pub trait Event: Clone + Send + Sync + 'static {}

/// In-process, best-effort event bus held in application state.
///
/// Emitting never blocks the caller. Subscribers are background tasks registered
/// on [`crate::App`]; each receives every event and matches on the enum.
///
/// BOUNDARY: this is deliberately in-process and at-most-once. Durable or
/// cross-service delivery (an outbox table, a broker) is a later, additive
/// concern — see ARCHITECTURE.md § Events. Do not grow this into a message queue.
#[derive(Clone)]
pub struct Events<E: Event> {
    tx: broadcast::Sender<E>,
}

impl<E: Event> Events<E> {
    /// Capacity is the per-subscriber buffer. A slow subscriber that falls
    /// behind by more than `capacity` events observes a lag error on receive;
    /// it must treat that as "resync from source of truth", not panic.
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Events { tx }
    }

    /// Publish an event. Returns immediately. If there are no subscribers the
    /// event is dropped — emission is a notification, not a durable write.
    pub fn emit(&self, event: E) {
        let _ = self.tx.send(event);
    }

    /// Subscribe. Used by [`crate::App`] when registering handlers; application
    /// code emits, the framework wires up receivers.
    pub fn subscribe(&self) -> broadcast::Receiver<E> {
        self.tx.subscribe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    enum TestEvent {
        Ping(u32),
    }
    impl Event for TestEvent {}

    #[tokio::test]
    async fn subscriber_receives_emitted_event() {
        let events = Events::new(8);
        let mut rx = events.subscribe();
        events.emit(TestEvent::Ping(42));
        assert_eq!(rx.recv().await.unwrap(), TestEvent::Ping(42));
    }

    #[test]
    fn emit_without_subscribers_does_not_panic() {
        let events: Events<TestEvent> = Events::new(8);
        events.emit(TestEvent::Ping(1)); // dropped silently, must not panic
    }
}
