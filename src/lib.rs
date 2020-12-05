use std::{
    fmt::Display,
    time::{Duration, Instant},
};

use color_eyre::eyre::{eyre, Context, Result};

pub mod parsers;

type PartFunction<Input, Output> = dyn Fn(Input) -> Result<Output>;

#[allow(non_snake_case)]
pub fn NOT_IMPLEMENTED<I>(_: I) -> Result<&'static str> {
    Ok("Not Implemented")
}

fn bench_function<Input, Output>(
    id: u8,
    input: Input,
    part: &PartFunction<Input, Output>,
) -> Result<()>
where
    Output: Display,
    Input: Copy,
{
    println!("-- Part {} --", id);
    let part_result = part(input).with_context(|| eyre!("Error running Part {}", id))?;
    println!("Result: {}", part_result);

    // Run a few times to get an estimate of how long it takes.
    let mut min_run = Duration::from_secs(u64::MAX);

    for _ in 0..5 {
        let now = Instant::now();
        let _ = part(input);
        let time = now.elapsed();

        if time < min_run {
            min_run = time;
        }
    }

    let total_runs = (3.0 / min_run.as_secs_f64()).ceil().max(10.0).min(10e6) as u32;

    let mut total_time = Duration::default();
    let mut min_run = Duration::from_secs(u64::MAX);
    let mut max_run = Duration::default();

    for _ in 0..total_runs {
        let start = Instant::now();
        let _ = part(input); // We'll just discard the result as we handled errors above.
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

    let min_prec = if min_run.as_nanos() < 1000 { 0 } else { 3 };
    let mean_prec = if mean_run.as_nanos() < 1000 { 0 } else { 3 };
    let max_prec = if max_run.as_nanos() < 1000 { 0 } else { 3 };

    println!(
        "Times for {} runs: [{:.min_prec$?} .. {:.mean_prec$?} .. {:.max_prec$?}]",
        human_format::Formatter::new().format(total_runs as f64),
        min_run,
        mean_run,
        max_run,
        min_prec = min_prec,
        mean_prec = mean_prec,
        max_prec = max_prec
    );

    println!();

    Ok(())
}

pub fn run<Input, Output, Output2>(
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
    println!("{}\n", name);

    bench_function(1, input, part1)?;
    bench_function(2, input, part2)?;

    Ok(())
}
