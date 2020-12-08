use std::{
    fmt::Display,
    io::{Read, Seek, SeekFrom},
    time::{Duration, Instant},
};

use bytesize::ByteSize;
use color_eyre::eyre::{eyre, Context, Result};

mod alloc;
pub use alloc::TracingAlloc;
use tui::{
    backend::{Backend, CrosstermBackend},
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols,
    text::Span,
    widgets::{Axis, Block, Borders, Chart, Dataset, GraphType, Row, Table},
    Frame, Terminal,
};

pub mod parsers;

type PartFunction<Input, Output> = dyn Fn(Input) -> Result<Output>;

struct Args {
    bench: bool,
    bench_mem: bool,
    part1_file: Option<String>,
    part2_file: Option<String>,
}

fn get_args() -> Args {
    let mut given_args = std::env::args().skip(1);
    let mut args = Args {
        bench: true,
        bench_mem: true,
        part1_file: None,
        part2_file: None,
    };

    while let Some(arg) = given_args.next() {
        match &*arg {
            "--nobench" => args.bench = false,
            "--nomem" => args.bench_mem = false,
            "--p1" => args.part1_file = given_args.next(),
            "--p2" => args.part2_file = given_args.next(),
            _ => {}
        }
    }

    args
}

struct BenchResult<T> {
    result: T,
    total_runs: u32,
    min_run: Duration,
    mean_run: Duration,
    max_run: Duration,
}

fn get_data(trace_input: &str) -> (u128, Vec<(f64, f64)>) {
    let mut points = Vec::new();
    let mut cur_bytes = 0;
    let mut prev_bytes = 0;
    let mut end_ts = 0;

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

        points.push((ts as f64, prev_bytes as f64));
        points.push((ts as f64, cur_bytes as f64));

        prev_bytes = cur_bytes;
    }

    (end_ts, points)
}

fn get_precision(val: Duration) -> usize {
    if val.as_nanos() < 1000 {
        0
    } else {
        3
    }
}

fn write_results_timing<'a, B: 'a, Output1, Output2>(
    f: &mut Frame<'a, B>,
    part1_result: &BenchResult<Output1>,
    part2_result: &BenchResult<Output2>,
    chunk: Rect,
) where
    B: Backend,
    Output1: Display,
    Output2: Display,
{
    let headers = [" ", "Result", "N. Runs", "Min", "Mean", "Max"];

    let min_prec = get_precision(part1_result.min_run);
    let mean_prec = get_precision(part1_result.mean_run);
    let max_prec = get_precision(part1_result.max_run);
    let total_runs = if part1_result.total_runs < 1000 {
        part1_result.total_runs.to_string()
    } else {
        human_format::Formatter::new().format(part1_result.total_runs as f64)
    };

    let part1_results = [
        "Part 1".to_owned(),
        part1_result.result.to_string(),
        total_runs,
        format!("{:.min_prec$?}", part1_result.min_run, min_prec = min_prec),
        format!(
            "{:.mean_prec$?}",
            part1_result.mean_run,
            mean_prec = mean_prec
        ),
        format!("{:.max_prec$?}", part1_result.max_run, max_prec = max_prec),
    ];

    let min_prec = get_precision(part2_result.min_run);
    let mean_prec = get_precision(part2_result.mean_run);
    let max_prec = get_precision(part2_result.max_run);
    let total_runs = if part2_result.total_runs < 1000 {
        part2_result.total_runs.to_string()
    } else {
        human_format::Formatter::new().format(part2_result.total_runs as f64)
    };

    let part2_results = [
        "Part 2".to_owned(),
        part2_result.result.to_string(),
        total_runs,
        format!("{:.min_prec$?}", part2_result.min_run, min_prec = min_prec),
        format!(
            "{:.mean_prec$?}",
            part2_result.mean_run,
            mean_prec = mean_prec
        ),
        format!("{:.max_prec$?}", part2_result.max_run, max_prec = max_prec),
    ];

    let part_results = Table::new(
        headers.iter(),
        vec![
            Row::Data(part1_results.iter()),
            Row::Data(part2_results.iter()),
        ]
        .into_iter(),
    )
    .block(Block::default())
    .widths(&[
        Constraint::Length(8),
        Constraint::Percentage(100),
        Constraint::Length(12),
        Constraint::Length(12),
        Constraint::Length(12),
        Constraint::Length(12),
    ]);
    f.render_widget(part_results, chunk);
}

fn draw_memory_graph<'a, B: Backend + 'a>(
    f: &mut Frame<'a, B>,
    part1_trace: &str,
    part2_trace: &str,
    mut chunk: Rect,
) {
    let (part1_end_ts, part1_data) = get_data(part1_trace);
    let (part2_end_ts, part2_data) = get_data(part2_trace);

    let max_x = part1_end_ts.max(part2_end_ts);
    let end_ts = Duration::from_nanos(max_x as u64);
    let max_x = max_x as f64;

    let max_y_p1 = part1_data
        .iter()
        .map(|(_, y)| *y)
        .max_by(|a, b| a.partial_cmp(b).unwrap())
        .unwrap_or(0.0);
    let max_y_p2 = part2_data
        .iter()
        .map(|(_, y)| *y)
        .max_by(|a, b| a.partial_cmp(b).unwrap())
        .unwrap_or(0.0);
    let max_y = max_y_p1.max(max_y_p2);

    let datasets = vec![
        Dataset::default()
            .name("Part 1")
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(Color::Cyan))
            .data(&part1_data),
        Dataset::default()
            .name("Part 2")
            .marker(symbols::Marker::Braille)
            .graph_type(GraphType::Line)
            .style(Style::default().fg(Color::LightYellow))
            .data(&part2_data),
    ];

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

fn print_results<Output1, Output2>(
    name: &str,
    part1_result: &BenchResult<Output1>,
    part1_trace: &str,
    part2_result: &BenchResult<Output2>,
    part2_trace: &str,
) -> Result<()>
where
    Output1: Display,
    Output2: Display,
{
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
            .constraints([Constraint::Length(5), Constraint::Percentage(100)])
            .split(outer_size);

        write_results_timing(f, part1_result, part2_result, main_chunks[0]);
        draw_memory_graph(f, part1_trace, part2_trace, main_chunks[1]);
    })?;

    Ok(())
}

fn bench_function<Input, Output>(
    alloc: &TracingAlloc,
    args: &Args,
    id: u8,
    input: Input,
    part: &PartFunction<Input, Output>,
) -> Result<BenchResult<Output>>
where
    Output: Display,
    Input: Copy,
{
    println!("Running part {}...", id);
    let part_result = part(input).with_context(|| eyre!("Error running Part {}", id))?;

    println!("Benching part {}...", id);
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

    if args.bench_mem {
        // Now the cache is warm, run with tracing.
        alloc.enable_tracing();
        let _ = part(input);
        alloc.disable_tracing();
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

    Ok(BenchResult {
        result: part_result,
        total_runs,
        min_run,
        mean_run,
        max_run,
    })
}

fn full_bench_fn<Input, Output>(
    alloc: &TracingAlloc,
    args: &Args,
    file: Option<&String>,
    input: Input,
    part_id: u8,
    part: &PartFunction<Input, Output>,
) -> Result<(String, BenchResult<Output>)>
where
    Output: Display,
    Input: Copy,
{
    if args.bench_mem {
        let trace_file = if let Some(path) = file {
            let mut fo = std::fs::OpenOptions::new();
            fo.write(true)
                .read(true)
                .truncate(true)
                .create(true)
                .open(path)?
        } else {
            tempfile::tempfile()?
        };

        alloc.set_file(trace_file);
        let result = bench_function(alloc, args, part_id, input, part)?;
        let mut mem_trace = String::new();

        let mut trace_file = alloc.clear_file().unwrap(); // Should get it back.
        trace_file.seek(SeekFrom::Start(0))?;
        trace_file.read_to_string(&mut mem_trace)?;

        Ok((mem_trace, result))
    } else {
        let part1_result = bench_function(alloc, args, part_id, input, part)?;
        Ok((String::new(), part1_result))
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
    let args = get_args();

    if !args.bench {
        println!("{}", name);

        let p1_result = part1(input).with_context(|| eyre!("Error running Part 1"))?;
        let p2_result = part2(input).with_context(|| eyre!("Error running Part 2"))?;

        println!("Part 1: {}", p1_result);
        println!("Part 2: {}", p2_result);

        return Ok(());
    }

    let (part1_trace, part1_result) = full_bench_fn(alloc, &args, args.part1_file.as_ref(), input, 1, part1)?;
    let (part2_trace, part2_result) = full_bench_fn(alloc, &args, args.part2_file.as_ref(), input, 2, part2)?;

    print_results(
        name,
        &part1_result,
        &part1_trace,
        &part2_result,
        &part2_trace,
    )?;

    Ok(())
}
