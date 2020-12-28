use crate::actor_runtime::{Actor, Status};
use crate::handlers::{Operation, TaskEliminated};
use crate::ids::{Id, IdOf};
use crate::lifecycle::{LifecycleNotifier, TaskDone};
use crate::linkage::Address;
use anyhow::Error;
use async_trait::async_trait;
use futures::{
    future::{select, Either, FusedFuture},
    Future, FutureExt,
};
use std::pin::Pin;
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::sync::watch;
use tokio::time::delay_until;
use uuid::Uuid;

/// Minimalistic actor that hasn't `Address`.
///
/// **Recommended** to implement sequences or intensive loops (routines).
#[async_trait]
pub trait LiteTask: Sized + Send + 'static {
    /// The result of a finished task.
    type Output: Send;

    /// Returns unique name of the `LiteTask`.
    /// Uses `Uuid` by default.
    fn name(&self) -> String {
        let uuid = Uuid::new_v4();
        format!("Task:{}({})", std::any::type_name::<Self>(), uuid)
    }

    /// Routine of the task that can contain loops.
    /// It can taks into accout provided receiver to implement graceful interruption.
    ///
    /// By default uses the following calling chain that you can override at any step:
    /// `routine` -> `interruptable_routine` -> `repeatable_routine` -> `retry_at` -> `retry_delay`
    async fn routine(self, mut stop: StopReceiver) -> Result<Self::Output, Error> {
        stop.or(self.interruptable_routine())
            .await
            .map_err(Error::from)
            // TODO: use `flatten` instead
            .and_then(std::convert::identity)
    }

    /// Routine that can be unconditionally interrupted.
    async fn interruptable_routine(mut self) -> Result<Self::Output, Error> {
        loop {
            let last_attempt = Instant::now();
            if let Err(err) = self.repeatable_routine().await {
                log::error!("Routine {} failed: {}", self.name(), err);
            }
            let instant = self.retry_at(last_attempt);
            delay_until(instant.into()).await;
        }
    }

    /// Routine that can be retried in case of fail.
    async fn repeatable_routine(&mut self) -> Result<(), Error> {
        Ok(())
    }

    /// When to do the next attempt for `repeatable_routine`.
    fn retry_at(&self, _last_attempt: Instant) -> Instant {
        Instant::now() + self.retry_delay(_last_attempt)
    }

    /// How long to wait to retry. Called by `retry_at` method.
    fn retry_delay(&self, _last_attempt: Instant) -> Duration {
        Duration::from_secs(5)
    }
}

pub(crate) fn spawn<T, S>(task: T, supervisor: Option<Address<S>>) -> StopSender
where
    T: LiteTask,
    S: Actor + TaskEliminated<T>,
{
    let id = Id::of_task(&task);
    let (stop_sender, stop_receiver) = stop_channel(id.clone());
    let id_of = IdOf::<T>::new(id.clone());
    let done_notifier = {
        match supervisor {
            None => LifecycleNotifier::ignore(),
            Some(super_addr) => {
                let event = TaskDone::new(id_of);
                let op = Operation::Done { id: id.clone() };
                LifecycleNotifier::once(super_addr, op, event)
            }
        }
    };
    let runtime = LiteRuntime {
        id,
        task,
        done_notifier,
        stop_receiver,
    };
    tokio::spawn(runtime.entrypoint());
    stop_sender
}

/// Just receives a stop signal.
pub trait StopSignal: Future<Output = ()> + FusedFuture + Send {}

impl<T> StopSignal for T where T: Future<Output = ()> + FusedFuture + Send {}

#[derive(Debug, Error)]
#[error("task interrupted by a signal")]
pub struct TaskStopped;

pub fn stop_channel(id: Id) -> (StopSender, StopReceiver) {
    let (tx, rx) = watch::channel(Status::Alive);
    let sender = StopSender { id, tx };
    let receiver = StopReceiver { status: rx };
    (sender, receiver)
}

// TODO: Rename to `TaskAddress`
// TODO: Make it cloneable
#[derive(Debug)]
pub struct StopSender {
    id: Id,
    tx: watch::Sender<Status>,
}

impl StopSender {
    // TODO: Return `IdOf`
    pub fn id(&self) -> Id {
        self.id.clone()
    }

    pub fn stop(&self) -> Result<(), Error> {
        self.tx.broadcast(Status::Stop).map_err(Error::from)
    }
}

/// Contains a receiver with a status of a task.
#[derive(Debug, Clone)]
pub struct StopReceiver {
    status: watch::Receiver<Status>,
}

impl StopReceiver {
    /// Returns `true` is the task can be alive.
    pub fn is_alive(&self) -> bool {
        *self.status.borrow() == Status::Alive
    }

    /// Returns a `Future` that completed when `Done` signal received.
    pub fn into_future(self) -> Pin<Box<dyn StopSignal>> {
        Box::pin(just_done(self.status).fuse())
    }

    /// Tries to execute provided `Future` to completion if the `ShutdownReceived`
    /// won't interrupted during that time.
    pub async fn or<Fut>(&mut self, fut: Fut) -> Result<Fut::Output, TaskStopped>
    where
        Fut: Future,
    {
        tokio::pin!(fut);
        let either = select(self.clone().into_future(), fut).await;
        match either {
            Either::Left((_done, _rem_fut)) => Err(TaskStopped),
            Either::Right((output, _rem_fut)) => Ok(output),
        }
    }
}

async fn just_done(mut status: watch::Receiver<Status>) {
    // TODO: tokio 0.3
    // while status.changed().await.is_ok() {
    while status.recv().await.is_some() {
        if status.borrow().is_done() {
            break;
        }
    }
}

struct LiteRuntime<T: LiteTask> {
    // TODO: Use `IdOf` here
    id: Id,
    task: T,
    done_notifier: Box<dyn LifecycleNotifier>,
    stop_receiver: StopReceiver,
}

impl<T: LiteTask> LiteRuntime<T> {
    async fn entrypoint(mut self) {
        log::info!("Task started: {:?}", self.id);
        if let Err(err) = self.task.routine(self.stop_receiver).await {
            if let Err(real_err) = err.downcast::<TaskStopped>() {
                // Can't downcast. It was a real error.
                log::error!("Task failed: {:?}: {}", self.id, real_err);
            }
        }
        log::info!("Task finished: {:?}", self.id);
        if let Err(err) = self.done_notifier.notify() {
            log::error!(
                "Can't send done notification from the task {:?}: {}",
                self.id,
                err
            );
        }
    }
}
