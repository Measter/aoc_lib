use std::{
    iter,
    panic::{self},
    thread,
    time::Duration,
};

use bytesize::ByteSize;
use console::{style, Term};
use crossbeam_channel::{Receiver, Sender};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use rayon::{ThreadPool, ThreadPoolBuilder};

use crate::{
    bench::{
        bench_worker, AlternateAnswer, Bench, BenchEvent, MemoryData, RuntimeData, SetupFunction,
    },
    print_alt_answers, print_footer, print_header, render_decimal, render_duration, BenchError,
    BenchResult, Day, TracingAlloc, ARGS, TABLE_DETAILED_COLS_WIDTH, TABLE_PRE_COL_WIDTH,
};

struct BenchedFunction {
    day: u8,
    day_function_id: u8,
    function: SetupFunction,
    message: String,
    is_error: bool,
    timing_data: Option<RuntimeData>,
    memory_data: Option<MemoryData>,
    finished_spinner: ProgressStyle,
    error_spinner: ProgressStyle,
    bar: Option<ProgressBar>,
    term_width: usize,
}

impl BenchedFunction {
    fn answer(&mut self, ans: String) {
        self.message = ans;
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
        self.message = err;
        self.is_error = true;
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
        if self.is_error || ARGS.run_type.is_run_only() {
            // Keep the error within the width of the terminal.
            self.message
                .char_indices()
                .nth(self.term_width - TABLE_PRE_COL_WIDTH)
                .map(|(i, _)| &self.message[..i])
                .unwrap_or(&self.message)
                .to_owned()
        } else {
            let msg_max_width = self
                .term_width
                .saturating_sub(TABLE_DETAILED_COLS_WIDTH)
                .max(12)
                .min(30);

            let msg = self
                .message
                .char_indices()
                .nth(msg_max_width)
                .map(|(i, _)| &self.message[..i])
                .unwrap_or(&self.message);

            let (mean_time, std_dev) = self
                .timing_data
                .as_ref()
                .map(|td| {
                    (
                        render_duration(td.mean, true),
                        render_duration(td.std_dev, false),
                    )
                })
                .unwrap_or_default();

            let (allocs, mem) = self
                .memory_data
                .as_ref()
                .map(|md| {
                    (
                        render_decimal(md.num_allocs),
                        format!("{}", ByteSize(md.max_memory as u64)),
                    )
                })
                .unwrap_or_default();

            format!(
                "{:<msg_width$} | {:<8} (σ {:<8}) | {:<7} | {}",
                msg,
                mean_time,
                std_dev,
                allocs,
                mem,
                msg_width = msg_max_width
            )
        }
    }
}

fn tick_bars_worker(bars: Vec<ProgressBar>) {
    loop {
        let all_finished = bars.iter().fold(true, |all_finished, bar| {
            if !bar.is_finished() {
                bar.tick();
            }
            all_finished & bar.is_finished()
        });

        if all_finished {
            break;
        }

        thread::sleep(Duration::from_millis(250));
    }
}

fn ui_update_worker(
    mut funcs: Vec<BenchedFunction>,
    receiver: Receiver<BenchEvent>,
    alt_answers: Sender<AlternateAnswer>,
    time_sender: Sender<Duration>,
) -> Vec<BenchedFunction> {
    for event in receiver.iter() {
        match event {
            BenchEvent::Answer {
                answer,
                id,
                is_alt: false,
            } => funcs[id].answer(answer),
            BenchEvent::Answer { answer, id, .. } => {
                let func = &mut funcs[id];
                alt_answers
                    .send(AlternateAnswer {
                        answer,
                        day: func.day,
                        day_function_id: func.day_function_id,
                    })
                    .expect("Failed to send alternate answer from UI thread");
                func.answer("Check alternate answers".to_owned());
            }
            BenchEvent::Memory { data, id } => funcs[id].memory(data),
            BenchEvent::Timing { data, id } => {
                time_sender
                    .send(data.mean)
                    .expect("Failed to send timing from UI thread");
                funcs[id].timing(data);
            }
            BenchEvent::Error { err, id } => funcs[id].error(err),
            BenchEvent::Finish { id } => funcs[id].finish(),
        }
    }

    funcs
}

fn bench_days_chunk(
    alloc: &'static TracingAlloc,
    mut funcs: Vec<BenchedFunction>,
    alt_answer_sender: Sender<AlternateAnswer>,
    spinner_style: &ProgressStyle,
    pool: &ThreadPool,
) -> Result<Duration, BenchError> {
    let (sender, receiver) = crossbeam_channel::unbounded();
    let multi_bars = MultiProgress::new();
    multi_bars.set_move_cursor(true);

    let mut bars = Vec::new();

    // In order to prevent the panic message from messing up our output, we'll
    // replace the panic hook with one that doesn't print anything.
    // One issue here is that there are sources of panics between here
    // and the restoration that *should* be printed. I need to figure out a way to handle that.
    let old_panic_hook = panic::take_hook();
    panic::set_hook(Box::new(|_| {})); // Just eat the panic.

    for (id, func) in funcs.iter_mut().enumerate() {
        let bar = multi_bars.add(ProgressBar::new_spinner());
        bar.set_prefix(format!("{:2}.{}", func.day, func.day_function_id));
        bar.set_style(spinner_style.clone());

        bars.push(bar.clone());
        func.bar = Some(bar);

        let bench = Bench {
            alloc,
            id,
            chan: sender.clone(),
            run_only: ARGS.run_type.is_run_only(),
            bench_time: ARGS.bench_time,
        };
        let day = func.day;
        let f = func.function;

        pool.spawn(move || bench_worker(day, bench, f));
    }

    // Using the built-in steady tick spawns a thread for each bar. We could have up to 50.
    // Seems wasteful. Let's just spawn a single thread to tick them all instead.
    let tick_thread = thread::spawn(move || tick_bars_worker(bars));

    // If we don't drop this thread's sender the handler thread will never stop.
    drop(sender);

    let (time_sender, time_receiver) = crossbeam_channel::unbounded();
    // We don't want to spawn the handler thread in the worker pool, because the benchmarking will
    // hog the pool's threads, meaning the UI updates won't happen in a timely manner.
    // Rayon's scope function seems to end up in the pool, so we need to make sure we get a new thread.
    let ui_update_thread =
        thread::spawn(move || ui_update_worker(funcs, receiver, alt_answer_sender, time_sender));

    let mb_join_res = multi_bars.join_and_clear();
    let ui_thread_res = ui_update_thread.join();
    let tick_res = tick_thread.join();

    panic::set_hook(old_panic_hook);

    mb_join_res.expect("Failed to join progress bars");
    tick_res.expect("Failed to join tick thread");
    let funcs = ui_thread_res.expect("Failed to join handler thread");

    // Now we've finished, to clear up a render bug when the parts finish rapidly
    // we'll re-render on stdout.
    for func in funcs {
        let day = format!("{:>2}.{}", func.day, func.day_function_id);
        let day = if func.is_error {
            style(day).red()
        } else {
            style(day).green()
        };
        println!("  {} | {}", day, func.render());
    }

    Ok(time_receiver.iter().sum())
}

pub fn run_simple_bench(alloc: &'static TracingAlloc, days: &[&Day]) -> BenchResult {
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

    // MultiProgress goes a bit nuts if the terminal isn't tall enough to display all the bars
    // at once. So we need to chunk the functions to bench based on how tall the terminal is.
    let stdout = Term::stdout();
    let (rows, cols) = stdout.size();
    // Add room for header and trailing line.
    let rows = rows.saturating_sub(5) as usize;

    print_header(cols as _);

    let spinner_style = ProgressStyle::default_spinner()
        .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ ")
        .template("{spinner} {prefix:.dim} | {msg}");
    let finished_spinner = spinner_style
        .clone()
        .template("{spinner} {prefix:.green} | {msg}");
    let error_spinner = spinner_style
        .clone()
        .template("{spinner} {prefix:.red} | {msg}");

    let mut benched_functions = Vec::new();
    let mut cur_chunk = Vec::new();

    for day in days {
        let parts = iter::once(day.part_1).chain(day.part_2).zip(1..);

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
                message: String::new(),
                is_error: false,
                timing_data: None,
                memory_data: None,
                finished_spinner: finished_spinner.clone(),
                error_spinner: error_spinner.clone(),
                bar: None,
                term_width: cols as usize,
            };

            cur_chunk.push(p1f);
        }
    }
    if !cur_chunk.is_empty() {
        benched_functions.push(cur_chunk);
    }

    let (alt_answer_sender, alt_answer_receiver) = crossbeam_channel::unbounded();

    let total_time = benched_functions
        .into_iter()
        .map(|days_chunk| {
            bench_days_chunk(
                alloc,
                days_chunk,
                alt_answer_sender.clone(),
                &spinner_style,
                &pool,
            )
        })
        .try_fold(Duration::ZERO, |acc, a| a.map(|a| a + acc))?;

    print_footer(total_time, cols as _);

    drop(alt_answer_sender);
    print_alt_answers(alt_answer_receiver);

    Ok(())
}
