use std::{error::Error, fmt};

use unicode_width::UnicodeWidthChar;
use vte::{Params, Parser, Perform};

use crate::session::Size;

pub const MAX_CONTROL_SEQUENCE: usize = 4096;
pub const MAX_SCREEN_CELLS: usize = 262_144;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Color {
    #[default]
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Attributes {
    pub foreground: Color,
    pub background: Color,
    pub underline_color: Color,
    pub bold: bool,
    pub faint: bool,
    pub italic: bool,
    pub underline: bool,
    pub blink: bool,
    pub inverse: bool,
    pub hidden: bool,
    pub strike: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Cell {
    character: char,
    combining: Option<Box<str>>,
    width: u8,
    attributes: Attributes,
}

impl Cell {
    fn blank(attributes: Attributes) -> Self {
        Self {
            character: ' ',
            combining: None,
            width: 1,
            attributes,
        }
    }

    pub fn character(&self) -> char {
        self.character
    }

    pub fn combining(&self) -> &str {
        self.combining.as_deref().unwrap_or("")
    }

    pub fn width(&self) -> u8 {
        self.width
    }

    pub fn attributes(&self) -> Attributes {
        self.attributes
    }

    pub fn is_continuation(&self) -> bool {
        self.width == 0
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Cursor {
    pub row: usize,
    pub column: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum MouseMode {
    #[default]
    Off,
    Press,
    Drag,
    Motion,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Modes {
    pub application_cursor_keys: bool,
    pub bracketed_paste: bool,
    pub cursor_visible: bool,
    pub mouse: MouseMode,
    pub sgr_mouse: bool,
}

impl Default for Modes {
    fn default() -> Self {
        Self {
            application_cursor_keys: false,
            bracketed_paste: false,
            cursor_visible: true,
            mouse: MouseMode::Off,
            sgr_mouse: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalError {
    InvalidSize,
    ScreenTooLarge,
}

impl fmt::Display for TerminalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidSize => "terminal dimensions must be non-zero",
            Self::ScreenTooLarge => "terminal screen exceeds 262144 cells",
        })
    }
}

impl Error for TerminalError {}

#[derive(Debug)]
pub struct Screen {
    rows: Vec<Vec<Cell>>,
    cursor: Cursor,
    saved_cursor: Cursor,
    scroll_top: usize,
    scroll_bottom: usize,
    tabs: Vec<bool>,
    wrap_pending: bool,
}

impl Screen {
    fn new(size: Size) -> Result<Self, TerminalError> {
        validate_size(size)?;
        let columns = usize::from(size.columns);
        let rows = usize::from(size.rows);
        Ok(Self {
            rows: vec![vec![Cell::blank(Attributes::default()); columns]; rows],
            cursor: Cursor::default(),
            saved_cursor: Cursor::default(),
            scroll_top: 0,
            scroll_bottom: rows - 1,
            tabs: tab_stops(columns),
            wrap_pending: false,
        })
    }

    pub fn rows(&self) -> &[Vec<Cell>] {
        &self.rows
    }

    pub fn cursor(&self) -> Cursor {
        self.cursor
    }

    pub fn size(&self) -> Size {
        Size {
            columns: self.columns() as u16,
            rows: self.height() as u16,
        }
    }

    fn columns(&self) -> usize {
        self.rows[0].len()
    }

    fn height(&self) -> usize {
        self.rows.len()
    }

    fn clear(&mut self) {
        for row in &mut self.rows {
            row.fill(Cell::blank(Attributes::default()));
        }
        self.cursor = Cursor::default();
        self.saved_cursor = Cursor::default();
        self.scroll_top = 0;
        self.scroll_bottom = self.height() - 1;
        self.wrap_pending = false;
    }

    fn resize(&mut self, size: Size) -> Result<(), TerminalError> {
        validate_size(size)?;
        let columns = usize::from(size.columns);
        let height = usize::from(size.rows);
        for row in &mut self.rows {
            row.resize(columns, Cell::blank(Attributes::default()));
            normalize_row(row);
        }
        self.rows
            .resize(height, vec![Cell::blank(Attributes::default()); columns]);
        self.cursor.row = self.cursor.row.min(height - 1);
        self.cursor.column = self.cursor.column.min(columns - 1);
        self.saved_cursor.row = self.saved_cursor.row.min(height - 1);
        self.saved_cursor.column = self.saved_cursor.column.min(columns - 1);
        self.scroll_top = 0;
        self.scroll_bottom = height - 1;
        self.tabs.resize(columns, false);
        for column in 1..columns {
            if column % 8 == 0 {
                self.tabs[column] = true;
            }
        }
        self.wrap_pending = false;
        Ok(())
    }

    fn blank_row(&self, attributes: Attributes) -> Vec<Cell> {
        vec![Cell::blank(attributes); self.columns()]
    }

    fn erase(&mut self, row: usize, start: usize, end: usize, attributes: Attributes) {
        if start >= end {
            return;
        }
        let mut start = start.min(self.columns());
        let mut end = end.min(self.columns());
        if start > 0 && self.rows[row][start].is_continuation() {
            start -= 1;
        }
        if end < self.columns() && end > 0 && self.rows[row][end - 1].width == 2 {
            end += 1;
        }
        let columns = self.columns();
        self.rows[row][start..end.min(columns)].fill(Cell::blank(attributes));
    }

    fn scroll_up(&mut self, count: usize, attributes: Attributes) {
        let count = count.min(self.scroll_bottom - self.scroll_top + 1);
        // ponytail: row rotation is O(rows); use a ring only if profiling shows it matters.
        for _ in 0..count {
            self.rows.remove(self.scroll_top);
            self.rows
                .insert(self.scroll_bottom, self.blank_row(attributes));
        }
    }

    fn scroll_down(&mut self, count: usize, attributes: Attributes) {
        let count = count.min(self.scroll_bottom - self.scroll_top + 1);
        for _ in 0..count {
            self.rows.remove(self.scroll_bottom);
            self.rows
                .insert(self.scroll_top, self.blank_row(attributes));
        }
    }
}

pub struct Terminal {
    parser: Parser<MAX_CONTROL_SEQUENCE>,
    guard: SequenceGuard,
    state: State,
}

impl Terminal {
    pub fn new(size: Size) -> Result<Self, TerminalError> {
        Ok(Self {
            parser: Parser::new_with_size(),
            guard: SequenceGuard::Ground,
            state: State::new(size)?,
        })
    }

    pub fn advance(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            let overflow = matches!(
                self.guard,
                SequenceGuard::Sequence {
                    length: MAX_CONTROL_SEQUENCE,
                    ..
                }
            );
            if self.guard.accept(byte) {
                self.parser.advance(&mut self.state, &[byte]);
            } else if overflow {
                self.parser = Parser::new_with_size();
            }
        }
    }

    pub fn resize(&mut self, size: Size) -> Result<(), TerminalError> {
        self.state.primary.resize(size)?;
        self.state.alternate.resize(size)
    }

    pub fn screen(&self) -> &Screen {
        self.state.screen()
    }

    pub fn modes(&self) -> Modes {
        self.state.modes
    }

    pub fn in_alternate_screen(&self) -> bool {
        self.state.alternate_active
    }

    pub fn take_responses(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.state.responses)
    }
}

#[derive(Clone, Copy, Debug)]
enum SequenceKind {
    Escape,
    Csi,
    Osc,
    Dcs,
    String,
}

#[derive(Clone, Copy, Debug)]
enum SequenceGuard {
    Ground,
    Sequence { kind: SequenceKind, length: usize },
    Discard { kind: SequenceKind, escaped: bool },
}

impl SequenceGuard {
    fn accept(&mut self, byte: u8) -> bool {
        match *self {
            Self::Ground if byte == 0x1b => {
                *self = Self::Sequence {
                    kind: SequenceKind::Escape,
                    length: 1,
                };
                true
            }
            Self::Ground => true,
            Self::Sequence { kind, length } => {
                if length == MAX_CONTROL_SEQUENCE {
                    *self = if discarded_sequence_ended(kind, false, byte) {
                        Self::Ground
                    } else {
                        Self::Discard {
                            kind,
                            escaped: byte == 0x1b,
                        }
                    };
                    false
                } else {
                    *self = next_sequence(kind, length + 1, byte);
                    true
                }
            }
            Self::Discard { kind, escaped } => {
                if discarded_sequence_ended(kind, escaped, byte) {
                    *self = Self::Ground;
                } else {
                    *self = Self::Discard {
                        kind,
                        escaped: byte == 0x1b,
                    };
                }
                false
            }
        }
    }
}

fn next_sequence(kind: SequenceKind, length: usize, byte: u8) -> SequenceGuard {
    use SequenceGuard::{Ground, Sequence};
    use SequenceKind::{Csi, Dcs, Escape, Osc, String as ControlString};
    match kind {
        Escape => match byte {
            b'[' => Sequence { kind: Csi, length },
            b']' => Sequence { kind: Osc, length },
            b'P' => Sequence { kind: Dcs, length },
            b'X' | b'^' | b'_' => Sequence {
                kind: ControlString,
                length,
            },
            0x20..=0x2f => Sequence {
                kind: Escape,
                length,
            },
            _ => Ground,
        },
        Csi => match byte {
            0x18 | 0x1a | 0x40..=0x7e => Ground,
            0x1b => Sequence {
                kind: Escape,
                length: 1,
            },
            _ => Sequence { kind, length },
        },
        Osc if byte == 0x07 => Ground,
        Osc | Dcs | ControlString if byte == 0x1b => Sequence {
            kind: Escape,
            length: 1,
        },
        _ => Sequence { kind, length },
    }
}

fn discarded_sequence_ended(kind: SequenceKind, escaped: bool, byte: u8) -> bool {
    match kind {
        SequenceKind::Escape => (0x30..=0x7e).contains(&byte),
        SequenceKind::Csi => (0x40..=0x7e).contains(&byte) || matches!(byte, 0x18 | 0x1a),
        SequenceKind::Osc => byte == 0x07 || escaped && byte == b'\\',
        SequenceKind::Dcs | SequenceKind::String => escaped && byte == b'\\',
    }
}

struct State {
    primary: Screen,
    alternate: Screen,
    alternate_active: bool,
    attributes: Attributes,
    saved_attributes: Attributes,
    modes: Modes,
    auto_wrap: bool,
    insert: bool,
    origin: bool,
    responses: Vec<u8>,
}

impl State {
    fn new(size: Size) -> Result<Self, TerminalError> {
        Ok(Self {
            primary: Screen::new(size)?,
            alternate: Screen::new(size)?,
            alternate_active: false,
            attributes: Attributes::default(),
            saved_attributes: Attributes::default(),
            modes: Modes::default(),
            auto_wrap: true,
            insert: false,
            origin: false,
            responses: Vec::new(),
        })
    }

    fn screen(&self) -> &Screen {
        if self.alternate_active {
            &self.alternate
        } else {
            &self.primary
        }
    }

    fn screen_mut(&mut self) -> &mut Screen {
        if self.alternate_active {
            &mut self.alternate
        } else {
            &mut self.primary
        }
    }

    fn print(&mut self, character: char) {
        let width = character.width().unwrap_or(0).min(2);
        if width == 0 {
            let screen = self.screen_mut();
            let mut column = screen.cursor.column;
            if screen.wrap_pending || column > 0 {
                column = column.saturating_sub(!screen.wrap_pending as usize);
                while column > 0 && screen.rows[screen.cursor.row][column].is_continuation() {
                    column -= 1;
                }
                let cell = &mut screen.rows[screen.cursor.row][column];
                let mut combining = cell.combining.take().map_or_else(String::new, String::from);
                combining.push(character);
                cell.combining = Some(combining.into_boxed_str());
            }
            return;
        }

        let attributes = self.attributes;
        let auto_wrap = self.auto_wrap;
        let insert = self.insert;
        let screen = self.screen_mut();
        if screen.wrap_pending {
            if auto_wrap {
                screen.cursor.column = 0;
                linefeed(screen, attributes);
            }
            screen.wrap_pending = false;
        }
        if width == 2 && screen.cursor.column + 1 == screen.columns() {
            if !auto_wrap {
                return;
            }
            screen.cursor.column = 0;
            linefeed(screen, attributes);
        }

        let row = screen.cursor.row;
        let column = screen.cursor.column;
        clear_wide_at(screen, row, column, attributes);
        if width == 2 {
            clear_wide_at(screen, row, column + 1, attributes);
        }
        if insert {
            let count = width.min(screen.columns() - column);
            for _ in 0..count {
                screen.rows[row].insert(column, Cell::blank(attributes));
                screen.rows[row].pop();
            }
            normalize_row(&mut screen.rows[row]);
        }
        screen.rows[row][column] = Cell {
            character,
            combining: None,
            width: width as u8,
            attributes,
        };
        if width == 2 {
            screen.rows[row][column + 1] = Cell {
                character: ' ',
                combining: None,
                width: 0,
                attributes,
            };
        }
        if column + width >= screen.columns() {
            screen.cursor.column = screen.columns() - 1;
            screen.wrap_pending = auto_wrap;
        } else {
            screen.cursor.column += width;
        }
    }

    fn move_cursor(&mut self, row: usize, column: usize) {
        let origin = self.origin;
        let screen = self.screen_mut();
        let (top, bottom) = if origin {
            (screen.scroll_top, screen.scroll_bottom)
        } else {
            (0, screen.height() - 1)
        };
        screen.cursor.row = (row + if origin { top } else { 0 }).clamp(top, bottom);
        screen.cursor.column = column.min(screen.columns() - 1);
        screen.wrap_pending = false;
    }

    fn save_cursor(&mut self) {
        self.saved_attributes = self.attributes;
        let screen = self.screen_mut();
        screen.saved_cursor = screen.cursor;
    }

    fn restore_cursor(&mut self) {
        self.attributes = self.saved_attributes;
        let screen = self.screen_mut();
        screen.cursor = screen.saved_cursor;
        screen.wrap_pending = false;
    }

    fn reset(&mut self) {
        let size = self.screen().size();
        *self = Self::new(size).expect("existing terminal size is valid");
    }

    fn set_private_mode(&mut self, mode: u16, enabled: bool) {
        match mode {
            1 => self.modes.application_cursor_keys = enabled,
            6 => {
                self.origin = enabled;
                self.move_cursor(0, 0);
            }
            7 => self.auto_wrap = enabled,
            25 => self.modes.cursor_visible = enabled,
            47 => self.alternate_active = enabled,
            1047 => {
                self.alternate_active = enabled;
                if enabled {
                    self.alternate.clear();
                }
            }
            1048 => {
                if enabled {
                    self.save_cursor();
                } else {
                    self.restore_cursor();
                }
            }
            1049 => {
                if enabled {
                    self.save_cursor();
                    self.alternate.clear();
                    self.alternate_active = true;
                } else {
                    self.alternate_active = false;
                    self.restore_cursor();
                }
            }
            1000 => {
                self.modes.mouse = if enabled {
                    MouseMode::Press
                } else {
                    MouseMode::Off
                }
            }
            1002 => {
                self.modes.mouse = if enabled {
                    MouseMode::Drag
                } else {
                    MouseMode::Off
                }
            }
            1003 => {
                self.modes.mouse = if enabled {
                    MouseMode::Motion
                } else {
                    MouseMode::Off
                }
            }
            1006 => self.modes.sgr_mouse = enabled,
            2004 => self.modes.bracketed_paste = enabled,
            _ => {}
        }
    }

    fn sgr(&mut self, params: &Params) {
        let values = params.iter().collect::<Vec<_>>();
        if values.is_empty() {
            self.attributes = Attributes::default();
            return;
        }
        let mut index = 0;
        while index < values.len() {
            let value = values[index][0];
            index += 1;
            match value {
                0 => self.attributes = Attributes::default(),
                1 => self.attributes.bold = true,
                2 => self.attributes.faint = true,
                3 => self.attributes.italic = true,
                4 => self.attributes.underline = true,
                5 | 6 => self.attributes.blink = true,
                7 => self.attributes.inverse = true,
                8 => self.attributes.hidden = true,
                9 => self.attributes.strike = true,
                22 => {
                    self.attributes.bold = false;
                    self.attributes.faint = false;
                }
                23 => self.attributes.italic = false,
                24 => self.attributes.underline = false,
                25 => self.attributes.blink = false,
                27 => self.attributes.inverse = false,
                28 => self.attributes.hidden = false,
                29 => self.attributes.strike = false,
                30..=37 => self.attributes.foreground = Color::Indexed(value as u8 - 30),
                38 => set_extended_color(&values, &mut index, &mut self.attributes.foreground),
                39 => self.attributes.foreground = Color::Default,
                40..=47 => self.attributes.background = Color::Indexed(value as u8 - 40),
                48 => set_extended_color(&values, &mut index, &mut self.attributes.background),
                49 => self.attributes.background = Color::Default,
                58 => set_extended_color(&values, &mut index, &mut self.attributes.underline_color),
                59 => self.attributes.underline_color = Color::Default,
                90..=97 => self.attributes.foreground = Color::Indexed(value as u8 - 82),
                100..=107 => self.attributes.background = Color::Indexed(value as u8 - 92),
                _ => {}
            }
        }
    }
}

impl Perform for State {
    fn print(&mut self, character: char) {
        self.print(character);
    }

    fn execute(&mut self, byte: u8) {
        let attributes = self.attributes;
        let screen = self.screen_mut();
        match byte {
            0x08 => {
                screen.cursor.column = screen.cursor.column.saturating_sub(1);
                screen.wrap_pending = false;
            }
            0x09 => {
                screen.cursor.column = ((screen.cursor.column + 1)..screen.columns())
                    .find(|&column| screen.tabs[column])
                    .unwrap_or(screen.columns() - 1);
                screen.wrap_pending = false;
            }
            0x0a..=0x0c => linefeed(screen, attributes),
            0x0d => {
                screen.cursor.column = 0;
                screen.wrap_pending = false;
            }
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], ignore: bool, byte: u8) {
        if ignore {
            return;
        }
        let attributes = self.attributes;
        match (intermediates, byte) {
            ([], b'7') => self.save_cursor(),
            ([], b'8') => self.restore_cursor(),
            ([], b'D') => linefeed(self.screen_mut(), attributes),
            ([], b'E') => {
                let screen = self.screen_mut();
                screen.cursor.column = 0;
                linefeed(screen, attributes);
            }
            ([], b'M') => {
                let screen = self.screen_mut();
                if screen.cursor.row == screen.scroll_top {
                    screen.scroll_down(1, attributes);
                } else {
                    screen.cursor.row = screen.cursor.row.saturating_sub(1);
                }
                screen.wrap_pending = false;
            }
            ([], b'H') => {
                let screen = self.screen_mut();
                screen.tabs[screen.cursor.column] = true;
            }
            ([], b'c') => self.reset(),
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        if ignore {
            return;
        }
        let private = intermediates == b"?";
        let values = params.iter().map(|param| param[0]).collect::<Vec<_>>();
        let first = values.first().copied().unwrap_or(0);
        let count = usize::from(first.max(1));
        let attributes = self.attributes;

        if private && matches!(action, 'h' | 'l') {
            for mode in values {
                self.set_private_mode(mode, action == 'h');
            }
            return;
        }
        if !private && matches!(action, 'h' | 'l') {
            if values.contains(&4) {
                self.insert = action == 'h';
            }
            return;
        }

        match action {
            'A' => {
                let origin = self.origin;
                let screen = self.screen_mut();
                let top = if origin { screen.scroll_top } else { 0 };
                screen.cursor.row = screen.cursor.row.saturating_sub(count).max(top);
                screen.wrap_pending = false;
            }
            'B' | 'e' => {
                let origin = self.origin;
                let screen = self.screen_mut();
                let bottom = if origin {
                    screen.scroll_bottom
                } else {
                    screen.height() - 1
                };
                screen.cursor.row = (screen.cursor.row + count).min(bottom);
                screen.wrap_pending = false;
            }
            'C' | 'a' => {
                let screen = self.screen_mut();
                screen.cursor.column = (screen.cursor.column + count).min(screen.columns() - 1);
                screen.wrap_pending = false;
            }
            'D' => {
                let screen = self.screen_mut();
                screen.cursor.column = screen.cursor.column.saturating_sub(count);
                screen.wrap_pending = false;
            }
            'E' => {
                let origin = self.origin;
                let screen = self.screen_mut();
                let bottom = if origin {
                    screen.scroll_bottom
                } else {
                    screen.height() - 1
                };
                screen.cursor.row = (screen.cursor.row + count).min(bottom);
                screen.cursor.column = 0;
                screen.wrap_pending = false;
            }
            'F' => {
                let origin = self.origin;
                let screen = self.screen_mut();
                let top = if origin { screen.scroll_top } else { 0 };
                screen.cursor.row = screen.cursor.row.saturating_sub(count).max(top);
                screen.cursor.column = 0;
                screen.wrap_pending = false;
            }
            'G' | '`' => {
                let screen = self.screen_mut();
                screen.cursor.column = count.saturating_sub(1).min(screen.columns() - 1);
                screen.wrap_pending = false;
            }
            'H' | 'f' => {
                let row = values.first().copied().unwrap_or(1).max(1) as usize - 1;
                let column = values.get(1).copied().unwrap_or(1).max(1) as usize - 1;
                self.move_cursor(row, column);
            }
            'd' => {
                let column = self.screen().cursor.column;
                self.move_cursor(count - 1, column);
            }
            'J' => erase_display(self.screen_mut(), first, attributes),
            'K' => erase_line(self.screen_mut(), first, attributes),
            '@' => insert_cells(self.screen_mut(), count, attributes),
            'P' => delete_cells(self.screen_mut(), count, attributes),
            'X' => {
                let screen = self.screen_mut();
                let row = screen.cursor.row;
                let column = screen.cursor.column;
                screen.erase(row, column, column + count, attributes);
            }
            'L' => insert_lines(self.screen_mut(), count, attributes),
            'M' => delete_lines(self.screen_mut(), count, attributes),
            'S' => self.screen_mut().scroll_up(count, attributes),
            'T' => self.screen_mut().scroll_down(count, attributes),
            'I' => move_tabs(self.screen_mut(), count, true),
            'Z' => move_tabs(self.screen_mut(), count, false),
            'g' => {
                let screen = self.screen_mut();
                match first {
                    0 => screen.tabs[screen.cursor.column] = false,
                    3 => screen.tabs.fill(false),
                    _ => {}
                }
            }
            'm' => self.sgr(params),
            'r' if !private => {
                let origin = self.origin;
                let screen = self.screen_mut();
                let top = values.first().copied().unwrap_or(1).max(1) as usize - 1;
                let bottom = values
                    .get(1)
                    .copied()
                    .filter(|value| *value != 0)
                    .map_or(screen.height(), usize::from)
                    .min(screen.height());
                if top < bottom {
                    screen.scroll_top = top;
                    screen.scroll_bottom = bottom - 1;
                    screen.cursor = Cursor {
                        row: if origin { top } else { 0 },
                        column: 0,
                    };
                    screen.wrap_pending = false;
                }
            }
            's' => self.save_cursor(),
            'u' => self.restore_cursor(),
            'n' if first == 5 => self.responses.extend_from_slice(b"\x1b[0n"),
            'n' if first == 6 => {
                let cursor = self.screen().cursor;
                let row = if private && self.origin {
                    cursor.row - self.screen().scroll_top + 1
                } else {
                    cursor.row + 1
                };
                self.responses.extend_from_slice(
                    format!(
                        "\x1b[{}{};{}R",
                        if private { "?" } else { "" },
                        row,
                        cursor.column + 1
                    )
                    .as_bytes(),
                );
            }
            'c' => self.responses.extend_from_slice(b"\x1b[?1;2c"),
            _ => {}
        }
    }

    fn osc_dispatch(&mut self, _params: &[&[u8]], _bell_terminated: bool) {
        // All OSC, including clipboard-writing OSC 52, is intentionally ignored.
    }
}

fn validate_size(size: Size) -> Result<(), TerminalError> {
    if size.columns == 0 || size.rows == 0 {
        return Err(TerminalError::InvalidSize);
    }
    if usize::from(size.columns) * usize::from(size.rows) > MAX_SCREEN_CELLS {
        return Err(TerminalError::ScreenTooLarge);
    }
    Ok(())
}

fn tab_stops(columns: usize) -> Vec<bool> {
    (0..columns)
        .map(|column| column != 0 && column % 8 == 0)
        .collect()
}

fn linefeed(screen: &mut Screen, attributes: Attributes) {
    if screen.cursor.row == screen.scroll_bottom {
        screen.scroll_up(1, attributes);
    } else if screen.cursor.row + 1 < screen.height() {
        screen.cursor.row += 1;
    }
    screen.wrap_pending = false;
}

fn clear_wide_at(screen: &mut Screen, row: usize, column: usize, attributes: Attributes) {
    if screen.rows[row][column].is_continuation() && column > 0 {
        screen.rows[row][column - 1] = Cell::blank(attributes);
    } else if screen.rows[row][column].width == 2 && column + 1 < screen.columns() {
        screen.rows[row][column + 1] = Cell::blank(attributes);
    }
}

fn normalize_row(row: &mut [Cell]) {
    for column in 0..row.len() {
        let broken_wide =
            row[column].width == 2 && (column + 1 == row.len() || row[column + 1].width != 0);
        let broken_continuation =
            row[column].width == 0 && (column == 0 || row[column - 1].width != 2);
        if broken_wide || broken_continuation {
            row[column] = Cell::blank(Attributes::default());
        }
    }
}

fn erase_display(screen: &mut Screen, mode: u16, attributes: Attributes) {
    let cursor = screen.cursor;
    match mode {
        0 => {
            screen.erase(cursor.row, cursor.column, screen.columns(), attributes);
            for row in cursor.row + 1..screen.height() {
                screen.erase(row, 0, screen.columns(), attributes);
            }
        }
        1 => {
            for row in 0..cursor.row {
                screen.erase(row, 0, screen.columns(), attributes);
            }
            screen.erase(cursor.row, 0, cursor.column + 1, attributes);
        }
        2 | 3 => {
            for row in 0..screen.height() {
                screen.erase(row, 0, screen.columns(), attributes);
            }
        }
        _ => {}
    }
}

fn erase_line(screen: &mut Screen, mode: u16, attributes: Attributes) {
    let cursor = screen.cursor;
    match mode {
        0 => screen.erase(cursor.row, cursor.column, screen.columns(), attributes),
        1 => screen.erase(cursor.row, 0, cursor.column + 1, attributes),
        2 => screen.erase(cursor.row, 0, screen.columns(), attributes),
        _ => {}
    }
}

fn insert_cells(screen: &mut Screen, count: usize, attributes: Attributes) {
    let row = screen.cursor.row;
    let column = screen.cursor.column;
    for _ in 0..count.min(screen.columns() - column) {
        screen.rows[row].insert(column, Cell::blank(attributes));
        screen.rows[row].pop();
    }
    normalize_row(&mut screen.rows[row]);
}

fn delete_cells(screen: &mut Screen, count: usize, attributes: Attributes) {
    let row = screen.cursor.row;
    let column = screen.cursor.column;
    for _ in 0..count.min(screen.columns() - column) {
        screen.rows[row].remove(column);
        screen.rows[row].push(Cell::blank(attributes));
    }
    normalize_row(&mut screen.rows[row]);
}

fn insert_lines(screen: &mut Screen, count: usize, attributes: Attributes) {
    let row = screen.cursor.row;
    if !(screen.scroll_top..=screen.scroll_bottom).contains(&row) {
        return;
    }
    for _ in 0..count.min(screen.scroll_bottom - row + 1) {
        screen.rows.remove(screen.scroll_bottom);
        screen.rows.insert(row, screen.blank_row(attributes));
    }
}

fn delete_lines(screen: &mut Screen, count: usize, attributes: Attributes) {
    let row = screen.cursor.row;
    if !(screen.scroll_top..=screen.scroll_bottom).contains(&row) {
        return;
    }
    for _ in 0..count.min(screen.scroll_bottom - row + 1) {
        screen.rows.remove(row);
        screen
            .rows
            .insert(screen.scroll_bottom, screen.blank_row(attributes));
    }
}

fn move_tabs(screen: &mut Screen, count: usize, forward: bool) {
    for _ in 0..count {
        screen.cursor.column = if forward {
            ((screen.cursor.column + 1)..screen.columns())
                .find(|&column| screen.tabs[column])
                .unwrap_or(screen.columns() - 1)
        } else {
            (0..screen.cursor.column)
                .rev()
                .find(|&column| screen.tabs[column])
                .unwrap_or(0)
        };
    }
    screen.wrap_pending = false;
}

fn set_extended_color(values: &[&[u16]], index: &mut usize, color: &mut Color) {
    let previous = values[*index - 1];
    if previous.len() > 1 {
        let components = if previous.get(1) == Some(&2) && previous.len() >= 5 {
            &previous[previous.len() - 3..]
        } else {
            &[]
        };
        if previous.get(1) == Some(&5) && previous.get(2).is_some_and(|value| *value <= 255) {
            *color = Color::Indexed(previous[2] as u8);
        } else if components.iter().all(|value| *value <= 255) && components.len() == 3 {
            *color = Color::Rgb(
                components[0] as u8,
                components[1] as u8,
                components[2] as u8,
            );
        }
        return;
    }

    match values.get(*index).and_then(|value| value.first()).copied() {
        Some(5)
            if values
                .get(*index + 1)
                .and_then(|value| value.first())
                .is_some_and(|value| *value <= 255) =>
        {
            *color = Color::Indexed(values[*index + 1][0] as u8);
            *index += 2;
        }
        Some(2)
            if values
                .get(*index + 1..*index + 4)
                .is_some_and(|rgb| rgb.iter().all(|value| value[0] <= 255)) =>
        {
            *color = Color::Rgb(
                values[*index + 1][0] as u8,
                values[*index + 2][0] as u8,
                values[*index + 3][0] as u8,
            );
            *index += 4;
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn terminal(columns: u16, rows: u16) -> Terminal {
        Terminal::new(Size { columns, rows }).unwrap()
    }

    fn line(terminal: &Terminal, row: usize) -> String {
        terminal.screen().rows()[row]
            .iter()
            .filter(|cell| !cell.is_continuation())
            .map(Cell::character)
            .collect()
    }

    #[test]
    fn parses_text_width_cursor_editing_and_scrolling() {
        let mut terminal = terminal(5, 2);
        terminal.advance("a界\u{301}bcdef".as_bytes());
        assert_eq!(line(&terminal, 0), "a界bc");
        assert_eq!(line(&terminal, 1), "def  ");
        assert_eq!(terminal.screen().rows()[0][1].combining(), "\u{301}");

        terminal.advance(b"\x1b[2;1H\x1b[2@XY\x1b[1P");
        assert_eq!(line(&terminal, 1), "XYef ");
        terminal.advance(b"\r\nnext\r\nlast");
        assert_eq!(line(&terminal, 0), "next ");
        assert_eq!(line(&terminal, 1), "last ");
    }

    #[test]
    fn tracks_sgr_buffers_and_input_modes() {
        let mut terminal = terminal(8, 2);
        terminal.advance(b"\x1b[1;38;5;42;48;2;1;2;3mX\x1b[?1;1002;1006;2004h");
        let attributes = terminal.screen().rows()[0][0].attributes();
        assert!(attributes.bold);
        assert_eq!(attributes.foreground, Color::Indexed(42));
        assert_eq!(attributes.background, Color::Rgb(1, 2, 3));
        assert_eq!(terminal.modes().mouse, MouseMode::Drag);
        assert!(terminal.modes().application_cursor_keys);
        assert!(terminal.modes().sgr_mouse);
        assert!(terminal.modes().bracketed_paste);

        terminal.advance(b"\x1b[38:2::4:5:6mY");
        assert_eq!(
            terminal.screen().rows()[0][1].attributes().foreground,
            Color::Rgb(4, 5, 6)
        );

        terminal.advance(b"\x1b[?1049hALT\x1b[?1049l");
        assert!(!terminal.in_alternate_screen());
        assert_eq!(line(&terminal, 0), "XY      ");
    }

    #[test]
    fn bounds_sizes_sequences_and_ignores_osc_52() {
        assert_eq!(
            Terminal::new(Size {
                columns: u16::MAX,
                rows: u16::MAX,
            })
            .err(),
            Some(TerminalError::ScreenTooLarge)
        );

        let mut terminal = terminal(8, 1);
        let mut oversized = b"\x1b]52;c;".to_vec();
        oversized.extend(std::iter::repeat_n(b'x', MAX_CONTROL_SEQUENCE));
        oversized.extend_from_slice(b"\x07safe");
        terminal.advance(&oversized);
        assert_eq!(line(&terminal, 0), "safe    ");

        let mut oversized_csi = b"\x1b[".to_vec();
        oversized_csi.extend(std::iter::repeat_n(b'1', MAX_CONTROL_SEQUENCE - 2));
        oversized_csi.extend_from_slice(b"mOK");
        terminal.advance(&oversized_csi);
        assert_eq!(line(&terminal, 0), "safeOK  ");
    }

    #[test]
    fn emits_standard_terminal_reports_and_resizes_without_reflow() {
        let mut terminal = terminal(4, 2);
        terminal.advance(b"abc\x1b[6n\x1b[c");
        assert_eq!(terminal.take_responses(), b"\x1b[1;4R\x1b[?1;2c");
        terminal
            .resize(Size {
                columns: 3,
                rows: 3,
            })
            .unwrap();
        assert_eq!(line(&terminal, 0), "abc");
        assert_eq!(line(&terminal, 2), "   ");
    }
}
