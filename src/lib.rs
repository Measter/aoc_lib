use bench::{BenchEvent, MemoryData, RuntimeData};
use bytesize::ByteSize;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use once_cell::sync::Lazy;
use rayon::ThreadPoolBuilder;
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

pub type BenchResult = Result<(), BenchError>;

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
    pub part_1: for<'a> fn(&'a str, Bench) -> Result<(), BenchError>,
    pub part_2: Option<for<'a> fn(&'a str, Bench) -> Result<(), BenchError>>,
}

struct BenchedFunction {
    // id: usize,
    // name: &'static str,
    // day: usize,
    answer: Option<String>,
    error: Option<String>,
    timing_data: Option<RuntimeData>,
    memory_data: Option<MemoryData>,
    finished_spinner: ProgressStyle,
    error_spinner: ProgressStyle,
    bar: ProgressBar,
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
        self.bar.set_style(self.finished_spinner.clone());
        let msg = self.render();
        self.bar.set_message(msg);
    }

    fn memory(&mut self, data: MemoryData) {
        self.memory_data = Some(data);
        let msg = self.render();
        self.bar.set_message(msg);
    }

    fn timing(&mut self, data: RuntimeData) {
        self.timing_data = Some(data);
        let msg = self.render();
        self.bar.set_message(msg);
    }

    fn error(&mut self, err: String) {
        self.error = Some(err);
        self.bar.set_style(self.error_spinner.clone());
        let msg = self.render();
        self.bar.set_message(msg);
    }

    fn finish(&mut self) {
        self.bar.finish()
    }

    fn render(&mut self) -> String {
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

pub fn run(alloc: &'static TracingAlloc, year: u16, days: &[Day]) -> Result<(), BenchError> {
    // We should limit the number of threads in the pool. Having too many
    // results in them basically fighting for priority with the two update threads
    // negatively effecting the benchmark.

    let num_threads = num_cpus::get().saturating_sub(2).max(1);

    let pool = ThreadPoolBuilder::new()
        .num_threads(num_threads)
        .build()
        .expect("Failed to build threadpool");

    let (sender, receiver) = channel::<BenchEvent>();

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

    let bars = MultiProgress::new();

    let mut benched_functions = Vec::new();
    let mut id = 0;

    for day in days {
        let parts = once(day.part_1).chain(day.part_2).zip(1..);

        for (f, i) in parts {
            let bar = bars.add(ProgressBar::new(10000));
            bar.set_prefix(format!("{:2}.{}", day.day, i));
            bar.set_style(spinner_style.clone());
            bar.enable_steady_tick(250);

            let p1f = BenchedFunction {
                // id,
                // day: day.day,
                // name: day.name,
                answer: None,
                error: None,
                timing_data: None,
                memory_data: None,
                finished_spinner: finished_spinner.clone(),
                error_spinner: error_spinner.clone(),
                bar,
            };

            benched_functions.push(p1f);

            let bench = Bench {
                alloc,
                id,
                chan: sender.clone(),
                args: &ARGS,
            };
            let sender = sender.clone();
            let input = input(year, day.day).open()?;

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

            id += 1;
        }
    }

    // If we don't drop this thread's sender the handler thread will never stop.
    drop(sender);

    let (time_sender, time_receiver) = channel::<Duration>();
    let handler_thread = thread::spawn(move || {
        for event in receiver.iter() {
            match event {
                BenchEvent::Answer { answer, id } => benched_functions[id].answer(answer),
                BenchEvent::Memory { data, id } => benched_functions[id].memory(data),
                BenchEvent::Timing { data, id } => {
                    time_sender
                        .send(data.mean_run)
                        .expect("Failed to send timing from handler thread");
                    benched_functions[id].timing(data);
                }
                BenchEvent::Error { err, id } => benched_functions[id].error(err),
                BenchEvent::Finish { id } => benched_functions[id].finish(),
            }
        }
    });

    bars.join().expect("Failed to join progress bars");
    handler_thread
        .join()
        .expect("Failed to join handler thread");

    let total_time = time_receiver.iter().sum();
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
