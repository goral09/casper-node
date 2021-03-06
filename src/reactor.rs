//! Reactor core.
//!
//! Any long running instance of the node application uses an event-dispatch pattern: Events are
//! generated and stored on an event queue, then processed one-by-one. This process happens inside
//! the reactor*, which also exclusively holds the state of the application besides pending events:
//!
//! 1. The reactor pops an event off the event queue (called a [`Scheduler`](type.Scheduler.html)).
//! 2. The event is dispatched by the reactor. Since the reactor holds mutable state, it can grant
//!    any component that processes an event mutable, exclusive access to its state.
//! 3. Once the (synchronous) event processing has completed, the component returns an effect.
//! 4. The reactor spawns a task that executes these effects and eventually schedules another event.
//! 5. meanwhile go to 1.
//!
//! # Reactors
//!
//! There is no single reactor, but rather a reactor for each application type, since it defines
//! which components are used and how they are wired up. The reactor defines the state by being a
//! `struct` of components, their initialization through the
//! [`Reactor::new()`](trait.Reactor.html#tymethod.new) and a method
//! [`Reactor::dispatch_event()`](trait.Reactor.html#tymethod.dispatch_event) to dispatch events to
//! components.
//!
//! With all these set up, a reactor can be [`run`](fn.run.html), causing it to execute
//! indefinitely, processing events.

mod error;
pub mod non_validator;
mod queue_kind;
pub mod validator;

use std::{
    fmt::{Debug, Display},
    mem,
};

use futures::FutureExt;
use tracing::{debug, info, trace, warn};

use crate::{
    effect::{Effect, EffectBuilder, Multiple},
    utils::{self, WeightedRoundRobin},
    ApiServerConfig, SmallNetworkConfig, StorageConfig,
};
pub use error::Error;
pub(crate) use error::Result;
pub use queue_kind::QueueKind;

/// Event scheduler
///
/// The scheduler is a combination of multiple event queues that are polled in a specific order. It
/// is the central hook for any part of the program that schedules events directly.
///
/// Components rarely use this, but use a bound `EventQueueHandle` instead.
pub type Scheduler<Ev> = WeightedRoundRobin<Ev, QueueKind>;

/// Event queue handle
///
/// The event queue handle is how almost all parts of the application interact with the reactor
/// outside of the normal event loop. It gives different parts a chance to schedule messages that
/// stem from things like external IO.
#[derive(Debug)]
pub struct EventQueueHandle<REv: 'static>(&'static Scheduler<REv>);

// Implement `Clone` and `Copy` manually, as `derive` will make it depend on `R` and `Ev` otherwise.
impl<REv> Clone for EventQueueHandle<REv> {
    fn clone(&self) -> Self {
        EventQueueHandle(self.0)
    }
}
impl<REv> Copy for EventQueueHandle<REv> {}

impl<REv> EventQueueHandle<REv> {
    pub(crate) fn new(scheduler: &'static Scheduler<REv>) -> Self {
        EventQueueHandle(scheduler)
    }

    /// Schedule an event on a specific queue.
    #[inline]
    pub(crate) async fn schedule<Ev>(self, event: Ev, queue_kind: QueueKind)
    where
        REv: From<Ev>,
    {
        self.0.push(event.into(), queue_kind).await
    }
}

/// Reactor core.
///
/// Any reactor should implement this trait and be executed by the `reactor::run` function.
pub(crate) trait Reactor: Sized {
    // Note: We've gone for the `Sized` bound here, since we return an instance in `new`. As an
    // alternative, `new` could return a boxed instance instead, removing this requirement.

    /// Event type associated with reactor.
    ///
    /// Defines what kind of event the reactor processes.
    type Event: Send + Debug + Display + 'static;

    /// Dispatches an event on the reactor.
    ///
    /// This function is typically only called by the reactor itself to dispatch an event. It is
    /// safe to call regardless, but will cause the event to skip the queue and things like
    /// accounting.
    fn dispatch_event(
        &mut self,
        effect_builder: EffectBuilder<Self::Event>,
        event: Self::Event,
    ) -> Multiple<Effect<Self::Event>>;

    /// Creates a new instance of the reactor.
    ///
    /// This method creates the full state, which consists of all components, and returns a reactor
    /// instances along with the effects the components generated upon instantiation.
    ///
    /// If any instantiation fails, an error is returned.
    fn new(
        validator_network_config: SmallNetworkConfig,
        api_server_config: ApiServerConfig,
        storage_config: StorageConfig,
        event_queue: EventQueueHandle<Self::Event>,
    ) -> Result<(Self, Multiple<Effect<Self::Event>>)>;
}

/// Runs a reactor.
///
/// Starts the reactor and associated background tasks, then enters main the event processing loop.
///
/// `run` will leak memory on start for global structures each time it is called.
///
/// Errors are returned only if component initialization fails.
#[inline]
async fn run<R: Reactor>(
    validator_network_config: SmallNetworkConfig,
    api_server_config: ApiServerConfig,
    storage_config: StorageConfig,
) -> Result<()> {
    let event_size = mem::size_of::<R::Event>();
    // Check if the event is of a reasonable size. This only emits a runtime warning at startup
    // right now, since storage size of events is not an issue per se, but copying might be
    // expensive if events get too large.
    if event_size > 16 * mem::size_of::<usize>() {
        warn!(
            "event size is {} bytes, consider reducing it or boxing",
            event_size
        );
    }

    let scheduler = Scheduler::<R::Event>::new(QueueKind::weights());

    // Create a new event queue for this reactor run.
    let scheduler = utils::leak(scheduler);

    let event_queue = EventQueueHandle::new(scheduler);
    let (mut reactor, initial_effects) = R::new(
        validator_network_config,
        api_server_config,
        storage_config,
        event_queue,
    )?;

    // Run all effects from component instantiation.
    process_effects(scheduler, initial_effects).await;

    info!("entering reactor main loop");
    let effect_builder = EffectBuilder::new(event_queue);
    loop {
        let (event, q) = scheduler.pop().await;

        // We log events twice, once in display and once in debug mode.
        debug!(%event, ?q);
        trace!(?event, ?q);

        // Dispatch the event, then execute the resulting effect.
        let effects = reactor.dispatch_event(effect_builder, event);
        process_effects(scheduler, effects).await;
    }
}

/// Spawns tasks that will process the given effects.
#[inline]
async fn process_effects<Ev>(scheduler: &'static Scheduler<Ev>, effects: Multiple<Effect<Ev>>)
where
    Ev: Send + 'static,
{
    // TODO: Properly carry around priorities.
    let queue_kind = QueueKind::default();

    for effect in effects {
        tokio::spawn(async move {
            for event in effect.await {
                scheduler.push(event, queue_kind).await
            }
        });
    }
}

/// Converts a single effect into another by wrapping it.
#[inline]
pub fn wrap_effect<Ev, REv, F>(wrap: F, effect: Effect<Ev>) -> Effect<REv>
where
    F: Fn(Ev) -> REv + Send + 'static,
    Ev: Send + 'static,
    REv: Send + 'static,
{
    // TODO: The double-boxing here is very unfortunate =(.
    (async move {
        let events: Multiple<Ev> = effect.await;
        events.into_iter().map(wrap).collect()
    })
    .boxed()
}

/// Converts multiple effects into another by wrapping.
#[inline]
pub fn wrap_effects<Ev, REv, F>(wrap: F, effects: Multiple<Effect<Ev>>) -> Multiple<Effect<REv>>
where
    F: Fn(Ev) -> REv + Send + 'static + Clone,
    Ev: Send + 'static,
    REv: Send + 'static,
{
    effects
        .into_iter()
        .map(move |effect| wrap_effect(wrap.clone(), effect))
        .collect()
}
