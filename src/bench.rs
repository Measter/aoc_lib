use std::{
    fmt::Display,
    io::{BufWriter, Read, Seek, SeekFrom, Write},
    sync::mpsc::Sender,
    time::{Duration, Instant},
};

use once_cell::sync::Lazy;
use thiserror::Error;

use crate::{alloc::TracingAlloc, Args, BenchError};

#[derive(Debug, Error)]
#[error("Error benching memory use: {:?}", .inner)]
pub struct MemoryBenchError {
    #[source]
    #[from]
    pub inner: std::io::Error,
}

#[derive(Default)]
pub(crate) struct RuntimeData {
    total_runs: u32,
    min_run: Duration,
    mean_run: Duration,
    max_run: Duration,
}

#[derive(Default)]
pub(crate) struct MemoryData {
    end_ts: u128,
    graph_points: Vec<(f64, f64)>,
    max_memory: usize,
}

fn get_data(trace_input: &str) -> MemoryData {
    let mut points = Vec::new();
    let mut cur_bytes = 0;
    let mut prev_bytes = 0;
    let mut end_ts = 0;
    let mut max_bytes = 0;

    for line in trace_input.lines() {
        let mut parts = line.split_whitespace().map(str::trim);

        let (size, ts): (isize, u128) = match (
            parts.next(),
            parts.next().map(str::parse),
            parts.next().map(str::parse),
        ) {
            (Some("A"), Some(Ok(ts)), Some(Ok(size))) => (size, ts),
            (Some("F"), Some(Ok(ts)), Some(Ok(size))) => (-size, ts),
            (Some("S"), Some(Ok(ts)), _) => (0, ts),
            (Some("E"), Some(Ok(ts)), _) => {
                end_ts = ts;
                (0, ts)
            }
            _ => {
                continue;
            }
        };

        cur_bytes += size;
        max_bytes = max_bytes.max(cur_bytes);

        points.push((ts as f64, prev_bytes as f64));
        points.push((ts as f64, cur_bytes as f64));

        prev_bytes = cur_bytes;
    }

    MemoryData {
        end_ts,
        graph_points: points,
        max_memory: max_bytes as usize,
    }
}

fn get_precision(val: Duration) -> usize {
    if val.as_nanos() < 1000 {
        0
    } else {
        3
    }
}

fn bench_function_runtime<Output, OutputErr>(
    args: &Args,
    func: impl Fn() -> Result<Output, OutputErr>,
) -> RuntimeData {
    // Run a few times to get an estimate of how long it takes.
    let mut min_run = Duration::from_secs(u64::MAX);

    for _ in 0..5 {
        let now = Instant::now();
        let _ = func();
        let time = now.elapsed();

        if time < min_run {
            min_run = time;
        }
    }

    let total_runs = (args.bench_time as f64 / min_run.as_secs_f64())
        .ceil()
        .max(10.0)
        .min(10e6) as u32;

    let mut total_time = Duration::default();
    let mut min_run = Duration::from_secs(u64::MAX);
    let mut max_run = Duration::default();

    for _ in 0..total_runs {
        let start = Instant::now();
        let _ = func(); // We'll just discard the result as we handled errors above.
        let elapsed = start.elapsed();

        total_time += start.elapsed();
        if elapsed < min_run {
            min_run = elapsed;
        }

        if elapsed > max_run {
            max_run = elapsed;
        }
    }

    let mean_run = total_time / total_runs;

    RuntimeData {
        total_runs,
        min_run,
        mean_run,
        max_run,
    }
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

    Ok(get_data(&mem_trace))
}

pub(crate) enum BenchEvent {
    Answer {
        answer: String,
        day: usize,
        part: usize,
    },
    Memory {
        data: MemoryData,
        day: usize,
        part: usize,
    },
    Timing {
        data: RuntimeData,
        day: usize,
        part: usize,
    },
    Error {
        err: String,
        day: usize,
        part: usize,
    },
}

pub struct Bench {
    alloc: &'static TracingAlloc,
    day_id: usize,
    part_id: usize,
    chan: Sender<BenchEvent>,
    args: &'static Lazy<Args>,
}

impl Bench {
    pub fn bench<T: Display, E: Display>(
        self,
        f: impl Fn() -> Result<T, E>,
    ) -> Result<(), BenchError> {
        match f() {
            Ok(t) => self
                .chan
                .send(BenchEvent::Answer {
                    answer: t.to_string(),
                    day: self.day_id,
                    part: self.part_id,
                })
                .map_err(|_| BenchError::ChannelError(self.day_id, self.part_id))?,
            Err(e) => {
                self.chan
                    .send(BenchEvent::Error {
                        err: e.to_string(),
                        day: self.day_id,
                        part: self.part_id,
                    })
                    .map_err(|_| BenchError::ChannelError(self.day_id, self.part_id))?;
                return Ok(());
            }
        }

        if !self.args.no_bench {
            if !self.args.no_mem {
                let data = bench_function_memory(self.alloc, &f)
                    .map_err(|e| BenchError::MemoryBenchError(e, self.day_id, self.part_id))?;

                self.chan
                    .send(BenchEvent::Memory {
                        data,
                        day: self.day_id,
                        part: self.part_id,
                    })
                    .map_err(|_| BenchError::ChannelError(self.day_id, self.part_id))?;
            }

            let data = bench_function_runtime(self.args, &f);
            self.chan
                .send(BenchEvent::Timing {
                    data,
                    day: self.day_id,
                    part: self.part_id,
                })
                .map_err(|_| BenchError::ChannelError(self.day_id, self.part_id))?;
        }

        Ok(())
    }
}
