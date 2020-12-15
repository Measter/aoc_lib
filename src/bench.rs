use std::{
    fmt::Display,
    io::{Read, Seek, SeekFrom},
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

struct RuntimeData {
    total_runs: u32,
    min_run: Duration,
    mean_run: Duration,
    max_run: Duration,
}

struct MemoryData {
    end_ts: u128,
    graph_points: Vec<(f64, f64)>,
    max_memory: usize,
}

pub struct BenchResult<T> {
    result: T,
    runtime: RuntimeData,
    memory: Option<MemoryData>,
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

    let min_prec = get_precision(part1_result.runtime.min_run);
    let mean_prec = get_precision(part1_result.runtime.mean_run);
    let max_prec = get_precision(part1_result.runtime.max_run);
    let total_runs = if part1_result.runtime.total_runs < 1000 {
        part1_result.runtime.total_runs.to_string()
    } else {
        human_format::Formatter::new().format(part1_result.runtime.total_runs as f64)
    };

    let part1_results = [
        "Part 1".to_owned(),
        part1_result.result.to_string(),
        total_runs,
        format!(
            "{:.min_prec$?}",
            part1_result.runtime.min_run,
            min_prec = min_prec
        ),
        format!(
            "{:.mean_prec$?}",
            part1_result.runtime.mean_run,
            mean_prec = mean_prec
        ),
        format!(
            "{:.max_prec$?}",
            part1_result.runtime.max_run,
            max_prec = max_prec
        ),
    ];

    let min_prec = get_precision(part2_result.runtime.min_run);
    let mean_prec = get_precision(part2_result.runtime.mean_run);
    let max_prec = get_precision(part2_result.runtime.max_run);
    let total_runs = if part2_result.runtime.total_runs < 1000 {
        part2_result.runtime.total_runs.to_string()
    } else {
        human_format::Formatter::new().format(part2_result.runtime.total_runs as f64)
    };

    let part2_results = [
        "Part 2".to_owned(),
        part2_result.result.to_string(),
        total_runs,
        format!(
            "{:.min_prec$?}",
            part2_result.runtime.min_run,
            min_prec = min_prec
        ),
        format!(
            "{:.mean_prec$?}",
            part2_result.runtime.mean_run,
            mean_prec = mean_prec
        ),
        format!(
            "{:.max_prec$?}",
            part2_result.runtime.max_run,
            max_prec = max_prec
        ),
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
    part1_data: Option<&MemoryData>,
    part2_data: Option<&MemoryData>,
    mut chunk: Rect,
) {
    let max_x = part1_data
        .map(|d| d.end_ts)
        .unwrap_or_default()
        .max(part2_data.map(|d| d.end_ts).unwrap_or_default());
    let end_ts = Duration::from_nanos(max_x as u64);
    let max_x = max_x as f64;

    let max_y_p1 = part1_data
        .and_then(|d| {
            d.graph_points
                .iter()
                .map(|(_, y)| *y)
                .max_by(|a, b| a.partial_cmp(b).unwrap())
        })
        .unwrap_or(0.0);

    let max_y_p2 = part2_data
        .and_then(|d| {
            d.graph_points
                .iter()
                .map(|(_, y)| *y)
                .max_by(|a, b| a.partial_cmp(b).unwrap())
        })
        .unwrap_or(0.0);
    let max_y = max_y_p1.max(max_y_p2);

    let mut datasets = Vec::new();
    if let Some(d) = part1_data {
        datasets.push(
            Dataset::default()
                .name("Part 1")
                .marker(symbols::Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(Color::Cyan))
                .data(&d.graph_points),
        );
    }

    if let Some(d) = part2_data {
        datasets.push(
            Dataset::default()
                .name("Part 2")
                .marker(symbols::Marker::Braille)
                .graph_type(GraphType::Line)
                .style(Style::default().fg(Color::LightYellow))
                .data(&d.graph_points),
        );
    }

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

fn print_results_table<Output1, Output2>(
    name: &str,
    part1_result: &BenchResult<Output1>,
    part2_result: &BenchResult<Output2>,
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
        draw_memory_graph(
            f,
            part1_result.memory.as_ref(),
            part2_result.memory.as_ref(),
            main_chunks[1],
        );
    })?;

    Ok(())
}

fn print_results_markdown<Output1, Output2>(
    name: &str,
    part1_result: &BenchResult<Output1>,
    part2_result: &BenchResult<Output2>,
) -> Result<()>
where
    Output1: Display,
    Output2: Display,
{
    let min_prec1 = get_precision(part1_result.runtime.min_run);
    let mean_prec1 = get_precision(part1_result.runtime.mean_run);
    let max_prec1 = get_precision(part1_result.runtime.max_run);
    let total_runs1 = if part1_result.runtime.total_runs < 1000 {
        part1_result.runtime.total_runs.to_string()
    } else {
        human_format::Formatter::new().format(part1_result.runtime.total_runs as f64)
    };

    let min_prec2 = get_precision(part2_result.runtime.min_run);
    let mean_prec2 = get_precision(part2_result.runtime.mean_run);
    let max_prec2 = get_precision(part2_result.runtime.max_run);
    let total_runs2 = if part2_result.runtime.total_runs < 1000 {
        part2_result.runtime.total_runs.to_string()
    } else {
        human_format::Formatter::new().format(part2_result.runtime.total_runs as f64)
    };

    println!("## {}", name);
    println!("||Result|N. Runs|Min|Mean|Max|Peak Mem.");
    println!("|---|---|---|---|---|---|---|");
    println!(
        "|Part 1|{}|{}|{:.min_prec$?}|{:.mean_prec$?}|{:.max_prec$?}|{}|",
        part1_result.result,
        total_runs1,
        part1_result.runtime.min_run,
        part1_result.runtime.mean_run,
        part1_result.runtime.max_run,
        part1_result
            .memory
            .as_ref()
            .map(|f| ByteSize(f.max_memory as _).to_string())
            .unwrap_or("N/A".to_owned()),
        min_prec = min_prec1,
        mean_prec = mean_prec1,
        max_prec = max_prec1,
    );
    println!(
        "|Part 2|{}|{}|{:.min_prec$?}|{:.mean_prec$?}|{:.max_prec$?}|{}|",
        part2_result.result,
        total_runs2,
        part2_result.runtime.min_run,
        part2_result.runtime.mean_run,
        part2_result.runtime.max_run,
        part2_result
            .memory
            .as_ref()
            .map(|f| ByteSize(f.max_memory as _).to_string())
            .unwrap_or("N/A".to_owned()),
        min_prec = min_prec2,
        mean_prec = mean_prec2,
        max_prec = max_prec2,
    );
    Ok(())
}

pub(crate) fn print_results<Output1, Output2>(
    output_type: OutputType,
    name: &str,
    part1_result: &BenchResult<Output1>,
    part2_result: &BenchResult<Output2>,
) -> Result<()>
where
    Output1: Display,
    Output2: Display,
{
    match output_type {
        OutputType::Table => print_results_table(name, part1_result, part2_result),
        OutputType::MarkDown => print_results_markdown(name, part1_result, part2_result),
    }
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

    print!("Benching part {}", id);
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

    let total_runs = (args.bench_time as f64 / min_run.as_secs_f64())
        .ceil()
        .max(10.0)
        .min(10e6) as u32;

    let bench_time = Duration::from_secs_f64(total_runs as f64 * min_run.as_secs_f64());
    println!(" for {} seconds", bench_time.as_secs());

    if !args.no_mem {
        // Now the cache is warm, run with tracing.
        alloc.enable_tracing();
        let _ = part(input);
        alloc.disable_tracing();
    }

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
        runtime: RuntimeData {
            total_runs,
            min_run,
            mean_run,
            max_run,
        },
        memory: None,
    })
}

pub(crate) fn benchmark<Input, Output>(
    alloc: &TracingAlloc,
    args: &Args,
    file: Option<&String>,
    input: Input,
    part_id: u8,
    part: &PartFunction<Input, Output>,
) -> Result<BenchResult<Output>>
where
    Output: Display,
    Input: Copy,
{
    if !args.no_mem {
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
        let mut result = bench_function(alloc, args, part_id, input, part)?;
        let mut mem_trace = String::new();

        let mut trace_file = alloc.clear_file().unwrap(); // Should get it back.
        trace_file.seek(SeekFrom::Start(0))?;
        trace_file.read_to_string(&mut mem_trace)?;

        result.memory = Some(get_data(&mem_trace));

        Ok(result)
    } else {
        let part1_result = bench_function(alloc, args, part_id, input, part)?;
        Ok(part1_result)
    }
}
