use std::{fmt::Display, iter, num::ParseIntError, time::Duration};

use once_cell::sync::Lazy;
use structopt::StructOpt;
use thiserror::Error;

mod alloc;
mod bench;
pub mod misc;
pub mod parsers;

pub use alloc::TracingAlloc;
pub use bench::Bench;
use bench::{simple::run_simple_bench, Function, MemoryBenchError};

use crate::bench::BenchEvent;

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

    #[error("Day {} not defined", .0)]
    DaysFilterError(u8),
}

#[allow(non_snake_case)]
pub fn UserError<E: Into<Box<dyn std::error::Error + Send + Sync>>>(e: E) -> BenchError {
    BenchError::UserError(e.into())
}

#[derive(Debug, Error)]
pub enum NoError {}

// Getting an inexplicable compiler error if I just try let structopt handle a the
// Option<Vec<u8>>, so I'm using this as a workaround.
fn parse_days_list(src: &str) -> Result<u8, ParseIntError> {
    src.parse()
}

#[derive(Clone, StructOpt, PartialEq, Eq)]
pub(crate) enum RunType {
    /// Just runs the day's primary functions.
    Run {
        #[structopt(short, long, parse(try_from_str = parse_days_list))]
        /// List of days to run [default: all]
        days: Option<Vec<u8>>,
    },
    /// Benchmarks the days' primary functions, and lists them in a simple format.
    Simple {
        #[structopt(short, long, parse(try_from_str = parse_days_list))]
        /// List of days to run [default: all]
        days: Option<Vec<u8>>,
    },
    /// Benchmarks all the days' functions, and provides a more detailed listing.
    Detailed {
        #[structopt(short, long, parse(try_from_str = parse_days_list))]
        /// List of days to run [default: all]
        days: Option<Vec<u8>>,
    },
}

impl RunType {
    pub(crate) fn is_run_only(&self) -> bool {
        matches!(self, RunType::Run { .. })
    }

    fn days(&self) -> Option<&[u8]> {
        match self {
            RunType::Run { days } | RunType::Simple { days } | RunType::Detailed { days } => {
                days.as_deref()
            }
        }
    }
}

#[derive(StructOpt)]
pub(crate) struct Args {
    #[structopt(subcommand)]
    // Selects how to run the days
    run_type: RunType,

    #[structopt(long, default_value = "3")]
    /// Benchmarking period in seconds to measure run time of parts
    bench_time: u64,

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

fn get_days<'d>(days: &'d [Day], filter: Option<&[u8]>) -> Result<Vec<&'d Day>, BenchError> {
    match filter {
        Some([]) | None => Ok(days.iter().collect()),
        Some(filter) => {
            let mut new_days = Vec::with_capacity(filter.len());

            for &filter_day in filter {
                let day = days
                    .iter()
                    .find(|d| d.day == filter_day)
                    .ok_or(BenchError::DaysFilterError(filter_day))?;
                new_days.push(day);
            }

            new_days.sort_by_key(|d| d.day);
            Ok(new_days)
        }
    }
}

pub fn get_precision(val: Duration) -> usize {
    if val.as_nanos() < 1000 {
        0
    } else {
        3
    }
}

fn print_header() {
    if ARGS.run_type.is_run_only() {
        println!("   Day | {:<30} ", "Answer");
        println!("_______|_{0:_<30}", "");
    } else {
        println!("   Day | {:<30} | {:<10} | Max Mem.", "Answer", "Time");
        println!("_______|_{0:_<30}_|_{0:_<10}_|______________", "");
    }
}

fn print_footer(total_time: Duration) {
    if ARGS.run_type.is_run_only() {
        println!("_______|_{0:_<30}", "");
    } else {
        let prec = get_precision(total_time);
        println!("_______|_{0:_<30}_|_{0:_<10}_|______________", "");
        println!(
            " Total Time: {:26} | {:.prec$?}",
            "",
            total_time,
            prec = prec
        );
    }
}

// No need for all of the complex machinery just to run the two functions, given we want
// panics to happen as normal.
fn run_single(alloc: &'static TracingAlloc, year: u16, day: &Day) -> Result<(), BenchError> {
    print_header();

    let (sender, receiver) = crossbeam_channel::unbounded();

    let parts = iter::once(day.part_1).chain(day.part_2).zip(1..);

    for (part, id) in parts {
        let dummy = Bench {
            alloc,
            id: 0,
            chan: sender.clone(),
            run_only: true,
            bench_time: 0,
        };

        let input = input(year, day.day).open()?;
        part(&input, dummy)?;

        let message = match receiver.recv().expect("Failed to receive from channel") {
            BenchEvent::Answer { answer: msg, .. } | BenchEvent::Error { err: msg, .. } => msg,
            _ => unreachable!("Should only receive an Answer or Error"),
        };

        println!("  {:>2}.{} | {}", day.day, id, message);
    }

    print_footer(Duration::ZERO);

    Ok(())
}

pub fn run(alloc: &'static TracingAlloc, year: u16, days: &[Day]) -> Result<(), BenchError> {
    let days = get_days(days, ARGS.run_type.days())?;

    println!("Advent of Code {}", year);
    match (&ARGS.run_type, &*days) {
        (RunType::Run { .. }, [day]) => run_single(alloc, year, day),
        (RunType::Run { .. } | RunType::Simple { .. }, days) => {
            run_simple_bench(alloc, year, &days)
        }
        (RunType::Detailed { .. }, _) => todo!(),
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
