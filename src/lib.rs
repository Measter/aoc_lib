use std::{fmt::Display, iter, sync::mpsc::channel};

use console::style;
use once_cell::sync::Lazy;
use structopt::StructOpt;
use thiserror::Error;

mod alloc;
mod bench;
pub mod misc;
pub mod parsers;

pub use alloc::TracingAlloc;
pub use bench::Bench;
use bench::{Function, MemoryBenchError};

use crate::bench::{simple::run_simple_bench, BenchEvent};

static ARGS: Lazy<Args> = Lazy::new(Args::from_args);

pub type BenchResult = Result<(), BenchError>;

#[derive(Debug, Error)]
pub enum BenchError {
    #[error("Error performing memory benchmark function {}: {}", .1, .0)]
    MemoryBenchError(MemoryBenchError, usize),

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

#[derive(Copy, Clone, StructOpt, PartialEq, Eq)]
pub(crate) enum RunType {
    /// Just runs the day's primary functions.
    Run,
    /// Benchmarks the days' primary functions, and lists them in a simple format.
    Simple,
    /// Benchmarks all the days' functions, and provides a more detailed listing.
    Detailed,
}

#[derive(StructOpt)]
pub(crate) struct Args {
    #[structopt(subcommand)]
    // Selects how to run the days
    run_type: RunType,

    #[structopt(long, default_value = "3")]
    /// Benchmarking period in seconds to measure run time of parts
    bench_time: u32,

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

fn basic_run(alloc: &'static TracingAlloc, year: u16, days: &[Day]) -> Result<(), BenchError> {
    println!("   Day | {:<30} ", "Answer");
    println!("_______|_{0:_<30}", "");

    let (sender, receiver) = channel();

    for day in days {
        let parts = iter::once(day.part_1).chain(day.part_2).zip(1..);

        for (f, i) in parts {
            let name = style(format!("{:2}.{}", day.day, i));
            let bench = Bench {
                alloc,
                id: 0,
                chan: sender.clone(),
                args: &ARGS,
            };

            let result = input(year, day.day)
                .open()
                .and_then(|input| f(&input, bench))
                .and_then(|_| receiver.recv().map_err(|_| BenchError::ChannelError(0)));

            match result {
                Ok(BenchEvent::Answer { answer, .. }) => {
                    println!("  {} | {}", name.green(), answer);
                }
                Ok(BenchEvent::Error { err, .. }) => {
                    println!("  {} | {}", name.red(), err);
                }
                Err(err @ (BenchError::InputFileError { .. } | BenchError::UserError(..))) => {
                    println!("  {} | {}", name.red(), err);
                }
                _ => unreachable!(),
            }
        }
    }

    Ok(())
}

pub fn run(alloc: &'static TracingAlloc, year: u16, days: &[Day]) -> Result<(), BenchError> {
    let args = Args::from_args();

    println!("Advent of Code {}", year);
    match args.run_type {
        RunType::Run => basic_run(alloc, year, days),
        RunType::Simple => run_simple_bench(alloc, year, days),
        RunType::Detailed => todo!(),
    }
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
