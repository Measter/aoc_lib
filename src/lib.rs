use once_cell::sync::Lazy;
use structopt::StructOpt;
use thiserror::Error;

mod alloc;
mod bench;
pub use bench::{BenchResult, MemoryBenchError};
pub mod parsers;
pub use alloc::TracingAlloc;

use std::fmt::Display;

static ARGS: Lazy<Args> = Lazy::new(Args::from_args);

type PartFunction<'a, Output, OutputErr> = dyn Fn() -> Result<Output, OutputErr> + 'a;

#[derive(Debug, Error)]
pub enum BenchError<T: std::fmt::Debug> {
    #[error("Error running {}: {:?}", .1, .0)]
    FunctionError(T, &'static str),

    #[error("Error performing memory benchmark {}: {}", .1, .0)]
    MemoryBenchError(#[source] MemoryBenchError, &'static str),
}

#[derive(Copy, Clone, StructOpt)]
pub(crate) enum OutputType {
    /// Print a table of timings with a memory use graph (default)
    Table,
    #[structopt(name = "markdown")]
    /// Print a markdown table
    MarkDown,
}

#[derive(Debug, Error)]
#[error("Error opening input file '{}': {:?}", .name, .inner)]
pub struct InputFileError {
    #[source]
    pub inner: std::io::Error,
    pub name: String,
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

    #[structopt(subcommand)]
    /// The layout of the output
    output: Option<OutputType>,
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
    pub fn open(self) -> Result<String, InputFileError> {
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

        Ok(std::fs::read_to_string(&path).map_err(|e| InputFileError {
            inner: e,
            name: path,
        })?)
    }
}

pub fn input(year: u16, day: u8) -> InputFile<ProblemInput> {
    InputFile {
        year,
        day,
        example_id: None,
    }
}

pub fn bench<Output, OutputErr: std::fmt::Debug>(
    alloc: &TracingAlloc,
    name: &'static str,
    func: &PartFunction<Output, OutputErr>,
) -> Result<(Output, BenchResult), BenchError<OutputErr>> {
    eprintln!("Running {}...", name);
    let res = func().map_err(|e| BenchError::FunctionError(e, name))?;

    let bench_res = if ARGS.no_bench {
        BenchResult::new(name)
    } else {
        bench::benchmark(alloc, &ARGS, name, func)
            .map_err(|e| BenchError::MemoryBenchError(e, name))?
    };

    Ok((res, bench_res))
}

pub fn display_results(name: &str, results: &[(&dyn Display, BenchResult)]) {
    if ARGS.no_bench {
        println!("{}", name);
        for (res, bench) in results.iter() {
            let output = format!("{}", res);
            if !output.is_empty() {
                eprintln!("{} Result: {}", bench.name, res);
            }
        }
    } else if results.is_empty() {
        eprintln!("No results to display");
    } else {
        bench::print_results(ARGS.output.unwrap_or(OutputType::Table), name, results);
    }
}
