use std::{ffi::CString, fmt::Write as _, fs, io::Write as _, path::Path, time::SystemTime};

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{
    outer::{Capabilities, ColorLevel, Profile},
    session::{PaneId, Rect, Session, Size},
    terminal::{Attributes, Cell, Color, Terminal},
};

pub type Snapshot = Vec<(PaneId, Vec<Vec<Cell>>)>;

#[derive(Clone, Copy)]
pub enum View {
    Live,
    Scroll(usize),
    Help { offset: usize, prefix: u8 },
}

#[derive(Clone, Copy)]
pub struct StatusLine<'a> {
    pub date_format: &'a str,
    pub time_format: &'a str,
    pub format: &'a str,
    pub label: &'a str,
    pub metrics: &'a Metrics,
    pub foreground: Color,
    pub background: Color,
    pub label_foreground: Color,
    pub label_background: Color,
    pub active_foreground: Color,
    pub active_background: Color,
}

#[derive(Default)]
pub struct Metrics {
    cpu_sample: Option<(u64, u64)>,
    cpu_usage: Option<u8>,
    memory_usage: Option<u8>,
    cpu_temperature: Option<i64>,
}

impl Metrics {
    pub fn refresh(&mut self, temperature_path: Option<&Path>) {
        if let Some(sample) = read_cpu_sample() {
            self.record_cpu_sample(sample);
        }
        self.memory_usage = read_memory_usage();
        self.cpu_temperature = temperature_path
            .and_then(|path| fs::read_to_string(path).ok())
            .and_then(|value| value.trim().parse::<i64>().ok())
            .map(|value| {
                if value.abs() >= 1_000 {
                    value / 1_000
                } else {
                    value
                }
            });
    }

    fn cpu_usage(&self) -> String {
        self.cpu_usage
            .map_or_else(|| "-".into(), |value| value.to_string())
    }

    fn memory_usage(&self) -> String {
        self.memory_usage
            .map_or_else(|| "-".into(), |value| value.to_string())
    }

    fn cpu_temperature(&self) -> String {
        self.cpu_temperature
            .map_or_else(|| "-".into(), |value| format!("{value}C"))
    }

    fn record_cpu_sample(&mut self, (total, idle): (u64, u64)) {
        if let Some((previous_total, previous_idle)) = self.cpu_sample {
            let elapsed = total.saturating_sub(previous_total);
            let idle = idle.saturating_sub(previous_idle);
            if let Some(usage) = elapsed
                .saturating_sub(idle)
                .saturating_mul(100)
                .checked_div(elapsed)
            {
                self.cpu_usage = Some(usage.min(100) as u8);
            }
        }
        self.cpu_sample = Some((total, idle));
    }
}

pub fn clock_key(now: SystemTime, seconds: bool) -> u64 {
    let value = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if seconds { value } else { value / 60 }
}

pub fn full(
    session: &Session,
    panes: &[(PaneId, &Terminal)],
    size: Size,
    status_line: StatusLine<'_>,
    message: Option<&str>,
    view: View,
    capabilities: Capabilities,
) -> Vec<u8> {
    let mut output = if capabilities.cursor_visibility {
        b"\x1b[?25l\x1b[H\x1b[2J".to_vec()
    } else {
        b"\x1b[H\x1b[2J".to_vec()
    };
    let content_rows = size.rows.saturating_sub(1);
    let rects = session.pane_rects(Size {
        columns: size.columns.max(1),
        rows: content_rows.max(1),
    });
    let active = session.active_pane();
    let mut attributes = None;

    if let View::Help { offset, prefix } = view {
        let lines = help_lines(prefix);
        for y in 0..content_rows {
            move_cursor(&mut output, y, 0);
            set_attributes(
                &mut output,
                &mut attributes,
                Attributes::default(),
                capabilities,
            );
            let line = lines
                .get(offset + usize::from(y))
                .map_or("", String::as_str);
            let line = truncate(line, usize::from(size.columns));
            output.extend_from_slice(line.as_bytes());
            output.extend(std::iter::repeat_n(
                b' ',
                usize::from(size.columns).saturating_sub(text_width(&line)),
            ));
        }
        output.extend_from_slice(&status(
            session,
            size,
            status_line,
            message,
            false,
            capabilities,
        ));
        return output;
    }

    for y in 0..content_rows {
        move_cursor(&mut output, y, 0);
        for x in 0..size.columns {
            if let Some((pane, rect)) = rects.iter().find(|(_, rect)| contains(*rect, x, y))
                && let Some((_, terminal)) = panes.iter().find(|(id, _)| id == pane)
            {
                let offset = match view {
                    View::Scroll(offset) if Some(*pane) == active => offset,
                    _ => 0,
                };
                let cell =
                    &terminal.view_row(offset, usize::from(y - rect.y))[usize::from(x - rect.x)];
                set_attributes(
                    &mut output,
                    &mut attributes,
                    cell.attributes(),
                    capabilities,
                );
                if !cell.is_continuation() {
                    push_char(&mut output, cell.character());
                    output.extend_from_slice(cell.combining().as_bytes());
                }
                continue;
            }

            set_attributes(
                &mut output,
                &mut attributes,
                Attributes::default(),
                capabilities,
            );
            push_char(
                &mut output,
                border(
                    &rects,
                    active,
                    x,
                    y,
                    !matches!(
                        capabilities.profile,
                        Profile::Dumb | Profile::Ansi | Profile::Vt100
                    ),
                ),
            );
        }
    }

    output.extend_from_slice(&status(
        session,
        size,
        status_line,
        message,
        false,
        capabilities,
    ));
    place_cursor(
        &mut output,
        active.filter(|_| matches!(view, View::Live)),
        &rects,
        panes,
        capabilities,
    );
    output
}

pub fn help_max_offset(rows: u16, prefix: u8) -> usize {
    help_lines(prefix)
        .len()
        .saturating_sub(usize::from(rows.saturating_sub(1)))
}

fn help_lines(prefix: u8) -> Vec<String> {
    let prefix = format!("Ctrl-{}", char::from(prefix + b'a' - 1));
    [
        "Termfold key reminder".to_string(),
        String::new(),
        format!("{prefix} {prefix}       = Send the prefix to the active application"),
        format!("{prefix} c            = Create tab"),
        format!("{prefix} n / p        = Next / previous tab"),
        format!("{prefix} 1..9 / 0     = Select tab 1..10"),
        format!("{prefix} | / -        = Split left/right / top/bottom"),
        format!("{prefix} Arrow        = Focus adjacent pane"),
        format!("{prefix} Ctrl-Arrow   = Resize pane by one cell"),
        format!("{prefix} r            = Enter resize mode"),
        format!("{prefix} x            = Close active pane"),
        format!("{prefix} d            = Detach"),
        format!("{prefix} [            = Enter scroll mode"),
        format!("{prefix} C            = Clear retained scrollback"),
        format!("{prefix} S            = Save retained scrollback"),
        format!("{prefix} ?            = Show this key reminder"),
        String::new(),
        "Help: Up/Down or k/j = line, Page Up/Page Down = page, q/Esc = exit".to_string(),
    ]
    .into()
}

pub fn snapshot(panes: &[(PaneId, &Terminal)]) -> Snapshot {
    panes
        .iter()
        .map(|(pane, terminal)| (*pane, terminal.screen().rows().to_vec()))
        .collect()
}

pub fn changes(
    session: &Session,
    panes: &[(PaneId, &Terminal)],
    size: Size,
    previous: &Snapshot,
    capabilities: Capabilities,
) -> Vec<u8> {
    let rects = session.pane_rects(Size {
        columns: size.columns.max(1),
        rows: size.rows.saturating_sub(1).max(1),
    });
    let mut output = if capabilities.cursor_visibility {
        b"\x1b[?25l".to_vec()
    } else {
        Vec::new()
    };
    let mut attributes = None;

    for (pane, terminal) in panes {
        let Some((_, rect)) = rects.iter().find(|(id, _)| id == pane) else {
            continue;
        };
        let rows = terminal.screen().rows();
        let old = previous.iter().find(|(id, _)| id == pane);
        for (y, row) in rows.iter().enumerate() {
            for (x, cell) in row.iter().enumerate() {
                if old.and_then(|(_, rows)| rows.get(y).and_then(|row| row.get(x))) == Some(cell) {
                    continue;
                }
                if cell.is_continuation() {
                    continue;
                }
                move_cursor(
                    &mut output,
                    rect.y.saturating_add(y as u16),
                    rect.x.saturating_add(x as u16),
                );
                set_attributes(
                    &mut output,
                    &mut attributes,
                    cell.attributes(),
                    capabilities,
                );
                push_char(&mut output, cell.character());
                output.extend_from_slice(cell.combining().as_bytes());
            }
        }
    }

    output.extend_from_slice(b"\x1b[0m");
    place_cursor(
        &mut output,
        session.active_pane(),
        &rects,
        panes,
        capabilities,
    );
    output
}

pub fn status(
    session: &Session,
    size: Size,
    status_line: StatusLine<'_>,
    message: Option<&str>,
    preserve_cursor: bool,
    capabilities: Capabilities,
) -> Vec<u8> {
    let width = usize::from(size.columns);
    let (date, time) = format_clock(status_line.date_format, status_line.time_format);
    let active = session.active_tab().unwrap_or(0);
    let (segments, _) = message.map_or_else(
        || {
            status_segments(
                session.name(),
                session.tab_count(),
                active,
                &date,
                &time,
                status_line,
                width,
            )
        },
        |message| message_segments(session.name(), active, message, width),
    );
    let mut output = Vec::new();
    if preserve_cursor {
        output.extend_from_slice(b"\x1b7");
    }
    move_cursor(&mut output, size.rows.saturating_sub(1), 0);
    let mut attributes = None;
    let mut used = 0;
    for segment in segments {
        let marked = segment.tab == Some(active);
        let (foreground, background) = match segment.style {
            SegmentStyle::Base | SegmentStyle::Fill => {
                (status_line.foreground, status_line.background)
            }
            SegmentStyle::Label => (status_line.label_foreground, status_line.label_background),
            SegmentStyle::Active => (status_line.active_foreground, status_line.active_background),
        };
        set_attributes(
            &mut output,
            &mut attributes,
            Attributes {
                foreground,
                background,
                bold: marked,
                underline: marked,
                ..Attributes::default()
            },
            capabilities,
        );
        output.extend_from_slice(segment.text.as_bytes());
        used += text_width(&segment.text);
    }
    set_attributes(
        &mut output,
        &mut attributes,
        Attributes {
            foreground: status_line.foreground,
            background: status_line.background,
            ..Attributes::default()
        },
        capabilities,
    );
    output.extend(std::iter::repeat_n(b' ', width.saturating_sub(used)));
    output.extend_from_slice(b"\x1b[0m");
    if preserve_cursor {
        output.extend_from_slice(b"\x1b8");
    }
    output
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum SegmentStyle {
    Base,
    Fill,
    Label,
    Active,
}

struct Segment {
    text: String,
    tab: Option<usize>,
    style: SegmentStyle,
}

fn segment(text: String, tab: Option<usize>, style: SegmentStyle) -> Segment {
    Segment { text, tab, style }
}

fn message_segments(
    session: &str,
    active: usize,
    message: &str,
    width: usize,
) -> (Vec<Segment>, usize) {
    let active_text = format!("[{}:shell]", active + 1);
    let session = format!("[{session}]");
    let mut result = Vec::new();
    let mut used = 0;
    for (text, marked, style) in [
        (active_text, Some(active), SegmentStyle::Active),
        (format!("  {session}"), None, SegmentStyle::Base),
        (format!("  {message}"), None, SegmentStyle::Base),
    ] {
        let text = truncate(&text, width.saturating_sub(used));
        used += text_width(&text);
        result.push(segment(text, marked, style));
        if used == width {
            break;
        }
    }
    (result, used)
}

fn status_segments(
    session: &str,
    tab_count: usize,
    active: usize,
    date: &str,
    time: &str,
    status_line: StatusLine<'_>,
    width: usize,
) -> (Vec<Segment>, usize) {
    let mut first = 0;
    let mut last = tab_count.saturating_sub(1);

    loop {
        let tabs = tab_segments(tab_count, active, first, last);
        let mut result = expand_status(
            status_line.format,
            session,
            tabs,
            status_line.label,
            date,
            time,
            status_line.metrics,
        );
        let used = segments_width(&result);
        if used <= width {
            let padding = width - used;
            if let Some(fill) = result
                .iter_mut()
                .find(|segment| segment.style == SegmentStyle::Fill)
            {
                fill.text = " ".repeat(padding);
            }
            return (result, width);
        }
        let left_distance = active.saturating_sub(first);
        let right_distance = last.saturating_sub(active);
        if right_distance >= left_distance && last > active {
            last -= 1;
        } else if first < active {
            first += 1;
        } else {
            break;
        }
    }

    let mut result = Vec::new();
    let mut used = 0;
    for (text, marked, style) in [
        (
            format!("[{}:shell]", active + 1),
            Some(active),
            SegmentStyle::Active,
        ),
        (format!("  {time}"), None, SegmentStyle::Base),
        (format!("  [{session}]"), None, SegmentStyle::Base),
    ] {
        let remaining = width.saturating_sub(used);
        if remaining == 0 {
            break;
        }
        let text = truncate(&text, remaining);
        used += text_width(&text);
        result.push(segment(text, marked, style));
    }
    (result, used)
}

fn expand_status(
    format: &str,
    session: &str,
    tabs: Vec<Segment>,
    label: &str,
    date: &str,
    time: &str,
    metrics: &Metrics,
) -> Vec<Segment> {
    let mut result = Vec::new();
    let mut rest = format;
    while let Some(start) = rest.find('{') {
        if start > 0 {
            result.push(segment(rest[..start].into(), None, SegmentStyle::Base));
        }
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else {
            break;
        };
        match &after[..end] {
            "session" => result.push(segment(session.into(), None, SegmentStyle::Base)),
            "tabs" => result.extend(tabs.iter().map(|tab| Segment {
                text: tab.text.clone(),
                tab: tab.tab,
                style: tab.style,
            })),
            "fill" => result.push(segment(String::new(), None, SegmentStyle::Fill)),
            "label" => result.push(segment(label.into(), None, SegmentStyle::Label)),
            "date" => result.push(segment(date.into(), None, SegmentStyle::Base)),
            "time" => result.push(segment(time.into(), None, SegmentStyle::Base)),
            "cpu_usage" => result.push(segment(metrics.cpu_usage(), None, SegmentStyle::Base)),
            "memory_usage" => {
                result.push(segment(metrics.memory_usage(), None, SegmentStyle::Base))
            }
            "cpu_temp" => result.push(segment(metrics.cpu_temperature(), None, SegmentStyle::Base)),
            _ => {}
        }
        rest = &after[end + 1..];
    }
    if !rest.is_empty() {
        result.push(segment(rest.into(), None, SegmentStyle::Base));
    }
    result
}

fn tab_segments(count: usize, active: usize, first: usize, last: usize) -> Vec<Segment> {
    let mut result = Vec::new();
    if first > 0 {
        result.push(segment("< ".into(), None, SegmentStyle::Base));
    }
    for index in first..=last {
        if index > first {
            result.push(segment("  ".into(), None, SegmentStyle::Base));
        }
        result.push(segment(
            if index == active {
                format!("[{}:shell]", index + 1)
            } else {
                format!("{}:shell", index + 1)
            },
            Some(index),
            if index == active {
                SegmentStyle::Active
            } else {
                SegmentStyle::Base
            },
        ));
    }
    if last + 1 < count {
        result.push(segment(" >".into(), None, SegmentStyle::Base));
    }
    result
}

pub fn status_tab_at(
    session: &Session,
    size: Size,
    status_line: StatusLine<'_>,
    message: Option<&str>,
    x: u16,
) -> Option<usize> {
    let active = session.active_tab()?;
    let (date, time) = format_clock(status_line.date_format, status_line.time_format);
    let (segments, _) = message.map_or_else(
        || {
            status_segments(
                session.name(),
                session.tab_count(),
                active,
                &date,
                &time,
                status_line,
                usize::from(size.columns),
            )
        },
        |message| message_segments(session.name(), active, message, usize::from(size.columns)),
    );
    let mut start = 0;
    for segment in segments {
        let end = start + text_width(&segment.text);
        if (start..end).contains(&usize::from(x)) {
            return segment.tab;
        }
        start = end;
    }
    None
}

fn read_cpu_sample() -> Option<(u64, u64)> {
    let source = fs::read_to_string("/proc/stat").ok()?;
    parse_cpu_sample(&source)
}

fn parse_cpu_sample(source: &str) -> Option<(u64, u64)> {
    let mut values = source.lines().next()?.split_whitespace();
    (values.next()? == "cpu").then_some(())?;
    let values = values
        .map(str::parse::<u64>)
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    let total = values.iter().copied().sum();
    let idle = values.get(3).copied().unwrap_or(0) + values.get(4).copied().unwrap_or(0);
    Some((total, idle))
}

fn read_memory_usage() -> Option<u8> {
    let source = fs::read_to_string("/proc/meminfo").ok()?;
    parse_memory_usage(&source)
}

fn parse_memory_usage(source: &str) -> Option<u8> {
    let mut total = None;
    let mut available = None;
    for line in source.lines() {
        let (name, value) = line.split_once(':')?;
        let value = value.split_whitespace().next()?.parse::<u64>().ok()?;
        match name {
            "MemTotal" => total = Some(value),
            "MemAvailable" => available = Some(value),
            _ => {}
        }
        if total.is_some() && available.is_some() {
            break;
        }
    }
    let total = total?;
    (total > 0).then(|| {
        (total
            .saturating_sub(available.unwrap_or_default())
            .saturating_mul(100)
            / total)
            .min(100) as u8
    })
}

fn format_clock(date_format: &str, time_format: &str) -> (String, String) {
    let Ok(format) = CString::new(format!("{date_format}\x1f{time_format}")) else {
        return (String::new(), String::new());
    };
    let mut now = 0;
    let mut local = std::mem::MaybeUninit::<libc::tm>::uninit();
    // SAFETY: time writes to a valid time_t and localtime_r initializes local.
    unsafe {
        libc::time(&raw mut now);
        if libc::localtime_r(&now, local.as_mut_ptr()).is_null() {
            return (String::new(), String::new());
        }
    }
    let mut output = [0_u8; 512];
    // SAFETY: all pointers reference initialized, correctly sized storage.
    let length = unsafe {
        libc::strftime(
            output.as_mut_ptr().cast(),
            output.len(),
            format.as_ptr(),
            local.as_ptr(),
        )
    };
    let output = String::from_utf8_lossy(&output[..length]);
    let (date, time) = output.split_once('\x1f').unwrap_or_default();
    (date.into(), time.into())
}

fn contains(rect: Rect, x: u16, y: u16) -> bool {
    x >= rect.x
        && x < rect.x.saturating_add(rect.width)
        && y >= rect.y
        && y < rect.y.saturating_add(rect.height)
}

fn border(rects: &[(PaneId, Rect)], active: Option<PaneId>, x: u16, y: u16, unicode: bool) -> char {
    let neighbor = |x, y| rects.iter().find(|(_, rect)| contains(*rect, x, y));
    let left = x.checked_sub(1).and_then(|x| neighbor(x, y));
    let right = neighbor(x.saturating_add(1), y);
    let above = y.checked_sub(1).and_then(|y| neighbor(x, y));
    let below = neighbor(x, y.saturating_add(1));
    let active = [left, right, above, below]
        .into_iter()
        .flatten()
        .any(|(pane, _)| Some(*pane) == active);
    let shape = (
        left.is_some() || right.is_some(),
        above.is_some() || below.is_some(),
    );
    if unicode {
        match (shape, active) {
            ((true, true), true) => '╋',
            ((true, false), true) => '┃',
            ((false, true), true) => '━',
            ((true, true), false) => '┼',
            ((true, false), false) => '│',
            ((false, true), false) => '─',
            ((false, false), _) => ' ',
        }
    } else if active {
        '#'
    } else {
        match shape {
            (true, true) => '+',
            (true, false) => '|',
            (false, true) => '-',
            (false, false) => ' ',
        }
    }
}

fn set_attributes(
    output: &mut Vec<u8>,
    current: &mut Option<Attributes>,
    wanted: Attributes,
    capabilities: Capabilities,
) {
    let wanted = adapt_attributes(wanted, capabilities);
    if *current == Some(wanted) {
        return;
    }
    *current = Some(wanted);
    let mut sequence = String::from("\x1b[0");
    for (enabled, code) in [
        (wanted.bold, 1),
        (wanted.faint, 2),
        (wanted.italic, 3),
        (wanted.underline, 4),
        (wanted.blink, 5),
        (wanted.inverse, 7),
        (wanted.hidden, 8),
        (wanted.strike, 9),
    ] {
        if enabled {
            let _ = write!(sequence, ";{code}");
        }
    }
    push_color(&mut sequence, wanted.foreground, 38, 30, 90);
    push_color(&mut sequence, wanted.background, 48, 40, 100);
    push_color(&mut sequence, wanted.underline_color, 58, 0, 0);
    sequence.push('m');
    output.extend_from_slice(sequence.as_bytes());
}

fn adapt_attributes(mut wanted: Attributes, capabilities: Capabilities) -> Attributes {
    if capabilities.color == ColorLevel::Monochrome {
        wanted.bold |= wanted.foreground != Color::Default;
        wanted.inverse |= wanted.background != Color::Default;
    }
    wanted.foreground = adapt_color(wanted.foreground, capabilities.color);
    wanted.background = adapt_color(wanted.background, capabilities.color);
    wanted.underline_color = if capabilities.color >= ColorLevel::Indexed256 {
        adapt_color(wanted.underline_color, capabilities.color)
    } else {
        Color::Default
    };
    wanted.faint &= capabilities.faint;
    wanted.italic &= capabilities.italic;
    wanted.blink &= capabilities.blink;
    wanted.strike &= capabilities.strike;
    wanted
}

fn adapt_color(color: Color, level: ColorLevel) -> Color {
    match (color, level) {
        (Color::Default, _) | (_, ColorLevel::Monochrome) => Color::Default,
        (Color::Indexed(index @ 0..=15), ColorLevel::Ansi16) => Color::Indexed(index),
        (Color::Indexed(index), ColorLevel::Ansi16) => {
            let (red, green, blue) = indexed_rgb(index);
            Color::Indexed(rgb_ansi(red, green, blue))
        }
        (Color::Rgb(red, green, blue), ColorLevel::Ansi16) => {
            Color::Indexed(rgb_ansi(red, green, blue))
        }
        (Color::Rgb(red, green, blue), ColorLevel::Indexed256) => {
            Color::Indexed(16 + 36 * cube(red) + 6 * cube(green) + cube(blue))
        }
        (color, ColorLevel::Indexed256 | ColorLevel::TrueColor) => color,
    }
}

fn cube(value: u8) -> u8 {
    ((u16::from(value) * 5 + 127) / 255) as u8
}

fn rgb_ansi(red: u8, green: u8, blue: u8) -> u8 {
    let bright = u8::from(red.max(green).max(blue) >= 192) * 8;
    bright + u8::from(red >= 128) + 2 * u8::from(green >= 128) + 4 * u8::from(blue >= 128)
}

fn indexed_rgb(index: u8) -> (u8, u8, u8) {
    if index >= 232 {
        let value = 8 + (index - 232) * 10;
        return (value, value, value);
    }
    let index = index.saturating_sub(16);
    let component = |value: u8| if value == 0 { 0 } else { 55 + value * 40 };
    (
        component(index / 36),
        component(index % 36 / 6),
        component(index % 6),
    )
}

fn push_color(output: &mut String, color: Color, extended: u8, normal: u8, bright: u8) {
    match color {
        Color::Default => {}
        Color::Indexed(index @ 0..=7) if normal != 0 => {
            let _ = write!(output, ";{}", normal + index);
        }
        Color::Indexed(index @ 8..=15) if bright != 0 => {
            let _ = write!(output, ";{}", bright + index - 8);
        }
        Color::Indexed(index) => {
            let _ = write!(output, ";{extended};5;{index}");
        }
        Color::Rgb(red, green, blue) => {
            let _ = write!(output, ";{extended};2;{red};{green};{blue}");
        }
    }
}

fn move_cursor(output: &mut Vec<u8>, row: u16, column: u16) {
    let _ = write!(output, "\x1b[{};{}H", row + 1, column + 1);
}

fn place_cursor(
    output: &mut Vec<u8>,
    active: Option<PaneId>,
    rects: &[(PaneId, Rect)],
    panes: &[(PaneId, &Terminal)],
    capabilities: Capabilities,
) {
    let Some(active) = active else {
        return;
    };
    let Some((_, rect)) = rects.iter().find(|(pane, _)| *pane == active) else {
        return;
    };
    let Some((_, terminal)) = panes.iter().find(|(pane, _)| *pane == active) else {
        return;
    };
    let cursor = terminal.screen().cursor();
    move_cursor(
        output,
        rect.y.saturating_add(cursor.row as u16),
        rect.x.saturating_add(cursor.column as u16),
    );
    if capabilities.cursor_visibility {
        output.extend_from_slice(if terminal.modes().cursor_visible {
            b"\x1b[?25h"
        } else {
            b"\x1b[?25l"
        });
    }
}

fn push_char(output: &mut Vec<u8>, character: char) {
    let mut bytes = [0; 4];
    output.extend_from_slice(character.encode_utf8(&mut bytes).as_bytes());
}

fn text_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

fn segments_width(segments: &[Segment]) -> usize {
    segments
        .iter()
        .map(|segment| text_width(&segment.text))
        .sum()
}

fn truncate(text: &str, width: usize) -> String {
    let mut used = 0;
    text.chars()
        .take_while(|character| {
            let next = character.width().unwrap_or(0);
            if used + next > width {
                false
            } else {
                used += next;
                true
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capabilities() -> Capabilities {
        crate::outer::select("auto", "xterm-256color", "truecolor").capabilities
    }

    fn status_line(metrics: &Metrics) -> StatusLine<'_> {
        StatusLine {
            date_format: "%Y-%m-%d",
            time_format: "%H:%M",
            format: "[{session}]  {tabs}{fill}|  {date} {time}",
            label: "",
            metrics,
            foreground: Color::Indexed(0),
            background: Color::Indexed(6),
            label_foreground: Color::Indexed(15),
            label_background: Color::Indexed(1),
            active_foreground: Color::Indexed(0),
            active_background: Color::Indexed(11),
        }
    }

    #[test]
    fn status_keeps_active_then_time_then_session_when_narrow() {
        let metrics = Metrics::default();
        let (segments, width) = status_segments(
            "demo",
            3,
            1,
            "2026-07-19",
            "18:42",
            status_line(&metrics),
            20,
        );
        let text = segments
            .into_iter()
            .map(|segment| segment.text)
            .collect::<String>();
        assert_eq!(width, 20);
        assert_eq!(text, "[2:shell]  18:42  [d");
    }

    #[test]
    fn status_removes_furthest_inactive_tabs_first() {
        let metrics = Metrics::default();
        let (segments, _) =
            status_segments("s", 5, 2, "2026-07-19", "18:42", status_line(&metrics), 49);
        let text = segments
            .into_iter()
            .map(|segment| segment.text)
            .collect::<String>();
        assert!(text.contains("< "), "{text}");
        assert!(text.contains("[3:shell]"), "{text}");
        assert!(text.contains(" >"), "{text}");
        assert!(!text.contains("1:shell"));
        assert!(!text.contains("5:shell"));
    }

    #[test]
    fn status_fills_to_right_align_unicode_and_samples_linux_metrics() {
        let mut metrics = Metrics::default();
        metrics.record_cpu_sample((100, 40));
        metrics.record_cpu_sample((200, 70));
        assert_eq!(metrics.cpu_usage(), "70");
        assert_eq!(
            parse_memory_usage("MemTotal: 1000 kB\nMemAvailable: 250 kB\n"),
            Some(75)
        );

        let line = StatusLine {
            format: "[{session}] {tabs} │ {label}{fill}CPU {cpu_usage}% {date} {time}",
            label: "PROD",
            ..status_line(&metrics)
        };
        let (segments, width) = status_segments("s", 1, 0, "2026-07-25", "18:42", line, 60);
        let text = segments
            .into_iter()
            .map(|segment| segment.text)
            .collect::<String>();
        assert_eq!(width, 60);
        assert_eq!(text_width(&text), 60);
        assert!(text.ends_with("CPU 70% 2026-07-25 18:42"), "{text}");
    }

    #[test]
    fn status_tab_hit_testing_matches_rendered_tabs() {
        let metrics = Metrics::default();
        let mut session = Session::new("s".into());
        session.create_tab().unwrap();
        let size = Size {
            columns: 80,
            rows: 24,
        };
        assert_eq!(
            status_tab_at(&session, size, status_line(&metrics), None, 5),
            Some(0)
        );
        assert_eq!(
            status_tab_at(&session, size, status_line(&metrics), None, 14),
            Some(1)
        );
    }

    #[test]
    fn pane_border_uses_box_drawing_with_ascii_fallback() {
        let mut session = Session::new("s".into());
        session
            .split_active(
                crate::session::Split::LeftRight,
                Size {
                    columns: 5,
                    rows: 2,
                },
            )
            .unwrap();
        let active = session.active_pane();
        let rects = session.pane_rects(Size {
            columns: 5,
            rows: 2,
        });
        assert_eq!(border(&rects, active, 2, 0, true), '┃');
        assert_eq!(border(&rects, None, 2, 0, true), '│');
        assert_eq!(border(&rects, active, 2, 0, false), '#');
        assert_eq!(border(&rects, None, 2, 0, false), '|');
    }

    #[test]
    fn full_render_includes_pane_content_and_status() {
        let metrics = Metrics::default();
        let session = Session::new("s".into());
        let pane = session.active_pane().unwrap();
        let mut terminal = Terminal::new(Size {
            columns: 40,
            rows: 2,
        })
        .unwrap();
        terminal.advance(b"hello");
        let output = full(
            &session,
            &[(pane, &terminal)],
            Size {
                columns: 40,
                rows: 3,
            },
            status_line(&metrics),
            None,
            View::Live,
            capabilities(),
        );
        let output = String::from_utf8_lossy(&output);
        assert!(output.contains("hello"));
        assert!(output.contains("[s]"));
        assert!(output.contains("[1:shell]"));
    }

    #[test]
    fn help_uses_the_configured_prefix_and_pages() {
        let metrics = Metrics::default();
        let session = Session::new("s".into());
        let output = full(
            &session,
            &[],
            Size {
                columns: 80,
                rows: 8,
            },
            status_line(&metrics),
            Some("HELP 1/3  Up/Down j/k PgUp/PgDn q/Esc"),
            View::Help {
                offset: 0,
                prefix: 1,
            },
            capabilities(),
        );
        let output = String::from_utf8_lossy(&output);
        assert!(output.contains("Ctrl-a c"));
        assert!(output.contains("= Create tab"));
        assert!(output.contains("HELP 1/3"));
        assert!(help_max_offset(8, 1) > 0);
    }

    #[test]
    fn content_change_updates_its_cell_without_clearing() {
        let session = Session::new("s".into());
        let pane = session.active_pane().unwrap();
        let mut terminal = Terminal::new(Size {
            columns: 10,
            rows: 2,
        })
        .unwrap();
        let previous = snapshot(&[(pane, &terminal)]);
        terminal.advance(b"\x1b[2;3HX");

        let output = changes(
            &session,
            &[(pane, &terminal)],
            Size {
                columns: 10,
                rows: 3,
            },
            &previous,
            capabilities(),
        );

        assert!(!output.windows(4).any(|bytes| bytes == b"\x1b[2J"));
        assert!(output.windows(6).any(|bytes| bytes == b"\x1b[2;3H"));
        assert!(output.contains(&b'X'));
    }

    #[test]
    fn multiline_output_restores_the_application_cursor() {
        let session = Session::new("s".into());
        let pane = session.active_pane().unwrap();
        let mut terminal = Terminal::new(Size {
            columns: 20,
            rows: 4,
        })
        .unwrap();
        let previous = snapshot(&[(pane, &terminal)]);
        terminal.advance(b"$ command\r\nresult\r\n$ ");

        let output = changes(
            &session,
            &[(pane, &terminal)],
            Size {
                columns: 20,
                rows: 5,
            },
            &previous,
            capabilities(),
        );

        assert!(output.ends_with(b"\x1b[3;3H\x1b[?25h"));
    }

    #[test]
    fn colors_and_attributes_downgrade_without_extended_sequences() {
        let mut output = Vec::new();
        let mut current = None;
        let capabilities =
            crate::outer::select("linux", "xterm-256color", "truecolor").capabilities;
        set_attributes(
            &mut output,
            &mut current,
            Attributes {
                foreground: Color::Rgb(255, 0, 0),
                italic: true,
                ..Attributes::default()
            },
            capabilities,
        );
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains(";91m"), "{output:?}");
        assert!(!output.contains(";3;"), "{output:?}");
        assert!(!output.contains(";2;"), "{output:?}");
    }
}
