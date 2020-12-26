use color_eyre::eyre::{eyre, Context, Result};
use structopt::StructOpt;

mod alloc;
mod bench;
pub mod parsers;
pub use alloc::TracingAlloc;

use std::fmt::Display;

type PartFunction<Input, Output> = dyn Fn(Input) -> Result<Output>;

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

    #[structopt(long = "p1")]
    /// File path for the part 1 memory trace
    part1_file: Option<String>,

    #[structopt(long = "p2")]
    /// File path for the part 2 memory trace
    part2_file: Option<String>,

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
    pub fn open(self) -> Result<String> {
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

        Ok(std::fs::read_to_string(&path)
            .with_context(|| eyre!("Unable to open file: {}", path))?)
    }
}

pub fn input(year: u16, day: u8) -> InputFile<ProblemInput> {
    InputFile {
        year,
        day,
        example_id: None,
    }
}

pub fn run<Input, Output, Output2>(
    alloc: &TracingAlloc,
    name: &str,
    input: Input,
    part1: &PartFunction<Input, Output>,
    part2: &PartFunction<Input, Output2>,
) -> Result<()>
where
    Output: Display,
    Output2: Display,
    Input: Copy,
{
    let args = Args::from_args();

    if args.no_bench {
        println!("{}", name);

        let p1_result = part1(input).with_context(|| eyre!("Error running Part 1"))?;
        let p2_result = part2(input).with_context(|| eyre!("Error running Part 2"))?;

        println!("Part 1: {}", p1_result);
        println!("Part 2: {}", p2_result);

        return Ok(());
    }

    let part1_result = bench::benchmark(alloc, &args, args.part1_file.as_ref(), input, 1, part1)?;
    let part2_result = bench::benchmark(alloc, &args, args.part2_file.as_ref(), input, 2, part2)?;

    bench::print_results(
        args.output.unwrap_or(OutputType::Table),
        name,
        &part1_result,
        &part2_result,
    )?;

    Ok(())
}
