use std::{panic, time::Duration};

use bytesize::ByteSize;
use console::Term;
use crossbeam_channel::Receiver;
use drawille::Canvas;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::{ThreadPool, ThreadPoolBuilder};

use crate::{
    bench::{bench_worker, BenchEvent, MemoryData, RuntimeData, SetupFunction},
    misc::ArrWindows,
    render_decimal, render_duration, Bench, BenchError, BenchResult, Day, TracingAlloc, ARGS,
};

struct BenchedFunction {
    name: &'static str,
    day: u8,
    day_function_id: String,
    function: SetupFunction,
    message: String,
    is_multiline_answer: bool,
    is_error: bool,
    timing_data: Option<RuntimeData>,
    memory_data: Option<MemoryData>,
}

fn render_function_data(func: BenchedFunction, term_width: u16) {
    let name = format!(" {} ", func.day_function_id,);
    println!("{:-^width$}", name, width = term_width as usize);
    print!("  Answer: ");
    if func.is_multiline_answer {
        println!();
        println!("{}", func.message);
        println!();
    } else {
        println!("{}", func.message);
    }

    if func.is_error {
        return;
    }

    let timing = func.timing_data.expect("No timing data?");
    println!("  -- Timing");
    println!(
        "    -- Mean:       {}    Std. Dev:   {}",
        render_duration(timing.mean, true),
        render_duration(timing.std_dev, false)
    );
    println!(
        "    -- 1st Quart.: {}    3rd Quart.: {}",
        render_duration(timing.first_quartile, false,),
        render_duration(timing.third_quartile, false,)
    );
    println!(
        "    -- Samples:    {}     Outliers:   {}",
        render_decimal(timing.sample_count),
        render_decimal(timing.outlier_count),
    );

    let memory = func.memory_data.expect("No memory data?");
    let max_memory = format!("{}", ByteSize(memory.max_memory as u64));
    println!("  -- Memory");
    println!(
        "    -- N. Allocs:  {}     Max Mem.: {}",
        render_decimal(memory.num_allocs),
        max_memory
    );

    if memory.num_allocs != 0 {
        const CHART_HEIGHT: f32 = 10.0 * 4.0;
        let chart_width = (term_width as u32 - max_memory.len() as u32 - 3) * 2;
        let x_per_pixel = memory.end_ts / chart_width as f32;
        let y_per_pixel = memory.max_memory as f32 / CHART_HEIGHT;

        let rendered_max_ts = render_duration(memory.end_ts_duration, false);
        let (_, ts_unit) = rendered_max_ts.rsplit_once(' ').unwrap();

        let mut canvas = Canvas::new(chart_width, 10);
        for &[(sx, sy), (ex, ey)] in ArrWindows::new(&memory.graph_points) {
            let sx = (sx / x_per_pixel) as u32;
            let sy = (CHART_HEIGHT - (sy / y_per_pixel)) as u32;
            let ex = (ex / x_per_pixel) as u32;
            let ey = (CHART_HEIGHT - (ey / y_per_pixel)) as u32;
            canvas.line(sx, sy, ex, ey);
        }
        let rows = canvas.rows();
        match &*rows {
            [first, middle @ .., end] => {
                println!(" {} {}", first, max_memory);
                middle.iter().for_each(|r| println!(" {}", r));
                println!(" {} {}", end, ByteSize(0));
            }
            _ => unreachable!(),
        }
        println!(
            " 0 {}{:>width$}",
            ts_unit,
            rendered_max_ts,
            width = (term_width as usize) - 5 - max_memory.len() - 2
        );
    }
}

fn ui_update_worker(funcs: &mut Vec<BenchedFunction>, bench_events: Receiver<BenchEvent>) {
    let progress_bar = ProgressBar::new(funcs.len() as u64);
    progress_bar.set_style(
        ProgressStyle::default_bar()
            .template(" [{elapsed_precise}] [{wide_bar:.cyan/blue}] {percent}% ({eta})")
            .progress_chars("#>-"),
    );

    bench_events.iter().for_each(|event| match event {
        BenchEvent::Answer { answer, id, is_alt } => {
            funcs[id].message = answer;
            funcs[id].is_multiline_answer = is_alt;
        }
        BenchEvent::Memory { data, id } => {
            funcs[id].memory_data = Some(data);
        }
        BenchEvent::Timing { data, id } => {
            funcs[id].timing_data = Some(data);
        }
        BenchEvent::Error { err, id } => {
            funcs[id].message = err;
            funcs[id].is_error = true;
            progress_bar.inc(1);
        }
        BenchEvent::Finish { .. } => {
            progress_bar.inc(1);
        }
    });

    progress_bar.finish_and_clear();
}

fn bench_days(
    alloc: &'static TracingAlloc,
    pool: &ThreadPool,
    mut funcs: Vec<BenchedFunction>,
    term_width: u16,
) -> Result<Duration, BenchError> {
    let (sender, receiver) = crossbeam_channel::unbounded();

    // In order to prevent the panic message from messing up our output, we'll
    // replace the panic hook with one that doesn't print anything.
    // One issue here is that there are sources of panics between here
    // and the restoration that *should* be printed. I need to figure out a way to handle that.
    let old_panic_hook = panic::take_hook();
    panic::set_hook(Box::new(|_| {})); // Just eat the panic.

    for (id, func) in funcs.iter().enumerate() {
        let bench = Bench {
            alloc,
            id,
            chan: sender.clone(),
            run_only: false,
            bench_time: ARGS.bench_time,
        };
        let day = func.day;
        let f = func.function;
        pool.spawn(move || bench_worker(day, bench, f));
    }

    // If we don't drop this thread's sender the handler thread will never stop.
    drop(sender);

    ui_update_worker(&mut funcs, receiver);

    panic::set_hook(old_panic_hook);

    // Now we've benchmarked, we'll render all the days.
    let mut total_time = Duration::ZERO;
    let mut day_id = 99;

    for func in funcs {
        if func.day != day_id {
            day_id = func.day;
            println!("{:#<width$}", "", width = term_width as usize);
            let day_num = format!("Day {}", func.day);
            println!("# {:^width$} #", day_num, width = term_width as usize - 4);
            println!("# {:^width$} #", func.name, width = term_width as usize - 4);
            println!("{:#<width$}", "", width = term_width as usize);
        }

        if let Some(time) = &func.timing_data {
            total_time += time.mean;
        }
        render_function_data(func, term_width);
        println!();
    }

    Ok(total_time)
}

pub fn run_detailed_bench(alloc: &'static TracingAlloc, days: &[&Day]) -> BenchResult {
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

    // We'll be rendering a graph the size of the terminal, so we need the width.
    let stdout = Term::stdout();
    let (_, term_width) = stdout.size();

    let mut benched_functions = Vec::new();
    for day in days {
        benched_functions.push(BenchedFunction {
            name: day.name,
            day: day.day,
            day_function_id: "Part 1".to_owned(),
            function: day.part_1,
            message: String::new(),
            is_multiline_answer: false,
            is_error: false,
            timing_data: None,
            memory_data: None,
        });
        if let Some(p2) = day.part_2 {
            benched_functions.push(BenchedFunction {
                name: day.name,
                day: day.day,
                day_function_id: "Part 2".to_owned(),
                function: p2,
                message: String::new(),
                is_multiline_answer: false,
                is_error: false,
                timing_data: None,
                memory_data: None,
            });
        }

        for &(name, extra) in day.other {
            benched_functions.push(BenchedFunction {
                name: day.name,
                day: day.day,
                day_function_id: name.to_owned(),
                function: extra,
                message: String::new(),
                is_multiline_answer: false,
                is_error: false,
                timing_data: None,
                memory_data: None,
            });
        }
    }

    let total_time = bench_days(alloc, &pool, benched_functions, term_width)?;
    println!("Total Time: {}", render_duration(total_time, false));

    Ok(())
}
