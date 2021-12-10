use std::{fmt::Display, iter, num::ParseIntError, time::Duration};

use console::{style, Term};
use crossbeam_channel::Receiver;
use once_cell::sync::Lazy;
use structopt::StructOpt;
use thiserror::Error;

mod alloc;
mod bench;
mod input;
pub mod misc;
pub mod parsers;

pub use alloc::TracingAlloc;
pub use bench::Bench;
use bench::{
    simple::run_simple_bench, AlternateAnswer, BenchEvent, MemoryBenchError, SetupFunction,
};
pub use input::*;

use crate::bench::detailed::run_detailed_bench;

static ARGS: Lazy<Args> = Lazy::new(Args::from_args);

pub type BenchResult = Result<(), BenchError>;

const TABLE_PRE_COL_WIDTH: usize = 9;
// The amount of space taken up by the ticker, day ID, and bench data columns, plus separators.
const TABLE_DETAILED_COLS_WIDTH: usize = TABLE_PRE_COL_WIDTH + 46;

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

pub struct ParseResult<T>(pub T);
impl<T> Display for ParseResult<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Parsed data")
    }
}

// Getting an inexplicable compiler error if I just try let structopt handle a the
// Option<Vec<u8>>, so I'm using this as a workaround.
fn parse_days_list(src: &str) -> Result<u8, ParseIntError> {
    src.parse()
}

#[derive(Clone, StructOpt, PartialEq, Eq)]
pub(crate) enum RunType {
    /// Just runs the day's primary functions.
    Run {
        #[structopt(parse(try_from_str = parse_days_list))]
        /// List of days to run [default: all]
        days: Vec<u8>,
    },
    /// Benchmarks the days' primary functions, and lists them in a simple format.
    Bench {
        #[structopt(parse(try_from_str = parse_days_list))]
        /// List of days to run [default: all]
        days: Vec<u8>,

        #[structopt(short)]
        /// Render more detailed benchmarking info.
        detailed: bool,
    },
}

impl RunType {
    pub(crate) fn is_run_only(&self) -> bool {
        matches!(self, RunType::Run { .. })
    }

    fn days(&self) -> &[u8] {
        match self {
            RunType::Run { days } | RunType::Bench { days, .. } => days,
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

#[derive(Clone)]
pub struct Day {
    pub name: &'static str,
    pub day: u8,
    pub part_1: SetupFunction,
    pub part_2: Option<SetupFunction>,
    pub parse: Option<SetupFunction>,
    pub other: Vec<(&'static str, SetupFunction)>,
}

fn get_days<'d>(days: &'d [Day], filter: &[u8]) -> Result<Vec<&'d Day>, BenchError> {
    match filter {
        [] => Ok(days.iter().collect()),
        filter => {
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

pub(crate) fn render_decimal(val: usize) -> String {
    let (factor, unit) = if val < 10usize.pow(3) {
        (10f64.powi(0), "")
    } else if val < 10usize.pow(6) {
        (10f64.powi(-3), " k")
    } else if val < 10usize.pow(9) {
        (10f64.powi(-6), " M")
    } else {
        (10f64.powi(-9), " B")
    };

    let val_f = (val as f64) * factor;
    let prec = if val < 1000 {
        0 // No need for decimals here.
    } else if val_f < 10.0 {
        3
    } else if val_f < 100.0 {
        2
    } else if val_f < 1000.0 {
        1
    } else {
        0
    };

    format!(
        "{:>width$.prec$}{}",
        val_f,
        unit,
        prec = prec,
        width = 7 - unit.len()
    )
}

pub fn render_duration(duration: Duration, colour: bool) -> String {
    // The logic here is basically copied from Criterion.
    let time = duration.as_nanos() as f64;

    let (factor, unit) = if time < 10f64.powi(0) {
        (10f64.powi(3), "ps")
    } else if time < 10f64.powi(3) {
        (10f64.powi(0), "ns")
    } else if time < 10f64.powi(6) {
        (10f64.powi(-3), "µs")
    } else if time < 10f64.powi(9) {
        (10f64.powi(-6), "ms")
    } else {
        (10f64.powi(-9), "s ")
    };

    let time = time * factor;

    let prec = if time < 10.0 {
        3
    } else if time < 100.0 {
        2
    } else if time < 1000.0 {
        1
    } else {
        0
    };

    let mut rendered_time = style(format!("{:>5.prec$}", time, prec = prec));
    let duration_millis = duration.as_millis();
    if colour {
        if duration_millis > 500 {
            rendered_time = rendered_time.red();
        } else if colour && duration_millis > 50 {
            rendered_time = rendered_time.yellow();
        }
    }

    format!("{} {}", rendered_time, unit)
}

fn print_header(term_width: usize) {
    if ARGS.run_type.is_run_only() {
        println!("   Day | Answer");
        println!("_______|_{0:_<30}", "");
    }
    {
        let msg_max_width = term_width
            .saturating_sub(TABLE_DETAILED_COLS_WIDTH)
            .max(12)
            .min(30);
        println!(
            "   Day | {:<max_width$} | {:<21} | Allocs  | Max Mem.",
            "Answer",
            "Time",
            max_width = msg_max_width
        );
        println!(
            "_______|_{0:_<max_width$}_|_{0:_<21}_|_________|__________",
            "",
            max_width = msg_max_width
        );
    }
}

fn print_footer(total_time: Duration, term_width: usize) {
    if ARGS.run_type.is_run_only() {
        println!("_______|_{0:_<30}", "");
    } else {
        let msg_max_width = term_width
            .saturating_sub(TABLE_DETAILED_COLS_WIDTH)
            .max(12)
            .min(30);
        let time = render_duration(total_time, false);
        println!(
            "_______|_{0:_<max_width$}_|_{0:_<21}_|_________|__________",
            "",
            max_width = msg_max_width
        );
        println!(
            " Total Time: {:max_width$} | {}",
            "",
            time,
            max_width = msg_max_width - 4
        );
    }
}

fn print_alt_answers(receiver: Receiver<AlternateAnswer>) {
    if !receiver.is_empty() {
        println!("\n -- Alternate Answers --");
        for alt_ans in receiver.iter() {
            println!("Day {}, Part: {}", alt_ans.day, alt_ans.day_function_id);
            println!("{}\n", alt_ans.answer);
        }
    }
}

// No need for all of the complex machinery just to run the two functions, given we want
// panics to happen as normal.
fn run_single(alloc: &'static TracingAlloc, day: &Day) -> Result<(), BenchError> {
    let stdout = Term::stdout();
    let (_, cols) = stdout.size();
    print_header(cols as _);

    let (sender, receiver) = crossbeam_channel::unbounded();
    let (alt_answer_sender, alt_answer_receiver) = crossbeam_channel::unbounded();

    let parts = iter::once(day.part_1).chain(day.part_2).zip(1..);

    for (part, id) in parts {
        let dummy = Bench {
            alloc,
            id: 0,
            chan: sender.clone(),
            run_only: true,
            bench_time: 0,
        };

        let input = input(day.day).open()?;
        part(&input, dummy)?;

        let message = match receiver.recv().expect("Failed to receive from channel") {
            BenchEvent::Answer {
                answer,
                is_alt: true,
                ..
            } => {
                alt_answer_sender
                    .send(AlternateAnswer {
                        answer,
                        day: day.day,
                        day_function_id: id,
                    })
                    .expect("Failed to send alternate answer");
                "Check alternate answers".to_owned()
            }
            BenchEvent::Answer { answer: msg, .. } | BenchEvent::Error { err: msg, .. } => msg,
            _ => unreachable!("Should only receive an Answer or Error"),
        };

        println!("  {:>2}.{} | {}", day.day, id, message);
    }

    print_footer(Duration::ZERO, cols as _);

    drop(alt_answer_sender);
    print_alt_answers(alt_answer_receiver);

    Ok(())
}

pub fn run(alloc: &'static TracingAlloc, year: u16, days: &[Day]) -> Result<(), BenchError> {
    let days = get_days(days, ARGS.run_type.days())?;

    println!("Advent of Code {}", year);
    match (&ARGS.run_type, &*days) {
        (RunType::Run { .. }, [day]) => run_single(alloc, day),
        (
            RunType::Run { .. }
            | RunType::Bench {
                detailed: false, ..
            },
            days,
        ) => run_simple_bench(alloc, days),

        (RunType::Bench { .. }, days) => run_detailed_bench(alloc, days),
    }
}
