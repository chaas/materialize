// Copyright Materialize, Inc. and contributors. All rights reserved.
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

//! Logging dataflows for events generated by various subsystems.

pub mod compute;
mod differential;
mod initialize;
mod reachability;
mod timely;

use std::any::Any;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Duration;

use ::timely::dataflow::operators::capture::{Event, EventLink, EventPusher};
use ::timely::logging::WorkerIdentifier;
use ::timely::progress::Timestamp as TimelyTimestamp;
use ::timely::scheduling::Activator;
use mz_compute_client::logging::{ComputeLog, DifferentialLog, LogVariant, TimelyLog};
use mz_expr::{permutation_for_arrangement, MirScalarExpr};
use mz_repr::{Datum, Diff, Row, RowPacker, SharedRow, Timestamp};
use mz_timely_util::activator::RcActivator;

use crate::logging::compute::Logger as ComputeLogger;
use crate::typedefs::RowRowAgent;

pub use crate::logging::initialize::initialize;

/// Logs events as a timely stream, with progress statements.
struct BatchLogger<T, E, P>
where
    P: EventPusher<Timestamp, (Duration, E, T)>,
{
    /// Time in milliseconds of the current expressed capability.
    time_ms: Timestamp,
    event_pusher: P,
    _phantom: ::std::marker::PhantomData<(E, T)>,
    /// Each time is advanced to the strictly next millisecond that is a multiple of this interval.
    /// This means we should be able to perform the same action on timestamp capabilities, and only
    /// flush buffers when this timestamp advances.
    interval_ms: u64,
    /// A stash for data that does not yet need to be sent.
    buffer: Vec<(Duration, E, T)>,
}

impl<T, E, P> BatchLogger<T, E, P>
where
    P: EventPusher<Timestamp, (Duration, E, T)>,
{
    /// Batch size in bytes for batches
    const BATCH_SIZE_BYTES: usize = 1 << 13;

    /// Calculate the default buffer size based on `(Duration, E, T)` tuples.
    fn buffer_capacity() -> usize {
        let size = ::std::mem::size_of::<(Duration, E, T)>();
        if size == 0 {
            Self::BATCH_SIZE_BYTES
        } else if size <= Self::BATCH_SIZE_BYTES {
            Self::BATCH_SIZE_BYTES / size
        } else {
            1
        }
    }

    /// Creates a new batch logger.
    fn new(event_pusher: P, interval_ms: u64) -> Self {
        BatchLogger {
            time_ms: Timestamp::minimum(),
            event_pusher,
            _phantom: ::std::marker::PhantomData,
            interval_ms,
            buffer: Vec::with_capacity(Self::buffer_capacity()),
        }
    }

    /// Publishes a batch of logged events and advances the capability.
    fn publish_batch(&mut self, time: &Duration, data: &mut Vec<(Duration, E, T)>) {
        // TODO(benesch): avoid dangerous `as` conversion.
        #[allow(clippy::as_conversions)]
        let new_time_ms = Timestamp::try_from(
            (((time.as_millis() as u64) / self.interval_ms) + 1) * self.interval_ms,
        )
        .expect("must fit");
        if !data.is_empty() {
            // If we don't need to grow our buffer, move
            if data.len() > self.buffer.capacity() - self.buffer.len() {
                self.event_pusher.push(Event::Messages(
                    self.time_ms,
                    self.buffer.drain(..).collect(),
                ));
            }

            self.buffer.append(data);
        }
        if self.time_ms < new_time_ms {
            // Flush buffered events that may need to advance.
            self.event_pusher.push(Event::Messages(
                self.time_ms,
                self.buffer.drain(..).collect(),
            ));
            if self.buffer.capacity() > Self::buffer_capacity() {
                self.buffer = Vec::with_capacity(Self::buffer_capacity())
            }

            // In principle we can buffer up until this point, if that is appealing to us.
            // We could buffer more aggressively if the logging interval were exposed
            // here, as the forward ticks would be that much less frequent.
            self.event_pusher
                .push(Event::Progress(vec![(new_time_ms, 1), (self.time_ms, -1)]));
        }
        self.time_ms = new_time_ms;
    }
}
impl<T, E, P> Drop for BatchLogger<T, E, P>
where
    P: EventPusher<Timestamp, (Duration, E, T)>,
{
    fn drop(&mut self) {
        self.event_pusher
            .push(Event::Progress(vec![(self.time_ms, -1)]));
    }
}

/// Parts to connect a logging dataflows the timely runtime.
///
/// This is just a bundle-type intended to make passing around its contents in the logging
/// initialization code more convenient.
#[derive(Clone)]
struct EventQueue<E> {
    link: Rc<EventLink<Timestamp, (Duration, WorkerIdentifier, E)>>,
    activator: RcActivator,
}

impl<E> EventQueue<E> {
    fn new(name: &str) -> Self {
        let activator_name = format!("{name}_activator");
        let activate_after = 128;
        Self {
            link: Rc::new(EventLink::new()),
            activator: RcActivator::new(activator_name, activate_after),
        }
    }
}

/// State shared between different logging dataflows.
#[derive(Default)]
struct SharedLoggingState {
    /// Activators for arrangement heap size operators.
    arrangement_size_activators: BTreeMap<usize, Activator>,
    /// Shared compute logger.
    compute_logger: Option<ComputeLogger>,
}

/// Helper to pack collections of [`Datum`]s into key and value row.
pub(crate) struct PermutedRowPacker {
    key: Vec<usize>,
    value: Vec<usize>,
}

impl PermutedRowPacker {
    /// Construct based on the information within the log variant.
    pub(crate) fn new<V: Into<LogVariant>>(variant: V) -> Self {
        let variant = variant.into();
        let key = variant.index_by();
        let (_, value) = permutation_for_arrangement(
            &key.iter()
                .cloned()
                .map(MirScalarExpr::Column)
                .collect::<Vec<_>>(),
            variant.desc().arity(),
        );
        Self { key, value }
    }

    /// Pack a slice of datums suitable for the key columns in the log variant.
    pub(crate) fn pack_slice(&mut self, datums: &[Datum]) -> (Row, Row) {
        self.pack_by_index(|packer, index| packer.push(datums[index]))
    }

    /// Pack using a callback suitable for the key columns in the log variant.
    pub(crate) fn pack_by_index<F: Fn(&mut RowPacker, usize)>(&mut self, logic: F) -> (Row, Row) {
        let binding = SharedRow::get();
        let mut row_builder = binding.borrow_mut();

        let mut packer = row_builder.packer();
        for index in &self.key {
            logic(&mut packer, *index);
        }
        let key_row = row_builder.clone();

        let mut packer = row_builder.packer();
        for index in &self.value {
            logic(&mut packer, *index);
        }
        let value_row = row_builder.clone();

        (key_row, value_row)
    }
}

/// Information about a collection exported from a logging dataflow.
struct LogCollection {
    /// Trace handle providing access to the logged records.
    trace: RowRowAgent<Timestamp, Diff>,
    /// Token that should be dropped to drop this collection.
    token: Rc<dyn Any>,
    /// Index of the dataflow exporting this collection.
    dataflow_index: usize,
}