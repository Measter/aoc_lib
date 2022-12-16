use std::{
    fmt::Display,
    hint::black_box,
    io::{BufWriter, Read, Seek, SeekFrom, Write},
    panic::catch_unwind,
    time::{Duration, Instant},
};

use crossbeam_channel::Sender;
use thiserror::Error;

use crate::{input, BenchError, BenchResult, TracingAlloc};

pub mod detailed;
pub mod simple;

pub type SetupFunction = for<'a> fn(&'a str, Bench) -> BenchResult;

const NANOS_PER_SECOND: f64 = 1_000_000_000.0;
const NANOS_PER_SECOND_INT: u128 = 1_000_000_000;
const MAX_SAMPLES: usize = 1_000_000;

#[derive(Debug, Error)]
#[error("Error benching memory use: {:?}", .inner)]
pub struct MemoryBenchError {
    #[source]
    #[from]
    pub inner: std::io::Error,
}

#[derive(Default)]
pub(crate) struct RuntimeData {
    pub(crate) sample_count: usize,
    pub(crate) mean: Duration,
    pub(crate) std_dev: Duration,
    pub(crate) first_quartile: Duration,
    pub(crate) third_quartile: Duration,
    pub(crate) outlier_count: usize,
}

#[derive(Default)]
pub(crate) struct MemoryData {
    pub(crate) end_ts: f32,
    pub(crate) end_ts_duration: Duration,
    pub(crate) graph_points: Vec<(f32, f32)>,
    pub(crate) max_memory: usize,
    pub(crate) num_allocs: usize,
}

fn read_memory_data(trace_input: &str) -> MemoryData {
    let mut points = Vec::new();
    let mut cur_bytes = 0;
    let mut prev_bytes = 0;
    let mut end_ts_duration = Duration::ZERO;
    let mut end_ts = 0.0;
    let mut max_bytes = 0;
    let mut num_allocs = 0;

    for line in trace_input.lines() {
        let mut parts = line.split_whitespace().map(str::trim);

        let (size, ts): (isize, u128) = match (
            parts.next(),
            parts.next().map(str::parse),
            parts.next().map(str::parse),
        ) {
            (Some("A"), Some(Ok(ts)), Some(Ok(size))) => {
                num_allocs += 1;
                (size, ts)
            }
            (Some("F"), Some(Ok(ts)), Some(Ok(size))) => (-size, ts),
            (Some("S"), Some(Ok(ts)), _) => (0, ts),
            (Some("E"), Some(Ok(ts)), _) => {
                let (secs, nanos) = (ts / NANOS_PER_SECOND_INT, ts % NANOS_PER_SECOND_INT);
                end_ts_duration = Duration::new(secs as _, nanos as _);
                end_ts = ts as f32;
                (0, ts)
            }
            _ => {
                continue;
            }
        };

        cur_bytes += size;
        max_bytes = max_bytes.max(cur_bytes);

        points.push((ts as f32, prev_bytes as f32));
        points.push((ts as f32, cur_bytes as f32));

        prev_bytes = cur_bytes;
    }

    MemoryData {
        end_ts,
        end_ts_duration,
        graph_points: points,
        max_memory: max_bytes as usize,
        num_allocs,
    }
}

// Not that this function expects the samples to be *SORTED* before being passed in.
fn generate_runtime_stats(samples: &[Duration]) -> RuntimeData {
    // I don't see any runtime going beyond 10 seconds, which would only result
    // in 10,000,000,000 ^ 2, or 10^20. A u128 has 10^38 precision.

    let (mean_sum, std_dev_sum) = samples.iter().fold(
        (Duration::ZERO, 0u128),
        |(mean_sum, std_dev_sum), &sample| {
            (mean_sum + sample, std_dev_sum + sample.as_nanos().pow(2))
        },
    );

    let mean = mean_sum / samples.len() as u32;
    let total_std_dev = (std_dev_sum / samples.len() as u128) - mean.as_nanos().pow(2);
    let total_std_dev = (total_std_dev as f64).sqrt() / NANOS_PER_SECOND;
    let std_dev = Duration::from_secs_f64(total_std_dev);

    let first_quartile = samples[samples.len() / 4];
    let third_quartile = samples[samples.len() * 3 / 4];

    RuntimeData {
        mean,
        std_dev,
        sample_count: samples.len(),
        outlier_count: 0,
        first_quartile,
        third_quartile,
    }
}

fn bench_function_runtime<Output, OutputErr>(
    bench_time: u64,
    func: impl Fn() -> Result<Output, OutputErr>,
) -> RuntimeData {
    let bench_start = Instant::now();
    let mut samples = Vec::with_capacity(MAX_SAMPLES);

    loop {
        let start = Instant::now();
        let res = func();
        let elapsed = start.elapsed();
        samples.push(elapsed);

        // Don't drop while measuring, in case the user returns a non-trivial type.
        // Also don't handle errors, as the function is assumed to be pure, and has already
        // had its return value checked in our caller.
        drop(black_box(res));

        if (bench_start.elapsed().as_secs() >= bench_time && samples.len() >= 10)
            || samples.len() > MAX_SAMPLES
        {
            break;
        }
    }

    samples.sort_unstable();
    let unfiltered_stats = generate_runtime_stats(&samples);

    // The raw samples have some pretty extreme outliers. We'll filter out those more than 2 standard
    // deviations from the unfiltered mean and recalculate the mean and std. dev.
    samples.retain(|&sample| {
        let (smaller, larger) = (
            sample.min(unfiltered_stats.mean),
            sample.max(unfiltered_stats.mean),
        );
        (larger - smaller) <= unfiltered_stats.std_dev * 2
    });

    let mut filtered_stats = generate_runtime_stats(&samples);
    filtered_stats.outlier_count = unfiltered_stats.sample_count - filtered_stats.sample_count;

    filtered_stats
}

fn bench_function_memory<Output, OutputErr>(
    alloc: &TracingAlloc,
    func: impl Fn() -> Result<Output, OutputErr>,
) -> Result<MemoryData, MemoryBenchError> {
    let trace_file = tempfile::tempfile()?;

    let writer = BufWriter::new(trace_file);
    alloc.set_file(writer);

    // No need to handle an error here, we did it earlier.
    alloc.enable_tracing();
    // Don't discard here, or dropping the return value will be caught
    // by the tracer.
    let res = func();
    alloc.disable_tracing();
    let _ = res;

    let mut mem_trace = String::new();

    let mut trace_writer = alloc.clear_file().unwrap(); // Should get it back.
    trace_writer.flush()?;

    let mut trace_file = trace_writer.into_inner().unwrap();
    trace_file.seek(SeekFrom::Start(0))?;
    trace_file.read_to_string(&mut mem_trace)?;

    Ok(read_memory_data(&mem_trace))
}

pub(crate) enum BenchEvent {
    Answer {
        answer: String,
        id: usize,
        is_alt: bool,
    },
    Memory {
        data: MemoryData,
        id: usize,
    },
    Timing {
        data: RuntimeData,
        id: usize,
    },
    Error {
        err: String,
        id: usize,
    },
    Finish {
        id: usize,
    },
}

pub(crate) struct AlternateAnswer {
    pub(crate) answer: String,
    pub(crate) day: u8,
    pub(crate) day_function_id: u8,
}

pub struct Bench {
    pub(crate) alloc: &'static TracingAlloc,
    pub(crate) id: usize,
    pub(crate) chan: Sender<BenchEvent>,
    pub(crate) run_only: bool,
    pub(crate) bench_time: u64,
}

impl Bench {
    pub fn bench_alt<T, E>(self, f: impl Fn() -> Result<T, E> + Copy) -> Result<(), BenchError>
    where
        T: Display,
        E: Display,
    {
        self.bench_inner(true, f)
    }
    pub fn bench<T, E>(self, f: impl Fn() -> Result<T, E> + Copy) -> Result<(), BenchError>
    where
        T: Display,
        E: Display,
    {
        self.bench_inner(false, f)
    }

    fn bench_inner<T, E>(
        self,
        is_alt: bool,
        f: impl Fn() -> Result<T, E> + Copy,
    ) -> Result<(), BenchError>
    where
        T: Display,
        E: Display,
    {
        let answer = f()
            .map_err(|e| {
                self.chan.send(BenchEvent::Error {
                    err: e.to_string(),
                    id: self.id,
                })
            })
            .map_err(|_| BenchError::ChannelError(self.id))?;

        self.chan
            .send(BenchEvent::Answer {
                answer: answer.to_string(),
                id: self.id,
                is_alt,
            })
            .map_err(|_| BenchError::ChannelError(self.id))?;

        if !self.run_only {
            let data = bench_function_memory(self.alloc, f)
                .map_err(|e| BenchError::MemoryBenchError(e, self.id))?;

            self.chan
                .send(BenchEvent::Memory { data, id: self.id })
                .map_err(|_| BenchError::ChannelError(self.id))?;

            let data = bench_function_runtime(self.bench_time, f);
            self.chan
                .send(BenchEvent::Timing { data, id: self.id })
                .map_err(|_| BenchError::ChannelError(self.id))?;
        }

        Ok(())
    }
}

pub(crate) fn bench_worker(day: u8, bench: Bench, func: SetupFunction) {
    let id = bench.id;
    let sender = bench.chan.clone();
    match input(day).open() {
        Ok(input) => {
            let did_panic = catch_unwind(|| func(&input, bench));

            match did_panic {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => {
                    sender
                        .send(BenchEvent::Error {
                            err: e.to_string(),
                            id,
                        })
                        .expect("Unable to send error");
                }
                Err(payload) => {
                    let msg = payload
                        .downcast_ref::<&str>()
                        .copied()
                        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
                        .unwrap_or("Unknown reason");

                    sender
                        .send(BenchEvent::Error {
                            err: format!("Panic: {}", msg),
                            id,
                        })
                        .expect("Unable to send error");
                }
            }
        }
        Err(BenchError::InputFileError { inner, name }) => {
            sender
                .send(BenchEvent::Error {
                    err: format!("{}: {:?}", name, inner.kind()),
                    id,
                })
                .expect("Unable to send error");
        }
        Err(_) => unreachable!(), // InputFile::open only returns one error variant.
    }

    sender
        .send(BenchEvent::Finish { id })
        .expect("Unable to send finish");
}
