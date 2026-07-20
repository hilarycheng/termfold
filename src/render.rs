use std::{ffi::CString, fmt::Write as _, io::Write as _, time::SystemTime};

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::{
    session::{PaneId, Rect, Session, Size},
    terminal::{Attributes, Cell, Color, Terminal},
};

pub type Snapshot = Vec<(PaneId, Vec<Vec<Cell>>)>;

pub struct Clock<'a> {
    pub date_format: &'a str,
    pub time_format: &'a str,
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
    clock: Clock<'_>,
) -> Vec<u8> {
    let mut output = b"\x1b[?25l\x1b[H\x1b[2J".to_vec();
    let content_rows = size.rows.saturating_sub(1);
    let rects = session.pane_rects(Size {
        columns: size.columns.max(1),
        rows: content_rows.max(1),
    });
    let active = session.active_pane();
    let mut attributes = None;

    for y in 0..content_rows {
        move_cursor(&mut output, y, 0);
        for x in 0..size.columns {
            if let Some((pane, rect)) = rects.iter().find(|(_, rect)| contains(*rect, x, y))
                && let Some((_, terminal)) = panes.iter().find(|(id, _)| id == pane)
            {
                let cell =
                    &terminal.screen().rows()[usize::from(y - rect.y)][usize::from(x - rect.x)];
                set_attributes(&mut output, &mut attributes, cell.attributes());
                if !cell.is_continuation() {
                    push_char(&mut output, cell.character());
                    output.extend_from_slice(cell.combining().as_bytes());
                }
                continue;
            }

            set_attributes(&mut output, &mut attributes, Attributes::default());
            output.push(border(&rects, active, x, y));
        }
    }

    output.extend_from_slice(&status(
        session,
        size,
        clock.date_format,
        clock.time_format,
        false,
    ));
    place_cursor(&mut output, active, &rects, panes);
    output
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
    previous: &mut Snapshot,
) -> Vec<u8> {
    let rects = session.pane_rects(Size {
        columns: size.columns.max(1),
        rows: size.rows.saturating_sub(1).max(1),
    });
    let mut output = b"\x1b[?25l".to_vec();
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
                set_attributes(&mut output, &mut attributes, cell.attributes());
                push_char(&mut output, cell.character());
                output.extend_from_slice(cell.combining().as_bytes());
            }
        }
    }

    *previous = snapshot(panes);
    output.extend_from_slice(b"\x1b[0m");
    place_cursor(&mut output, session.active_pane(), &rects, panes);
    output
}

pub fn status(
    session: &Session,
    size: Size,
    date_format: &str,
    time_format: &str,
    preserve_cursor: bool,
) -> Vec<u8> {
    let width = usize::from(size.columns);
    let (date, time) = format_clock(date_format, time_format);
    let (segments, _) = status_segments(
        session.name(),
        session.tab_count(),
        session.active_tab().unwrap_or(0),
        &date,
        &time,
        width,
    );
    let mut output = Vec::new();
    if preserve_cursor {
        output.extend_from_slice(b"\x1b7");
    }
    move_cursor(&mut output, size.rows.saturating_sub(1), 0);
    output.extend_from_slice(b"\x1b[0;7m");
    let mut used = 0;
    for (text, active) in segments {
        output.extend_from_slice(if active {
            b"\x1b[0;1;4;7m"
        } else {
            b"\x1b[0;7m"
        });
        output.extend_from_slice(text.as_bytes());
        used += text_width(&text);
    }
    output.extend(std::iter::repeat_n(b' ', width.saturating_sub(used)));
    output.extend_from_slice(b"\x1b[0m");
    if preserve_cursor {
        output.extend_from_slice(b"\x1b8");
    }
    output
}

fn status_segments(
    session: &str,
    tab_count: usize,
    active: usize,
    date: &str,
    time: &str,
    width: usize,
) -> (Vec<(String, bool)>, usize) {
    let session = format!("[{session}]");
    let clock = format!("{date} {time}");
    let mut first = 0;
    let mut last = tab_count.saturating_sub(1);

    loop {
        let tabs = tab_segments(tab_count, active, first, last);
        let used = text_width(&session) + 2 + segments_width(&tabs) + 5 + text_width(&clock);
        if used <= width {
            let mut result = vec![(session.clone(), false), ("  ".into(), false)];
            result.extend(tabs);
            result.push((format!("  |  {clock}"), false));
            return (result, used);
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
    for (text, marked) in [
        (format!("[{}:shell]", active + 1), true),
        (format!("  {time}"), false),
        (format!("  {session}"), false),
    ] {
        let remaining = width.saturating_sub(used);
        if remaining == 0 {
            break;
        }
        let text = truncate(&text, remaining);
        used += text_width(&text);
        result.push((text, marked));
    }
    (result, used)
}

fn tab_segments(count: usize, active: usize, first: usize, last: usize) -> Vec<(String, bool)> {
    let mut result = Vec::new();
    if first > 0 {
        result.push(("< ".into(), false));
    }
    for index in first..=last {
        if index > first {
            result.push(("  ".into(), false));
        }
        result.push((
            if index == active {
                format!("[{}:shell]", index + 1)
            } else {
                format!("{}:shell", index + 1)
            },
            index == active,
        ));
    }
    if last + 1 < count {
        result.push((" >".into(), false));
    }
    result
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

fn border(rects: &[(PaneId, Rect)], active: Option<PaneId>, x: u16, y: u16) -> u8 {
    let neighbor = |x, y| rects.iter().find(|(_, rect)| contains(*rect, x, y));
    let left = x.checked_sub(1).and_then(|x| neighbor(x, y));
    let right = neighbor(x.saturating_add(1), y);
    let above = y.checked_sub(1).and_then(|y| neighbor(x, y));
    let below = neighbor(x, y.saturating_add(1));
    if [left, right, above, below]
        .into_iter()
        .flatten()
        .any(|(pane, _)| Some(*pane) == active)
    {
        return b'#';
    }
    match (
        left.is_some() || right.is_some(),
        above.is_some() || below.is_some(),
    ) {
        (true, true) => b'+',
        (true, false) => b'|',
        (false, true) => b'-',
        (false, false) => b' ',
    }
}

fn set_attributes(output: &mut Vec<u8>, current: &mut Option<Attributes>, wanted: Attributes) {
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
    output.extend_from_slice(if terminal.modes().cursor_visible {
        b"\x1b[?25h"
    } else {
        b"\x1b[?25l"
    });
}

fn push_char(output: &mut Vec<u8>, character: char) {
    let mut bytes = [0; 4];
    output.extend_from_slice(character.encode_utf8(&mut bytes).as_bytes());
}

fn text_width(text: &str) -> usize {
    UnicodeWidthStr::width(text)
}

fn segments_width(segments: &[(String, bool)]) -> usize {
    segments.iter().map(|(text, _)| text_width(text)).sum()
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

    #[test]
    fn status_keeps_active_then_time_then_session_when_narrow() {
        let (segments, width) = status_segments("demo", 3, 1, "2026-07-19", "18:42", 20);
        let text = segments
            .into_iter()
            .map(|(text, _)| text)
            .collect::<String>();
        assert_eq!(width, 20);
        assert_eq!(text, "[2:shell]  18:42  [d");
    }

    #[test]
    fn status_removes_furthest_inactive_tabs_first() {
        let (segments, _) = status_segments("s", 5, 2, "2026-07-19", "18:42", 49);
        let text = segments
            .into_iter()
            .map(|(text, _)| text)
            .collect::<String>();
        assert!(text.contains("< "), "{text}");
        assert!(text.contains("[3:shell]"), "{text}");
        assert!(text.contains(" >"), "{text}");
        assert!(!text.contains("1:shell"));
        assert!(!text.contains("5:shell"));
    }

    #[test]
    fn active_pane_border_uses_distinct_ascii() {
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
        assert_eq!(border(&rects, active, 2, 0), b'#');
        assert_eq!(border(&rects, None, 2, 0), b'|');
    }

    #[test]
    fn full_render_includes_pane_content_and_status() {
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
            Clock {
                date_format: "%Y-%m-%d",
                time_format: "%H:%M",
            },
        );
        let output = String::from_utf8_lossy(&output);
        assert!(output.contains("hello"));
        assert!(output.contains("[s]"));
        assert!(output.contains("[1:shell]"));
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
        let mut previous = snapshot(&[(pane, &terminal)]);
        terminal.advance(b"\x1b[2;3HX");

        let output = changes(
            &session,
            &[(pane, &terminal)],
            Size {
                columns: 10,
                rows: 3,
            },
            &mut previous,
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
        let mut previous = snapshot(&[(pane, &terminal)]);
        terminal.advance(b"$ command\r\nresult\r\n$ ");

        let output = changes(
            &session,
            &[(pane, &terminal)],
            Size {
                columns: 20,
                rows: 5,
            },
            &mut previous,
        );

        assert!(output.ends_with(b"\x1b[3;3H\x1b[?25h"));
    }
}
