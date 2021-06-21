use bench::{BenchEvent, MemoryData, RuntimeData};
use bytesize::ByteSize;
use console::Term;
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
use once_cell::sync::Lazy;
use rayon::{ThreadPool, ThreadPoolBuilder};
use structopt::StructOpt;
use thiserror::Error;

mod alloc;
mod bench;
pub mod misc;
pub use bench::{Bench, MemoryBenchError};
pub mod parsers;
pub use alloc::TracingAlloc;

use std::{fmt::Display, iter::once, sync::mpsc::channel, thread, time::Duration};

static ARGS: Lazy<Args> = Lazy::new(Args::from_args);

pub type BenchResult = Result<(), BenchError>;
type Function = for<'a> fn(&'a str, Bench) -> BenchResult;

#[derive(Debug, Error)]
pub enum BenchError {
    #[error("Error performing memory benchmark function {}: {}", .1, .0)]
    MemoryBenchError(#[source] MemoryBenchError, usize),

    #[error("Error returning benchmark result for function {}", .0)]
    ChannelError(usize),

    #[error("Error opening input file '{}': {:}", .name, .inner)]
    InputFileError {
        #[source]
        inner: std::io::Error,
        name: String,
    },

    #[error("{}", .0)]
    UserError(#[source] Box<dyn std::error::Error + Send + Sync>),
}

#[derive(Copy, Clone, StructOpt)]
pub(crate) enum OutputType {
    /// Print a table of timings with a memory use graph (default)
    Table,
    #[structopt(name = "markdown")]
    /// Print a markdown table
    MarkDown,
}

#[derive(StructOpt)]
pub(crate) struct Args {
    #[structopt(long)]
    /// Skip all benchmarking
    no_bench: bool,

    #[structopt(long)]
    /// Skip memory benchmarking
    no_mem: bool,

    #[structopt(long, default_value = "3")]
    /// Benchmarking period in seconds to measure run time of parts
    bench_time: u32,

    // #[structopt(subcommand)]
    // /// The layout of the output
    // output: Option<OutputType>,
    #[structopt(long = "threads")]
    /// How many worker threads to spawn for benchmarking [default: cores - 2, min: 1]
    num_threads: Option<usize>,
}

pub struct ProblemInput;
impl Display for ProblemInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("")
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Example {
    Parse,
    Part1,
    Part2,
    Other(&'static str),
}

impl Display for Example {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let output = match self {
            Example::Parse => "parse",
            Example::Part1 => "part1",
            Example::Part2 => "part2",
            Example::Other(s) => s,
        };

        f.write_str(output)
    }
}

pub struct InputFile<T> {
    year: u16,
    day: u8,
    example_id: Option<(Example, T)>,
}

impl InputFile<ProblemInput> {
    pub fn example<T: Display>(self, part: Example, id: T) -> InputFile<T> {
        InputFile {
            year: self.year,
            day: self.day,
            example_id: Some((part, id)),
        }
    }
}

impl<T: Display> InputFile<T> {
    pub fn open(self) -> Result<String, BenchError> {
        let path = if let Some((part, id)) = self.example_id {
            format!(
                "./example_inputs/aoc_{:02}{:02}_{}-{}.txt",
                self.year % 100,
                self.day,
                part,
                id
            )
        } else {
            format!("./inputs/aoc_{:02}{:02}.txt", self.year % 100, self.day)
        };

        std::fs::read_to_string(&path).map_err(|e| BenchError::InputFileError {
            inner: e,
            name: path,
        })
    }
}

pub fn input(year: u16, day: u8) -> InputFile<ProblemInput> {
    InputFile {
        year,
        day,
        example_id: None,
    }
}

#[derive(Copy, Clone)]
pub struct Day {
    pub name: &'static str,
    pub day: u8,
    pub part_1: Function,
    pub part_2: Option<Function>,
}

struct BenchedFunction {
    // name: &'static str,
    day: u8,
    day_function_id: u8,
    function: Function,
    answer: Option<String>,
    error: Option<String>,
    timing_data: Option<RuntimeData>,
    memory_data: Option<MemoryData>,
    finished_spinner: ProgressStyle,
    error_spinner: ProgressStyle,
    bar: Option<ProgressBar>,
}

fn get_precision(val: Duration) -> usize {
    if val.as_nanos() < 1000 {
        0
    } else {
        3
    }
}

impl BenchedFunction {
    fn answer(&mut self, ans: String) {
        self.answer = Some(ans);
        if let Some(bar) = &self.bar {
            bar.set_style(self.finished_spinner.clone());
            let msg = self.render();
            bar.set_message(msg);
        }
    }

    fn memory(&mut self, data: MemoryData) {
        self.memory_data = Some(data);
        if let Some(bar) = &self.bar {
            let msg = self.render();
            bar.set_message(msg);
        }
    }

    fn timing(&mut self, data: RuntimeData) {
        self.timing_data = Some(data);
        if let Some(bar) = &self.bar {
            let msg = self.render();
            bar.set_message(msg);
        }
    }

    fn error(&mut self, err: String) {
        self.error = Some(err);
        if let Some(bar) = &self.bar {
            bar.set_style(self.error_spinner.clone());
            let msg = self.render();
            bar.set_message(msg);
        }
    }

    fn finish(&mut self) {
        if let Some(bar) = &self.bar {
            bar.finish()
        }
    }

    fn render(&self) -> String {
        if let Some(err) = self.error.as_deref() {
            err.to_string()
        } else {
            let ans = self.answer.as_deref().unwrap_or("");
            let time = self
                .timing_data
                .as_ref()
                .map(|td| {
                    let prec = get_precision(td.mean_run);
                    format!("{:.prec$?}", td.mean_run, prec = prec)
                })
                .unwrap_or_else(String::new);
            let mem = self
                .memory_data
                .as_ref()
                .map(|md| format!("{}", ByteSize(md.max_memory as u64)))
                .unwrap_or_else(String::new);

            format!("{:<30} | {:<10} | {}", ans, time, mem)
        }
    }
}

fn bench_days_chunk(
    alloc: &'static TracingAlloc,
    year: u16,
    mut funcs: Vec<BenchedFunction>,
    spinner_style: &ProgressStyle,
    pool: &ThreadPool,
) -> Result<Duration, BenchError> {
    let (sender, receiver) = channel::<BenchEvent>();
    let multi_bars = MultiProgress::new();
    multi_bars.set_draw_target(ProgressDrawTarget::stdout());

    let mut bars = Vec::new();

    for (id, func) in funcs.iter_mut().enumerate() {
        let bar = multi_bars.add(ProgressBar::new(10000));
        bar.set_prefix(format!("{:2}.{}", func.day, func.day_function_id));
        bar.set_style(spinner_style.clone());

        bars.push(bar.clone());
        func.bar = Some(bar);

        let bench = Bench {
            alloc,
            id,
            chan: sender.clone(),
            args: &ARGS,
        };
        let sender = sender.clone();
        let input = input(year, func.day).open()?;
        let f = func.function;

        pool.spawn(move || {
            if let Err(e) = f(&input, bench) {
                sender
                    .send(BenchEvent::Error {
                        err: e.to_string(),
                        id,
                    })
                    .expect("Unable to send error");

                sender
                    .send(BenchEvent::Finish { id })
                    .expect("Unable to send finish");
            }
        });
    }

    // Using the built-in steady tick spawns a thread for each bar. We could have up to 50.
    // Seems wasteful. Let's just spawn a single thread to tick them all instead.
    let tick_thread = thread::spawn(move || loop {
        let all_finished = bars.iter().fold(true, |all_finished, bar| {
            bar.tick();
            all_finished & bar.is_finished()
        });

        if all_finished {
            break;
        }

        thread::sleep(Duration::from_millis(250));
    });

    // If we don't drop this thread's sender the handler thread will never stop.
    drop(sender);

    // We don't want to spawn the handler thread in the worker pool, because the benchmarking will
    // hog the pool's threads, meaning the UI updates won't happen in a timely manner.
    let (time_sender, time_receiver) = channel::<Duration>();
    let handler_thread = thread::spawn(move || {
        for event in receiver.iter() {
            match event {
                BenchEvent::Answer { answer, id } => funcs[id].answer(answer),
                BenchEvent::Memory { data, id } => funcs[id].memory(data),
                BenchEvent::Timing { data, id } => {
                    time_sender
                        .send(data.mean_run)
                        .expect("Failed to send timing from handler thread");
                    funcs[id].timing(data);
                }
                BenchEvent::Error { err, id } => funcs[id].error(err),
                BenchEvent::Finish { id } => funcs[id].finish(),
            }
        }
    });

    multi_bars.join().expect("Failed to join progress bars");
    handler_thread
        .join()
        .expect("Failed to join handler thread");
    tick_thread.join().expect("Failed to join tick thread");

    Ok(time_receiver.iter().sum())
}

pub fn run(alloc: &'static TracingAlloc, year: u16, days: &[Day]) -> Result<(), BenchError> {
    // We should limit the number of threads in the pool. Having too many
    // results in them basically fighting for priority with the two update threads
    // negatively effecting the benchmark.
    let num_threads = ARGS
        .num_threads
        .unwrap_or_else(|| num_cpus::get_physical().saturating_sub(2))
        .max(1);

    let pool = ThreadPoolBuilder::new()
        .num_threads(num_threads)
        .build()
        .expect("Failed to build threadpool");

    println!("Advent of Code {}", year);
    println!("   Day | {:<30} | {:<10} | Max Mem.", "Answer", "Time");
    println!("_______|_{0:_<30}_|_{0:_<10}_|______________", "");

    let spinner_style = ProgressStyle::default_spinner()
        .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ ")
        .template("{spinner} {prefix:.dim} | {msg}");
    let finished_spinner = spinner_style
        .clone()
        .template("{spinner} {prefix:.green} | {msg}");
    let error_spinner = spinner_style
        .clone()
        .template("{spinner} {prefix:.red} | {msg}");

    // MultiProgress goes a bit nuts if the terminal isn't tall enough to display all the bars
    // at once. So we need to chunk the functions to bench based on how tall the terminal is.
    let stdout = Term::stdout();
    let (rows, _) = stdout.size();
    // Add room for header and trailing line.
    let rows = rows.saturating_sub(5) as usize;

    let mut benched_functions = Vec::new();
    let mut cur_chunk = Vec::new();

    for day in days {
        let parts = once(day.part_1).chain(day.part_2).zip(1..);

        for (f, i) in parts {
            if cur_chunk.len() == rows {
                benched_functions.push(cur_chunk);
                cur_chunk = Vec::new();
            }

            let p1f = BenchedFunction {
                day: day.day,
                // name: day.name,
                day_function_id: i,
                function: f,
                answer: None,
                error: None,
                timing_data: None,
                memory_data: None,
                finished_spinner: finished_spinner.clone(),
                error_spinner: error_spinner.clone(),
                bar: None,
            };

            cur_chunk.push(p1f);
        }
    }
    if !cur_chunk.is_empty() {
        benched_functions.push(cur_chunk);
    }

    let total_time = benched_functions
        .into_iter()
        .map(|days_chunk| bench_days_chunk(alloc, year, days_chunk, &spinner_style, &pool))
        .try_fold(Duration::ZERO, |acc, a| a.map(|a| a + acc))?;

    let prec = get_precision(total_time);
    println!("_______|_{0:_<30}_|_{0:_<10}_|______________", "");
    println!(
        " Total Time: {:26} | {:.prec$?}",
        "",
        total_time,
        prec = prec
    );

    Ok(())
}

#[macro_export]
macro_rules! day {
    (day $id:literal: $name:literal
        1: $p1:ident
    ) => {
        pub static DAY: $crate::Day = $crate::Day {
            name: $name,
            day: $id,
            part_1: $p1,
            part_2: None,
        };
    };
    (day $id:literal: $name:literal
        1: $p1:ident
        2: $p2:ident
    ) => {
        pub static DAY: $crate::Day = $crate::Day {
            name: $name,
            day: $id,
            part_1: $p1,
            part_2: Some($p2),
        };
    };
}
