use std::{
    io::{BufWriter, Read, Seek, SeekFrom, Write},
    time::{Duration, Instant},
};

use bytesize::ByteSize;
use color_eyre::eyre::{eyre, Context, Result};
use tui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::Span,
    widgets::{Axis, Block, Borders, Chart, Dataset, GraphType, Row, Table},
    Frame, Terminal,
};

use crate::{alloc::TracingAlloc, Args, OutputType, PartFunction};

#[derive(Default)]
struct RuntimeData {
    total_runs: u32,
    min_run: Duration,
    mean_run: Duration,
    max_run: Duration,
}

#[derive(Default)]
struct MemoryData {
    end_ts: u128,
    graph_points: Vec<(f64, f64)>,
    max_memory: usize,
}

#[derive(Default)]
pub struct BenchResult {
    pub(crate) name: &'static str,
    runtime: RuntimeData,
    memory: Option<MemoryData>,
}

impl BenchResult {
    pub(crate) fn new(name: &'static str) -> BenchResult {
        Self {
            name,
            ..Default::default()
        }
    }
}

fn get_data(trace_input: &str) -> MemoryData {
    let mut points = Vec::new();
    let mut cur_bytes = 0;
    let mut prev_bytes = 0;
    let mut end_ts = 0;
    let mut max_bytes = 0;

    for line in trace_input.lines() {
        let mut parts = line.split_whitespace().map(str::trim);

        let (size, ts): (isize, u128) = match (
            parts.next(),
            parts.next().map(str::parse),
            parts.next().map(str::parse),
        ) {
            (Some("A"), Some(Ok(ts)), Some(Ok(size))) => (size, ts),
            (Some("F"), Some(Ok(ts)), Some(Ok(size))) => (-size, ts),
            (Some("S"), Some(Ok(ts)), _) => (0, ts),
            (Some("E"), Some(Ok(ts)), _) => {
                end_ts = ts;
                (0, ts)
            }
            _ => {
                continue;
            }
        };

        cur_bytes += size;
        max_bytes = max_bytes.max(cur_bytes);

        points.push((ts as f64, prev_bytes as f64));
        points.push((ts as f64, cur_bytes as f64));

        prev_bytes = cur_bytes;
    }

    MemoryData {
        end_ts,
        graph_points: points,
        max_memory: max_bytes as usize,
    }
}

fn get_precision(val: Duration) -> usize {
    if val.as_nanos() < 1000 {
        0
    } else {
        3
    }
}

fn write_results_table<'a, B: 'a + Backend>(
    f: &mut Frame<'a, B>,
    chunk: Rect,
    results: &[(String, BenchResult)],
) {
    let headers = [" ", "Result", "N. Runs", "Min", "Mean", "Max", "Max Mem."];

    let output_results = results.iter().map(|(output, bench)| {
        let min_prec = get_precision(bench.runtime.min_run);
        let mean_prec = get_precision(bench.runtime.mean_run);
        let max_prec = get_precision(bench.runtime.max_run);
        let total_runs = if bench.runtime.total_runs < 1000 {
            bench.runtime.total_runs.to_string()
        } else {
            human_format::Formatter::new().format(bench.runtime.total_runs as f64)
        };

        let max_mem = bench.memory.as_ref().map(|m| m.max_memory).unwrap_or(0);

        Row::Data(
            vec![
                bench.name.to_owned(),
                output.to_string(),
                total_runs,
                format!("{:.min_prec$?}", bench.runtime.min_run, min_prec = min_prec),
                format!(
                    "{:.mean_prec$?}",
                    bench.runtime.mean_run,
                    mean_prec = mean_prec
                ),
                format!("{:.max_prec$?}", bench.runtime.max_run, max_prec = max_prec),
                ByteSize(max_mem as _).to_string(),
            ]
            .into_iter(),
        )
    });

    let part_results = Table::new(headers.iter(), output_results)
        .block(Block::default())
        .widths(&[
            Constraint::Length(8),
            Constraint::Percentage(100),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(12),
            Constraint::Length(12),
        ]);
    f.render_widget(part_results, chunk);
}

fn draw_memory_graph<'a, B: Backend + 'a>(
    f: &mut Frame<'a, B>,
    mut chunk: Rect,
    results: &[(String, BenchResult)],
) {
    let max_x = results
        .iter()
        .filter_map(|(_, bench)| bench.memory.as_ref())
        .map(|mem| mem.end_ts)
        .max()
        .unwrap_or_default();
    let end_ts = Duration::from_nanos(max_x as u64);
    let max_x = max_x as f64;

    let max_y = results
        .iter()
        .filter_map(|(_, bench)| bench.memory.as_ref())
        .map(|mem| mem.max_memory)
        .max()
        .unwrap_or(0) as f64;

    let colors = [
        Color::Cyan,
        Color::LightYellow,
        Color::LightRed,
        Color::LightGreen,
    ];

    let datasets: Vec<_> = results
        .iter()
        .zip(colors.iter().cycle())
        .flat_map(|((_, bench), color)| bench.memory.as_ref().map(|b| (bench.name, b, color)))
        .map(|(name, bench, color)| {
            Dataset::default()
                .name(name)
                .marker(symbols::Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(*color))
                .data(&bench.graph_points)
        })
        .collect();

    let chart_block = Block::default()
        .title(Span::styled(
            "Memory Usage",
            Style::default()
                .fg(Color::Gray)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::TOP)
        .border_style(Style::default().fg(Color::DarkGray));

    let chart = Chart::new(datasets)
        .block(chart_block)
        .x_axis(Axis::default().bounds([0.0, max_x]).labels(vec![
            Span::styled(0.to_string(), Style::default().fg(Color::Gray)),
            Span::styled(format!("{:?}", end_ts), Style::default().fg(Color::Gray)),
        ]))
        .y_axis(Axis::default().bounds([0.0, max_y]).labels(vec![
            Span::styled(ByteSize(0).to_string(), Style::default().fg(Color::Gray)),
            Span::styled(
                ByteSize(max_y as _).to_string(),
                Style::default().fg(Color::Gray),
            ),
        ]))
        .style(Style::default().fg(Color::DarkGray))
        .hidden_legend_constraints((Constraint::Ratio(1, 1), Constraint::Ratio(1, 1)));
    chunk.height += 1;
    f.render_widget(chart, chunk);
}

fn print_results_table(name: &str, results: &[(String, BenchResult)]) -> Result<()> {
    let stdout = std::io::stdout();
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    terminal.draw(|f| {
        let mut size = f.size();
        size.height -= 1;

        let block = Block::default()
            .title(Span::styled(
                name,
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray));
        let outer_size = block.inner(size);
        f.render_widget(block, size);

        let main_chunks = Layout::default()
            .direction(Direction::Vertical)
            .margin(1)
            .constraints([
                Constraint::Length(3 + results.len() as u16),
                Constraint::Percentage(100),
            ])
            .split(outer_size);

        write_results_table(f, main_chunks[0], results);
        draw_memory_graph(f, main_chunks[1], results);
    })?;

    Ok(())
}

fn print_results_markdown(name: &str, results: &[(String, BenchResult)]) -> Result<()> {
    println!("## {}", name);
    println!("||Result|N. Runs|Min|Mean|Max|Peak Mem.");
    println!("|---|---|---|---|---|---|---|");

    for (part_output, result) in results {
        let min_prec = get_precision(result.runtime.min_run);
        let mean_prec = get_precision(result.runtime.mean_run);
        let max_prec = get_precision(result.runtime.max_run);
        let total_runs = if result.runtime.total_runs < 1000 {
            result.runtime.total_runs.to_string()
        } else {
            human_format::Formatter::new().format(result.runtime.total_runs as f64)
        };

        println!(
            "|{}|{}|{}|{:.min_prec$?}|{:.mean_prec$?}|{:.max_prec$?}|{}|",
            result.name,
            part_output,
            total_runs,
            result.runtime.min_run,
            result.runtime.mean_run,
            result.runtime.max_run,
            result
                .memory
                .as_ref()
                .map(|f| ByteSize(f.max_memory as _).to_string())
                .unwrap_or("N/A".to_owned()),
            min_prec = min_prec,
            mean_prec = mean_prec,
            max_prec = max_prec,
        );
    }

    println!();
    Ok(())
}

pub(crate) fn print_results(
    output_type: OutputType,
    name: &str,
    results: &[(String, BenchResult)],
) -> Result<()> {
    match output_type {
        OutputType::Table => print_results_table(name, results),
        OutputType::MarkDown => print_results_markdown(name, results),
    }
}

fn bench_function_runtime<Output>(
    args: &Args,
    name: &str,
    func: &PartFunction<Output>,
) -> Result<RuntimeData> {
    eprint!("Benching runtime of {}", name);
    // Run a few times to get an estimate of how long it takes.
    let mut min_run = Duration::from_secs(u64::MAX);

    for _ in 0..5 {
        let now = Instant::now();
        let _ = func();
        let time = now.elapsed();

        if time < min_run {
            min_run = time;
        }
    }

    let total_runs = (args.bench_time as f64 / min_run.as_secs_f64())
        .ceil()
        .max(10.0)
        .min(10e6) as u32;

    let bench_time = Duration::from_secs_f64(total_runs as f64 * min_run.as_secs_f64());
    eprintln!(" for {} seconds", bench_time.as_secs());

    let mut total_time = Duration::default();
    let mut min_run = Duration::from_secs(u64::MAX);
    let mut max_run = Duration::default();

    for _ in 0..total_runs {
        let start = Instant::now();
        let _ = func(); // We'll just discard the result as we handled errors above.
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

    Ok(RuntimeData {
        total_runs,
        min_run,
        mean_run,
        max_run,
    })
}

fn bench_function_memory<Output>(
    alloc: &TracingAlloc,
    name: &str,
    func: &PartFunction<Output>,
) -> Result<MemoryData> {
    eprintln!("Benching memory of {}", name);
    let trace_file = tempfile::tempfile()?;

    let writer = BufWriter::new(trace_file);
    alloc.set_file(writer);

    // No need to handle an error here, we did it earlier.
    alloc.enable_tracing();
    let _ = func();
    alloc.disable_tracing();

    let mut mem_trace = String::new();

    let mut trace_writer = alloc.clear_file().unwrap(); // Should get it back.
    trace_writer.flush()?;

    let mut trace_file = trace_writer.into_inner().unwrap();
    trace_file.seek(SeekFrom::Start(0))?;
    trace_file.read_to_string(&mut mem_trace)?;

    Ok(get_data(&mem_trace))
}

pub(crate) fn benchmark<Output>(
    alloc: &TracingAlloc,
    args: &Args,
    name: &'static str,
    func: &PartFunction<Output>,
) -> Result<BenchResult> {
    let runtime = bench_function_runtime(args, name, func)
        .with_context(|| eyre!("Error benchmarking runtime of {}", name))?;

    let memory = if !args.no_mem {
        Some(bench_function_memory(alloc, name, func)?)
    } else {
        None
    };

    Ok(BenchResult {
        name,
        runtime,
        memory,
    })
}
