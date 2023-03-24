// Copyright Materialize, Inc. and contributors. All rights reserved.
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

// BEGIN LINT CONFIG
// DO NOT EDIT. Automatically generated by bin/gen-lints.
// Have complaints about the noise? See the note in misc/python/materialize/cli/gen-lints.py first.
#![allow(clippy::style)]
#![allow(clippy::complexity)]
#![allow(clippy::large_enum_variant)]
#![allow(clippy::mutable_key_type)]
#![allow(clippy::stable_sort_primitive)]
#![allow(clippy::map_entry)]
#![allow(clippy::box_default)]
#![warn(clippy::bool_comparison)]
#![warn(clippy::clone_on_ref_ptr)]
#![warn(clippy::no_effect)]
#![warn(clippy::unnecessary_unwrap)]
#![warn(clippy::dbg_macro)]
#![warn(clippy::todo)]
#![warn(clippy::wildcard_dependencies)]
#![warn(clippy::zero_prefixed_literal)]
#![warn(clippy::borrowed_box)]
#![warn(clippy::deref_addrof)]
#![warn(clippy::double_must_use)]
#![warn(clippy::double_parens)]
#![warn(clippy::extra_unused_lifetimes)]
#![warn(clippy::needless_borrow)]
#![warn(clippy::needless_question_mark)]
#![warn(clippy::needless_return)]
#![warn(clippy::redundant_pattern)]
#![warn(clippy::redundant_slicing)]
#![warn(clippy::redundant_static_lifetimes)]
#![warn(clippy::single_component_path_imports)]
#![warn(clippy::unnecessary_cast)]
#![warn(clippy::useless_asref)]
#![warn(clippy::useless_conversion)]
#![warn(clippy::builtin_type_shadow)]
#![warn(clippy::duplicate_underscore_argument)]
#![warn(clippy::double_neg)]
#![warn(clippy::unnecessary_mut_passed)]
#![warn(clippy::wildcard_in_or_patterns)]
#![warn(clippy::collapsible_if)]
#![warn(clippy::collapsible_else_if)]
#![warn(clippy::crosspointer_transmute)]
#![warn(clippy::excessive_precision)]
#![warn(clippy::overflow_check_conditional)]
#![warn(clippy::as_conversions)]
#![warn(clippy::match_overlapping_arm)]
#![warn(clippy::zero_divided_by_zero)]
#![warn(clippy::must_use_unit)]
#![warn(clippy::suspicious_assignment_formatting)]
#![warn(clippy::suspicious_else_formatting)]
#![warn(clippy::suspicious_unary_op_formatting)]
#![warn(clippy::mut_mutex_lock)]
#![warn(clippy::print_literal)]
#![warn(clippy::same_item_push)]
#![warn(clippy::useless_format)]
#![warn(clippy::write_literal)]
#![warn(clippy::redundant_closure)]
#![warn(clippy::redundant_closure_call)]
#![warn(clippy::unnecessary_lazy_evaluations)]
#![warn(clippy::partialeq_ne_impl)]
#![warn(clippy::redundant_field_names)]
#![warn(clippy::transmutes_expressible_as_ptr_casts)]
#![warn(clippy::unused_async)]
#![warn(clippy::disallowed_methods)]
#![warn(clippy::disallowed_macros)]
#![warn(clippy::disallowed_types)]
#![warn(clippy::from_over_into)]
// END LINT CONFIG

//! Open-loop benchmark for measuring UPSERT performance. Run using
//!
//! ```
//! $ cargo run --example upsert_open_loop -- --runtime 10sec
//! ```
//!
//!
//! Notes from @guswynn's testing, and remaining work before this
//! benchmark can be trusted:
//!
//! - Reduce some extraneous clones.
//! - Make the data-generator actually generate new values for previous keys (right now, all keys
//! are unique)
//! - Ensure that rocksb has read-your-writes, in-process, without "transactions" (docs are unclear here)
//! - Limit the size of write batches (and possibly multi-gets, based on
//!     <https://github.com/facebook/rocksdb/wiki/RocksDB-FAQ#basic-readwrite>).
//!     - Make it possible to configure the MemTable type (we probably want `Vector` for this
//!     workload).
//! - Ensure sst files are actually being written, from all workers.
//! - Figure out why this workload has large numbers of empty batches (???)
//! - Have at least a few people sign off on this being a reasonable benchmark.
//!     - In debug mode on a laptop, it seems to sometimes be cpu-bound, not storage-bound.
//! - Figure out if the code is broken, of its just macos that causes multi-second (sometimes 10+
//! second) stalls.
//! - Sort values before writing them.
//! - Improve the metrics we write. Currently `lag` is very opaque.
//!
//! Additional notes:
//! - Its unclear if the single-thread-per-rocksdb-instance model is performant, or considered a
//! reasonable model.
//! - Its unclear if fsync can be entirely turned off in rocksdb. If it can, we should turn that
//! off
//! - I think shutdown is a bit iffy right now, sometimes a pthread error can happen...
//!

#![allow(clippy::cast_precision_loss)]

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::bail;
use differential_dataflow::Hashable;
use mz_ore::task;
use mz_timely_util::probe::{Handle, ProbeNotify};
use timely::dataflow::channels::pact::Exchange;
use timely::dataflow::operators::Operator;
use timely::dataflow::{Scope, Stream};
use timely::progress::Antichain;
use tokio::sync::mpsc::error::SendError;
use tokio::sync::mpsc::UnboundedReceiver;
use tokio::task::JoinHandle;
use tracing::{debug, error, info, info_span, trace, Instrument};

use mz_build_info::{build_info, BuildInfo};
use mz_orchestrator_tracing::{StaticTracingConfig, TracingCliArgs};
use mz_ore::cast::CastFrom;
use mz_ore::cli::{self, CliConfig};
use mz_ore::metrics::MetricsRegistry;
use mz_persist::indexed::columnar::ColumnarRecords;
use mz_persist::workload::DataGenerator;
use mz_timely_util::builder_async::{Event as AsyncEvent, OperatorBuilder as AsyncOperatorBuilder};

// TODO(aljoscha): Make workload configurable: cardinality of keyspace, hot vs. cold keys, the
// works.
//
// Note that this module is currently unused.
#[path = "upsert_open_loop/workload.rs"]
mod workload;

const BUILD_INFO: BuildInfo = build_info!();

/// Open-loop benchmark for persistence.
#[derive(Debug, clap::Parser)]
pub struct Args {
    /// Number of sources.
    #[clap(long, value_name = "W", default_value_t = 1)]
    num_sources: usize,

    /// Number of (timely) workers/threads.
    #[clap(long, value_name = "R", default_value_t = 1)]
    num_workers: usize,

    /// Runtime in a whole number of seconds
    #[clap(long, parse(try_from_str = humantime::parse_duration), value_name = "S", default_value = "60s")]
    runtime: Duration,

    /// How many records writers should emit per second.
    #[clap(long, value_name = "R", default_value_t = 10000)]
    records_per_second: usize,

    /// Size of records (goodbytes) in bytes.
    #[clap(long, value_name = "B", default_value_t = 64)]
    record_size_bytes: usize,

    /// Batch size in number of records (if applicable).
    #[clap(long, env = "", value_name = "R", default_value_t = 100)]
    batch_size: usize,

    /// Duration between subsequent informational log outputs.
    #[clap(long, parse(try_from_str = humantime::parse_duration), value_name = "L", default_value = "1s")]
    logging_granularity: Duration,

    /// The address of the internal HTTP server.
    #[clap(long, value_name = "HOST:PORT", default_value = "127.0.0.1:6878")]
    internal_http_listen_addr: SocketAddr,

    /// Path of a file to write metrics at the end of the run.
    #[clap(long)]
    metrics_file: Option<String>,

    #[clap(flatten)]
    tracing: TracingCliArgs,

    // RocksDB settings
    /// Whether or not to use rocksdb. Defaults to using an in-memory hashmap.
    #[clap(long)]
    use_rocksdb: bool,

    /// Whether or not to use the WAL in rocksdb.
    #[clap(long)]
    use_wal: bool,

    /// Whether or not to cleanup the rocksdb instances
    /// in the temporary directory.
    #[clap(long)]
    dont_cleanup_rocksdb: bool,
    /*
    /// Use rocksdb transactional db
    #[clap(long)]
    use_rocksb_transactions: bool,
    */
}

fn main() {
    let args: Args = cli::parse_args(CliConfig::default());

    // Mirror the tokio Runtime configuration in our production binaries.
    let ncpus_useful = usize::max(1, std::cmp::min(num_cpus::get(), num_cpus::get_physical()));
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(ncpus_useful)
        .enable_all()
        .build()
        .expect("Failed building the Runtime");

    let _ = runtime
        .block_on(args.tracing.configure_tracing(StaticTracingConfig {
            service_name: "upsert-open-loop",
            build_info: BUILD_INFO,
        }))
        .expect("failed to init tracing");

    let root_span = info_span!("upsert_open_loop");
    let res = runtime.block_on(run(args).instrument(root_span));

    if let Err(err) = res {
        eprintln!("error: {:#}", err);
        std::process::exit(1);
    }
}

pub async fn run(args: Args) -> Result<(), anyhow::Error> {
    let metrics_registry = MetricsRegistry::new();
    {
        let metrics_registry = metrics_registry.clone();
        info!(
            "serving internal HTTP server on http://{}/metrics",
            args.internal_http_listen_addr
        );
        mz_ore::task::spawn(
            || "http_server",
            axum::Server::bind(&args.internal_http_listen_addr).serve(
                axum::Router::new()
                    .route(
                        "/metrics",
                        axum::routing::get(move || async move {
                            mz_http_util::handle_prometheus(&metrics_registry).await
                        }),
                    )
                    .into_make_service(),
            ),
        );
    }

    let num_sources = args.num_sources;
    let num_workers = args.num_workers;
    run_benchmark(args, metrics_registry, num_sources, num_workers).await
}

async fn run_benchmark(
    args: Args,
    _metrics_registry: MetricsRegistry,
    num_sources: usize,
    num_workers: usize,
) -> Result<(), anyhow::Error> {
    let num_records_total = args.records_per_second * usize::cast_from(args.runtime.as_secs());
    let data_generator =
        DataGenerator::new(num_records_total, args.record_size_bytes, args.batch_size);

    let benchmark_description = format!(
        "num-sources={} num-workers={} runtime={:?} num_records_total={} records-per-second={} record-size-bytes={} batch-size={}",
        args.num_sources, args.num_workers, args.runtime, num_records_total, args.records_per_second,
        args.record_size_bytes, args.batch_size);

    info!("starting benchmark: {}", benchmark_description);

    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::unbounded_channel();
    let progress_tx = Arc::new(Mutex::new(Some(progress_tx)));

    let mut generator_handles: Vec<JoinHandle<Result<String, anyhow::Error>>> = vec![];
    let mut source_rxs = BTreeMap::new();

    // All workers should have the starting time (so they can consistently track progress
    // and reason about lag independently).
    let start = Instant::now();

    // This controls the time that data generators (and in turn sources) downgrade to. We know this,
    // and probe the time at the end of the pipeline to figure out the lag.
    let shared_source_time = Arc::new(AtomicU64::new(
        start.elapsed().as_millis().try_into().unwrap(),
    ));

    // The batch interarrival time. We'll use this quantity to rate limit the
    // data generation.
    // No other known way to convert `usize` to `f64`.
    #[allow(clippy::as_conversions)]
    let time_per_batch = {
        let records_per_second_f64 = args.records_per_second as f64;
        let batch_size_f64 = args.batch_size as f64;

        let batches_per_second = records_per_second_f64 / batch_size_f64;
        Duration::from_secs(1).div_f64(batches_per_second)
    };

    for source_id in 0..num_sources {
        let data_generator = data_generator.clone();
        let start = start.clone();

        let (generator_tx, generator_rx) = tokio::sync::mpsc::unbounded_channel();
        source_rxs.insert(source_id, generator_rx);

        let shared_source_time = Arc::clone(&shared_source_time);

        // Intentionally create the span outside the task to set the parent.
        let generator_span = info_span!("generator", source_id);
        let data_generator_handle = mz_ore::task::spawn(
            || format!("data-generator-{}", source_id),
            async move {
                info!("starting data generator {}", source_id);

                // The number of batches this data generator has sent over to the
                // corresponding writer task.
                let mut batch_idx = 0;
                // The last time we emitted progress information to stdout, expressed
                // as a relative duration from start.
                let mut prev_log = Duration::from_millis(0);

                let mut current_source_time = shared_source_time.load(Ordering::SeqCst);

                loop {
                    // Data generation can be CPU expensive, so generate it
                    // in a spawn_blocking to play nicely with the rest of
                    // the async code.
                    let mut data_generator = data_generator.clone();
                    // Intentionally create the span outside the task to set the
                    // parent.
                    let batch_span = info_span!("batch", batch_idx);
                    let batch = mz_ore::task::spawn_blocking(
                        || "data_generator-batch",
                        move || {
                            batch_span
                                .in_scope(|| data_generator.gen_batch(usize::cast_from(batch_idx)))
                        },
                    )
                    .await
                    .expect("task failed");
                    trace!("data generator {} made a batch", source_id);
                    let batch = match batch {
                        Some(x) => x,
                        None => {
                            let records_sent = usize::cast_from(batch_idx) * args.batch_size;
                            let finished = format!(
                                "Data generator {} finished after {} ms and sent {} records",
                                source_id,
                                start.elapsed().as_millis(),
                                records_sent
                            );
                            return Ok(finished);
                        }
                    };
                    batch_idx += 1;

                    // Sleep so this doesn't busy wait if it's ahead of
                    // schedule.
                    let elapsed = start.elapsed();
                    let next_batch_time = time_per_batch * (batch_idx);
                    let sleep = next_batch_time.saturating_sub(elapsed);
                    if sleep > Duration::ZERO {
                        async {
                            debug!("Data generator ahead of schedule, sleeping for {:?}", sleep);
                            tokio::time::sleep(sleep).await
                        }
                        .instrument(info_span!("throttle"))
                        .await;
                    }

                    // send will only error if the matching receiver has been dropped.
                    if let Err(SendError(_)) = generator_tx.send(GeneratorEvent::Data(batch)) {
                        bail!("receiver unexpectedly dropped");
                    }
                    trace!("data generator {} wrote a batch", source_id);

                    let new_source_time =
                        shared_source_time.load(std::sync::atomic::Ordering::SeqCst);
                    if new_source_time > current_source_time {
                        current_source_time = new_source_time;
                        if let Err(SendError(_)) =
                            generator_tx.send(GeneratorEvent::Progress(current_source_time))
                        {
                            bail!("receiver unexpectedly dropped");
                        }
                        trace!("data generator {source_id} downgraded to {current_source_time}");
                    }

                    if elapsed - prev_log > args.logging_granularity {
                        let records_sent = usize::cast_from(batch_idx) * args.batch_size;
                        debug!(
                            "After {} ms data generator {} has sent {} records.",
                            start.elapsed().as_millis(),
                            source_id,
                            records_sent
                        );
                        prev_log = elapsed;
                    }
                }
            }
            .instrument(generator_span),
        );

        generator_handles.push(data_generator_handle);
    }

    let source_rxs = Arc::new(Mutex::new(source_rxs));

    let rocks_dir = tempfile::tempdir().unwrap();
    let dir_path = rocks_dir.path().to_owned();
    info!(
        "RocksDB instances will be hosted at: {}",
        dir_path.display()
    );

    if args.dont_cleanup_rocksdb {
        std::mem::forget(rocks_dir);
    }

    let timely_config = timely::Config::process(num_workers);
    let timely_guards = timely::execute::execute(timely_config, move |timely_worker| {
        let progress_tx = Arc::clone(&progress_tx);
        let source_rxs = Arc::clone(&source_rxs);

        let dir_path = dir_path.clone();

        let probe = timely_worker.dataflow::<u64, _, _>(move |scope| {
            let mut source_streams = Vec::new();

            for source_id in 0..num_sources {
                let source_rxs = Arc::clone(&source_rxs);

                let source_stream = generator_source(scope, source_id, source_rxs);

                let upsert_stream = upsert(
                    scope,
                    &source_stream,
                    source_id,
                    args.use_rocksdb
                        .then(|| IoThreadRocksDB::new(&dir_path, scope.index(), args.use_wal)),
                );

                // Choose a different worker for the counting.
                // TODO(aljoscha): Factor out into function.
                let worker_id = scope.index();
                let worker_count = scope.peers();
                let chosen_worker =
                    usize::cast_from((source_id + 1).hashed() % u64::cast_from(worker_count));
                let active_worker = chosen_worker == worker_id;

                let _output: Stream<_, ()> = upsert_stream.unary_frontier(
                    Exchange::new(move |_| u64::cast_from(chosen_worker)),
                    &format!("source-{source_id}-counter"),
                    move |_caps, _info| {
                        let mut count = 0;
                        let mut buffer = Vec::new();
                        move |input, _output| {
                            input.for_each(|_time, data| {
                                data.swap(&mut buffer);
                                for _record in buffer.drain(..) {
                                    count += 1;
                                }
                            });

                            if input.frontier().is_empty() && active_worker {
                                assert!(count == num_records_total);
                                info!(
                                    "Processing {} finished \
                                    after {} ms and processed {count} records",
                                    source_id,
                                    start.elapsed().as_millis(),
                                );
                            }
                        }
                    },
                );

                source_streams.push(upsert_stream);
            }

            let probe = Handle::default();

            for source_stream in source_streams {
                source_stream.probe_notify_with(vec![probe.clone()]);
            }

            let worker_id = scope.index();

            let active_worker = 0 == worker_id;

            let progress_op =
                AsyncOperatorBuilder::new("progress-bridge".to_string(), scope.clone());

            let probe_clone = probe.clone();
            let _shutdown_button = progress_op.build(move |_capabilities| async move {
                if !active_worker {
                    return;
                }

                let progress_tx = progress_tx
                    .lock()
                    .expect("lock poisoned")
                    .take()
                    .expect("someone took our progress_tx");

                loop {
                    let _progressed = probe_clone.progressed().await;
                    let mut frontier = Antichain::new();
                    probe_clone.with_frontier(|new_frontier| {
                        frontier.clone_from(&new_frontier.to_owned())
                    });
                    if !frontier.is_empty() {
                        let progress_ts = frontier.into_option().unwrap();
                        if let Err(SendError(_)) = progress_tx.send(progress_ts) {
                            return;
                        }
                    } else {
                        // We're done!
                        return;
                    }
                }
            });

            probe
        });

        // Step until our sources shut down.
        while probe.less_than(&u64::MAX) {
            timely_worker.step();
        }
    })
    .unwrap();

    let start_clone = start.clone();
    task::spawn(|| "lag-observer", async move {
        while let Some(observed_time) = progress_rx.recv().await {
            // TODO(aljoscha): The lag here also depends on when the generator task downgrades the
            // time, which it only does _after_ it creates a new batch. We have to change it to
            // immediately downgrade when we change the shared source timestamp.
            // TODO(aljoscha): Make the output similar to the persist open-loop benchmark, where we
            // output throughput, num records, etc.
            let now: u64 = start_clone.elapsed().as_millis().try_into().unwrap();
            let diff = now - observed_time;
            info!("lag: {diff}ms");
        }
        trace!("progress channel closed!");
    });

    while start.elapsed() < args.runtime {
        let current_time = shared_source_time.load(Ordering::SeqCst);
        let new_time: u64 = start.elapsed().as_millis().try_into().unwrap();

        if new_time > current_time + 1000 {
            shared_source_time.store(new_time, Ordering::SeqCst);
        }

        tokio::time::sleep(Duration::from_millis(1000)).await;
    }

    for handle in generator_handles {
        match handle.await? {
            Ok(finished) => info!("{}", finished),
            Err(e) => error!("error: {:?}", e),
        }
    }

    for g in timely_guards.join() {
        g.expect("timely process failed");
    }

    Ok(())
}

enum GeneratorEvent {
    Progress(u64),
    Data(ColumnarRecords),
}

/// A source that reads from it's unbounded channel and returns a `Stream` of the contained records.
///
/// Only one worker is expected to read from the channel that the associated generator is writing
/// to.
fn generator_source<G>(
    scope: &G,
    source_id: usize,
    generator_rxs: Arc<Mutex<BTreeMap<usize, UnboundedReceiver<GeneratorEvent>>>>,
) -> Stream<G, (Vec<u8>, Vec<u8>)>
where
    G: Scope<Timestamp = u64>,
{
    let generator_rxs = Arc::clone(&generator_rxs);

    let scope = scope.clone();
    let worker_id = scope.index();
    let worker_count = scope.peers();

    let chosen_worker = usize::cast_from(source_id.hashed() % u64::cast_from(worker_count));
    let active_worker = chosen_worker == worker_id;

    let mut source_op = AsyncOperatorBuilder::new(format!("source-{source_id}"), scope);

    let (mut output, output_stream) = source_op.new_output();

    let _shutdown_button = source_op.build(move |mut capabilities| async move {
        if !active_worker {
            return;
        }

        let mut cap = capabilities.pop().expect("missing capability");

        let mut generator_rx = {
            let mut generator_rxs = generator_rxs.lock().expect("lock poisoned");
            generator_rxs
                .remove(&source_id)
                .expect("someone else took our channel")
        };

        while let Some(event) = generator_rx.recv().await {
            match event {
                GeneratorEvent::Progress(ts) => {
                    trace!("source {source_id}, progress: {ts}");
                    cap.downgrade(&ts);
                }
                GeneratorEvent::Data(batch) => {
                    // TODO(aljoscha): Work with the diff field.
                    for record in batch
                        .iter()
                        .map(|((key, value), _ts, _diff)| (key.to_vec(), value.to_vec()))
                    {
                        // TODO(aljoscha): Is this the most efficient way of emitting all those
                        // records?
                        output.give(&cap, record).await;
                    }
                }
            }
        }
    });

    output_stream
}

/// A representative upsert operator.
fn upsert<G>(
    scope: &G,
    source_stream: &Stream<G, (Vec<u8>, Vec<u8>)>,
    source_id: usize,
    rocksdb: Option<IoThreadRocksDB>,
) -> Stream<G, (Vec<u8>, Vec<u8>, i32)>
where
    G: Scope<Timestamp = u64>,
{
    if let Some(rocksdb) = rocksdb {
        upsert_core(scope, source_stream, source_id, rocksdb)
    } else {
        upsert_core(scope, source_stream, source_id, BTreeMap::new())
    }
}

fn upsert_core<G, M: Map + 'static>(
    scope: &G,
    source_stream: &Stream<G, (Vec<u8>, Vec<u8>)>,
    source_id: usize,
    mut current_values: M,
) -> Stream<G, (Vec<u8>, Vec<u8>, i32)>
where
    G: Scope<Timestamp = u64>,
{
    let mut upsert_op =
        AsyncOperatorBuilder::new(format!("source-{source_id}-upsert"), scope.clone());

    let mut input = upsert_op.new_input(
        source_stream,
        Exchange::new(|d: &(Vec<u8>, Vec<u8>)| d.0.hashed()),
    );
    let (mut output, source_stream): (_, Stream<_, (Vec<u8>, Vec<u8>, i32)>) =
        upsert_op.new_output();

    upsert_op.build(move |capabilities| async move {
        drop(capabilities);

        // Just use a basic nested-map like the old upsert implementation to buffer values by time.
        let mut pending_values: BTreeMap<u64, (_, BTreeMap<_, _>)> = BTreeMap::new();

        let mut frontier = Antichain::from_elem(0);
        while let Some(event) = input.next_mut().await {
            match event {
                AsyncEvent::Data(cap, buffer) => {
                    for (k, v) in buffer.drain(..) {
                        let time = *cap.time();
                        let map = &mut pending_values
                            .entry(time)
                            .or_insert_with(|| (cap.delayed(cap.time()), BTreeMap::new()))
                            .1;
                        // In real upsert we sort by offset, but here we just
                        // choose the latest one.
                        map.insert(k, v);
                    }
                }
                AsyncEvent::Progress(new_frontier) => frontier = new_frontier,
            }
            let mut removed_times = Vec::new();

            // TODO(guswynn): All this code is pretty gross, but I couldn't figure out a better
            // way to share `&mut Capability`s and also provide the `Map` implementations the
            // largest batches possible.
            let mut batches = Vec::new();
            let mut caps = Vec::new();
            for (time, (cap, map)) in pending_values.iter_mut() {
                if frontier.less_equal(time) {
                    break;
                }
                let len = map.len();
                batches.push(std::mem::take(map).into_iter().collect());
                caps.push((cap, len));

                removed_times.push(*time)
            }

            // `caps` holds references to `Capability`s, and also the number of values
            // in the mega-batches they correspond too.
            let mut cap_iter = caps.into_iter();
            let mut cur_cap = None;
            for (k, v, previous_v) in current_values.ingest(batches).await {
                let cap = match &mut cur_cap {
                    None => {
                        let stuff = cur_cap.insert(cap_iter.next().unwrap());
                        stuff.1 -= 1;
                        &mut stuff.0
                    }
                    Some((_cap, num)) if *num == 0 => {
                        &mut cur_cap.insert(cap_iter.next().unwrap()).0
                    }
                    Some((cap, num)) => {
                        *num -= 1;
                        cap
                    }
                };
                if let Some(previous_v) = previous_v {
                    // we might be able to avoid this extra key clone here,
                    // if we really tried
                    output.give(*cap, (k.clone(), previous_v, -1)).await;
                }
                // we don't do deletes right now
                output.give(*cap, (k, v, 1)).await;
            }

            // Discard entries, capabilities for complete times.
            for time in removed_times {
                pending_values.remove(&time);
            }
        }
    });

    source_stream
}

type KV = (Vec<u8>, Vec<u8>);
type KVAndPrevious = (Vec<u8>, Vec<u8>, Option<Vec<u8>>);
type Batch<T = KV> = Vec<T>;

/// A "map" we can ingest data into.
#[async_trait::async_trait]
trait Map {
    /// Ingest the batches of `KV`s. Return the `KV`s back in a single
    /// allocation, along with that key's previous value, if it existed.
    ///
    /// The input and output use different nesting schemes
    /// (nested batches vs flat) to reduce additional allocations in
    /// the `upsert` operator.
    async fn ingest(&mut self, batches: Vec<Batch>) -> Batch<KVAndPrevious>;
}

#[async_trait::async_trait]
impl Map for BTreeMap<Vec<u8>, Vec<u8>> {
    async fn ingest(&mut self, batches: Vec<Batch>) -> Batch<KVAndPrevious> {
        let mut out = Vec::new();
        for batch in batches {
            for (k, v) in batch {
                // TODO(guswynn): reduce clones.
                let in_k = k.clone();
                let in_v = v.clone();
                out.push((k, v, self.insert(in_k, in_v)))
            }
        }
        out
    }
}

use rocksdb::{Error, DB};
use std::path::Path;
use tokio::sync::oneshot::{channel, Sender};

#[derive(Clone)]
struct IoThreadRocksDB {
    tx: crossbeam_channel::Sender<(Vec<Batch>, Sender<Result<Batch<KVAndPrevious>, Error>>)>,
}

impl IoThreadRocksDB {
    fn new(temp_dir: &Path, index: usize, use_wal: bool) -> Self {
        // bounded??
        let (tx, rx): (
            _,
            crossbeam_channel::Receiver<(Vec<Batch>, Sender<Result<Batch<KVAndPrevious>, Error>>)>,
        ) = crossbeam_channel::unbounded();
        let db: DB = DB::open_default(temp_dir.join(index.to_string())).unwrap();
        std::thread::spawn(move || {
            let mut wo = rocksdb::WriteOptions::new();
            wo.disable_wal(!use_wal);

            'batch: while let Ok((batches, resp)) = rx.recv() {
                let size: usize = batches.iter().map(|b| b.len()).sum();

                // TODO(guswynn): this should probably be lifted into the upsert operator.
                if size == 0 {
                    let _ = resp.send(Ok(Vec::new()));
                    continue;
                }

                let gets = db.multi_get(
                    batches
                        .iter()
                        .flat_map(|b| b.iter().map(|(k, _)| k.as_slice())),
                );

                let mut previous = Vec::new();
                let mut writes = rocksdb::WriteBatch::default();

                // TODO(guswynn): sort by key before writing.
                for ((k, v), get) in batches.into_iter().flat_map(|b| b.into_iter()).zip(gets) {
                    writes.put(k.as_slice(), v.as_slice());

                    match get {
                        Ok(prev) => {
                            previous.push((k, v, prev));
                        }
                        Err(e) => {
                            // Give up on the batch on errors.
                            let _ = resp.send(Err(e));
                            continue 'batch;
                        }
                    }
                }
                match db.write_opt(writes, &wo) {
                    Ok(()) => {}
                    Err(e) => {
                        // Give up on the batch on errors.
                        let _ = resp.send(Err(e));
                        continue 'batch;
                    }
                }
                info!("finished writing batch size({size}) for worker {index}");

                let _ = resp.send(Ok(previous));
            }
        });

        Self { tx }
    }

    async fn ingest_inner(&mut self, batches: Vec<Batch>) -> Batch<KVAndPrevious> {
        let (tx, rx) = channel();

        // We assume the rocksdb thread doesnt shutdown before timely
        self.tx.send((batches, tx)).unwrap();

        // We also unwrap all rocksdb errors here.
        rx.await.unwrap().unwrap()
    }
}

#[async_trait::async_trait]
impl Map for IoThreadRocksDB {
    async fn ingest(&mut self, batches: Vec<Batch>) -> Batch<KVAndPrevious> {
        self.ingest_inner(batches).await
    }
}
