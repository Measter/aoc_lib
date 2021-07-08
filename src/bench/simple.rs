use std::{
    iter,
    panic::catch_unwind,
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use bytesize::ByteSize;
use console::{style, Term};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use rayon::{ThreadPool, ThreadPoolBuilder};

use crate::{
    bench::{get_precision, Bench, BenchEvent, Function, MemoryData, RuntimeData},
    input, BenchError, BenchResult, Day, RunType, TracingAlloc, ARGS,
};

struct BenchedFunction {
    // name: &'static str,
    day: u8,
    day_function_id: u8,
    function: Function,
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
        if self.is_error || ARGS.run_type == RunType::Run {
            // Keep the error within the width of the terminal.
            self.message
                .char_indices()
                .nth(self.term_width - 10)
                .map(|(i, _)| &self.message[..i])
                .unwrap_or(&self.message)
                .to_owned()
        } else {
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

            format!("{:<30} | {:<10} | {}", self.message, time, mem)
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
    let (sender, receiver) = crossbeam_channel::unbounded();
    let multi_bars = MultiProgress::new();
    multi_bars.set_move_cursor(true);

    let mut bars = Vec::new();

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
            args: &ARGS,
        };
        let sender = sender.clone();
        let day = func.day;
        let f = func.function;

        pool.spawn(move || {
            match input(year, day).open() {
                Ok(input) => {
                    let did_panic = catch_unwind(|| f(&input, bench));
                    match did_panic {
                        Ok(Ok(_)) => {}
                        Ok(Err(e)) => {
                            sender
                                .send(BenchEvent::Error {
                                    err: e.to_string(),
                                    id,
                                })
                                .expect("Unable to send error");
                        }
                        Err(_) => {
                            sender
                                .send(BenchEvent::Error {
                                    err: "Bench setup panicked!".to_owned(),
                                    id,
                                })
                                .expect("Unable to send error");
                        }
                    }
                }
                Err(BenchError::InputFileError { inner, name }) => {
                    sender
                        .send(BenchEvent::Error {
                            err: format!("{}: {:?}", name, inner.kind()),
                            id,
                        })
                        .expect("Unable to send error");
                }
                Err(_) => unreachable!(), // InputFile::open only returns one error variant.
            }

            sender
                .send(BenchEvent::Finish { id })
                .expect("Unable to send finish");
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

    let (time_sender, time_receiver) = crossbeam_channel::unbounded();
    // We don't want to spawn the handler thread in the worker pool, because the benchmarking will
    // hog the pool's threads, meaning the UI updates won't happen in a timely manner.
    // Rayon's scope function seems to end up in the pool, so we need to make sure we get a new thread.
    // We also need both this thread and the handler thread to have access to funcs, but spawn needs
    // 'static. In the words of Jon Hoo, this makes me sad...
    let funcs = Arc::new(Mutex::new(funcs));
    let handler_thread = thread::spawn({
        let funcs = funcs.clone();
        move || {
            let mut funcs = funcs.lock().unwrap();
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
        }
    });

    multi_bars
        .join_and_clear()
        .expect("Failed to join progress bars");
    handler_thread
        .join()
        .expect("Failed to join handler thread");
    tick_thread.join().expect("Failed to join tick thread");

    // Now we've finished, to clear up a render bug when the parts finish rapidly
    // we'll re-render on stdout.
    let funcs = funcs.lock().unwrap();
    for func in &*funcs {
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

fn print_header() {
    if ARGS.run_type == RunType::Run {
        println!("   Day | {:<30} ", "Answer");
        println!("_______|_{0:_<30}", "");
    } else {
        println!("   Day | {:<30} | {:<10} | Max Mem.", "Answer", "Time");
        println!("_______|_{0:_<30}_|_{0:_<10}_|______________", "");
    }
}

fn print_footer(total_time: Duration) {
    if ARGS.run_type == RunType::Simple {
        let prec = get_precision(total_time);
        println!("_______|_{0:_<30}_|_{0:_<10}_|______________", "");
        println!(
            " Total Time: {:26} | {:.prec$?}",
            "",
            total_time,
            prec = prec
        );
    } else {
        println!("_______|_{0:_<30}", "");
    }
}

pub fn run_simple_bench(alloc: &'static TracingAlloc, year: u16, days: &[Day]) -> BenchResult {
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

    print_header();

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
    let (rows, cols) = stdout.size();
    // Add room for header and trailing line.
    let rows = rows.saturating_sub(5) as usize;

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

    let total_time = benched_functions
        .into_iter()
        .map(|days_chunk| bench_days_chunk(alloc, year, days_chunk, &spinner_style, &pool))
        .try_fold(Duration::ZERO, |acc, a| a.map(|a| a + acc))?;

    print_footer(total_time);

    Ok(())
}
