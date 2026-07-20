use std::{error::Error, fmt};

pub const MAX_TABS: usize = 32;
pub const MAX_PANES_PER_TAB: usize = 16;
pub const MAX_SESSIONS_PER_USER: usize = 32;

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PaneId(u32);

impl PaneId {
    pub fn get(self) -> u32 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Split {
    LeftRight,
    TopBottom,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Size {
    pub columns: u16,
    pub rows: u16,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Rect {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StateError {
    TabLimit,
    PaneLimit,
    TooSmall,
    NoAdjacentPane,
    CannotResize,
    EmptySession,
}

impl fmt::Display for StateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::TabLimit => "session already has 32 tabs",
            Self::PaneLimit => "tab already has 16 panes",
            Self::TooSmall => "active pane is too small to split",
            Self::NoAdjacentPane => "no pane exists in that direction",
            Self::CannotResize => "pane cannot be resized in that direction",
            Self::EmptySession => "session has no tabs",
        })
    }
}

impl Error for StateError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CloseResult {
    PaneClosed,
    TabClosed,
    SessionEmpty,
}

#[derive(Debug)]
pub struct Session {
    name: String,
    tabs: Vec<Tab>,
    active_tab: usize,
    next_pane_id: u32,
}

impl Session {
    pub fn new(name: String) -> Self {
        Self {
            name,
            tabs: vec![Tab::new(PaneId(1))],
            active_tab: 0,
            next_pane_id: 2,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    pub fn active_tab(&self) -> Option<usize> {
        (!self.tabs.is_empty()).then_some(self.active_tab)
    }

    pub fn active_pane(&self) -> Option<PaneId> {
        self.tabs.get(self.active_tab).map(|tab| tab.active)
    }

    pub fn pane_count(&self) -> usize {
        self.tabs
            .get(self.active_tab)
            .map_or(0, |tab| tab.layout.pane_count())
    }

    pub fn create_tab(&mut self) -> Result<PaneId, StateError> {
        if self.tabs.is_empty() {
            return Err(StateError::EmptySession);
        }
        if self.tabs.len() == MAX_TABS {
            return Err(StateError::TabLimit);
        }
        let pane = self.next_pane();
        self.tabs.push(Tab::new(pane));
        self.active_tab = self.tabs.len() - 1;
        Ok(pane)
    }

    pub fn select_tab(&mut self, index: usize) -> bool {
        if index >= self.tabs.len() {
            return false;
        }
        self.active_tab = index;
        true
    }

    pub fn select_pane(&mut self, pane: PaneId) -> bool {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else {
            return false;
        };
        if !tab.layout.contains(pane) {
            return false;
        }
        tab.active = pane;
        true
    }

    pub fn next_tab(&mut self) -> Result<(), StateError> {
        if self.tabs.is_empty() {
            return Err(StateError::EmptySession);
        }
        self.active_tab = (self.active_tab + 1) % self.tabs.len();
        Ok(())
    }

    pub fn previous_tab(&mut self) -> Result<(), StateError> {
        if self.tabs.is_empty() {
            return Err(StateError::EmptySession);
        }
        self.active_tab = (self.active_tab + self.tabs.len() - 1) % self.tabs.len();
        Ok(())
    }

    pub fn split_active(&mut self, split: Split, size: Size) -> Result<PaneId, StateError> {
        let tab = self
            .tabs
            .get(self.active_tab)
            .ok_or(StateError::EmptySession)?;
        if tab.layout.pane_count() == MAX_PANES_PER_TAB {
            return Err(StateError::PaneLimit);
        }
        let active_rect = tab
            .layout
            .rects(size)
            .into_iter()
            .find(|(pane, _)| *pane == tab.active)
            .map(|(_, rect)| rect)
            .expect("active pane belongs to its tab");
        let fits = match split {
            Split::LeftRight => active_rect.width >= 3,
            Split::TopBottom => active_rect.height >= 3,
        };
        if !fits {
            return Err(StateError::TooSmall);
        }

        let pane = self.next_pane();
        let tab = &mut self.tabs[self.active_tab];
        tab.layout.split(tab.active, pane, split);
        tab.active = pane;
        Ok(pane)
    }

    pub fn focus(&mut self, direction: Direction, size: Size) -> Result<PaneId, StateError> {
        let tab = self
            .tabs
            .get_mut(self.active_tab)
            .ok_or(StateError::EmptySession)?;
        let rects = tab.layout.rects(size);
        let active_rect = rects
            .iter()
            .find(|(pane, _)| *pane == tab.active)
            .map(|(_, rect)| *rect)
            .expect("active pane belongs to its tab");
        let next = rects
            .into_iter()
            .filter(|(pane, _)| *pane != tab.active)
            .filter_map(|(pane, rect)| {
                direction_rank(active_rect, rect, direction).map(|rank| (rank, pane))
            })
            .min()
            .map(|(_, pane)| pane)
            .ok_or(StateError::NoAdjacentPane)?;
        tab.active = next;
        Ok(next)
    }

    pub fn resize(&mut self, direction: Direction, size: Size) -> Result<(), StateError> {
        let tab = self
            .tabs
            .get_mut(self.active_tab)
            .ok_or(StateError::EmptySession)?;
        match tab.layout.resize(tab.active, direction, full_rect(size)) {
            ResizeResult::Moved => Ok(()),
            ResizeResult::Blocked => Err(StateError::CannotResize),
            ResizeResult::NotFound => Err(StateError::NoAdjacentPane),
        }
    }

    pub fn pane_rects(&self, size: Size) -> Vec<(PaneId, Rect)> {
        self.tabs
            .get(self.active_tab)
            .map_or_else(Vec::new, |tab| tab.layout.rects(size))
    }

    pub fn close_active_pane(&mut self, size: Size) -> Result<CloseResult, StateError> {
        let active = self.active_pane().ok_or(StateError::EmptySession)?;
        self.close_pane(active, size)
    }

    pub fn close_pane(&mut self, pane: PaneId, size: Size) -> Result<CloseResult, StateError> {
        let tab_index = self
            .tabs
            .iter()
            .position(|tab| tab.layout.contains(pane))
            .ok_or(StateError::NoAdjacentPane)?;
        let tab = self.tabs.get(tab_index).ok_or(StateError::EmptySession)?;
        let rects = tab.layout.rects(size);
        let pane_rect = rects
            .iter()
            .find(|(candidate, _)| *candidate == pane)
            .map(|(_, rect)| *rect)
            .expect("pane belongs to its tab");
        let nearest = rects
            .into_iter()
            .filter(|(candidate, _)| *candidate != pane)
            .min_by_key(|(candidate, rect)| (rect_distance(pane_rect, *rect), *candidate))
            .map(|(candidate, _)| candidate);

        if let Some(nearest) = nearest {
            let tab = &mut self.tabs[tab_index];
            tab.layout = tab
                .layout
                .clone()
                .remove(pane)
                .expect("another pane survives the close");
            if tab.active == pane {
                tab.active = nearest;
            }
            return Ok(CloseResult::PaneClosed);
        }

        self.tabs.remove(tab_index);
        if self.tabs.is_empty() {
            self.active_tab = 0;
            Ok(CloseResult::SessionEmpty)
        } else {
            if tab_index < self.active_tab {
                self.active_tab -= 1;
            }
            self.active_tab = self.active_tab.min(self.tabs.len() - 1);
            Ok(CloseResult::TabClosed)
        }
    }

    fn next_pane(&mut self) -> PaneId {
        let pane = PaneId(self.next_pane_id);
        self.next_pane_id += 1;
        pane
    }
}

#[derive(Debug)]
struct Tab {
    layout: Layout,
    active: PaneId,
}

impl Tab {
    fn new(pane: PaneId) -> Self {
        Self {
            layout: Layout::Pane(pane),
            active: pane,
        }
    }
}

#[derive(Clone, Debug)]
enum Layout {
    Pane(PaneId),
    Split {
        direction: Split,
        offset: i32,
        first: Box<Self>,
        second: Box<Self>,
    },
}

impl Layout {
    fn pane_count(&self) -> usize {
        match self {
            Self::Pane(_) => 1,
            Self::Split { first, second, .. } => first.pane_count() + second.pane_count(),
        }
    }

    fn contains(&self, target: PaneId) -> bool {
        match self {
            Self::Pane(pane) => *pane == target,
            Self::Split { first, second, .. } => first.contains(target) || second.contains(target),
        }
    }

    fn split(&mut self, target: PaneId, pane: PaneId, direction: Split) {
        match self {
            Self::Pane(current) if *current == target => {
                *self = Self::Split {
                    direction,
                    offset: 0,
                    first: Box::new(Self::Pane(target)),
                    second: Box::new(Self::Pane(pane)),
                };
            }
            Self::Split { first, second, .. } => {
                if first.contains(target) {
                    first.split(target, pane, direction);
                } else {
                    second.split(target, pane, direction);
                }
            }
            Self::Pane(_) => unreachable!("target pane belongs to layout"),
        }
    }

    fn remove(self, target: PaneId) -> Option<Self> {
        match self {
            Self::Pane(pane) if pane == target => None,
            Self::Pane(_) => Some(self),
            Self::Split {
                direction,
                offset,
                first,
                second,
            } => {
                if first.contains(target) {
                    match first.remove(target) {
                        Some(first) => Some(Self::Split {
                            direction,
                            offset,
                            first: Box::new(first),
                            second,
                        }),
                        None => Some(*second),
                    }
                } else {
                    match second.remove(target) {
                        Some(second) => Some(Self::Split {
                            direction,
                            offset,
                            first,
                            second: Box::new(second),
                        }),
                        None => Some(*first),
                    }
                }
            }
        }
    }

    fn rects(&self, size: Size) -> Vec<(PaneId, Rect)> {
        let mut output = Vec::with_capacity(self.pane_count());
        self.collect_rects(full_rect(size), &mut output);
        output
    }

    fn collect_rects(&self, rect: Rect, output: &mut Vec<(PaneId, Rect)>) {
        match self {
            Self::Pane(pane) => output.push((*pane, rect)),
            Self::Split {
                direction,
                offset,
                first,
                second,
            } => {
                let (first_rect, second_rect) =
                    child_rects(rect, *direction, *offset, first, second);
                first.collect_rects(first_rect, output);
                second.collect_rects(second_rect, output);
            }
        }
    }

    fn minimum_size(&self) -> Size {
        match self {
            Self::Pane(_) => Size {
                columns: 1,
                rows: 1,
            },
            Self::Split {
                direction,
                first,
                second,
                ..
            } => {
                let first = first.minimum_size();
                let second = second.minimum_size();
                match direction {
                    Split::LeftRight => Size {
                        columns: first
                            .columns
                            .saturating_add(second.columns)
                            .saturating_add(1),
                        rows: first.rows.max(second.rows),
                    },
                    Split::TopBottom => Size {
                        columns: first.columns.max(second.columns),
                        rows: first.rows.saturating_add(second.rows).saturating_add(1),
                    },
                }
            }
        }
    }

    fn resize(&mut self, target: PaneId, direction: Direction, rect: Rect) -> ResizeResult {
        let Self::Split {
            direction: split,
            offset,
            first,
            second,
        } = self
        else {
            return ResizeResult::NotFound;
        };

        let target_in_first = first.contains(target);
        let (first_rect, second_rect) = child_rects(rect, *split, *offset, first, second);
        let nested = if target_in_first {
            first.resize(target, direction, first_rect)
        } else {
            second.resize(target, direction, second_rect)
        };
        if nested != ResizeResult::NotFound {
            return nested;
        }

        let delta = match (*split, direction, target_in_first) {
            (Split::LeftRight, Direction::Right, true)
            | (Split::TopBottom, Direction::Down, true) => 1,
            (Split::LeftRight, Direction::Left, false)
            | (Split::TopBottom, Direction::Up, false) => -1,
            _ => return ResizeResult::NotFound,
        };
        let total = match split {
            Split::LeftRight => rect.width,
            Split::TopBottom => rect.height,
        };
        if total < 3 {
            return ResizeResult::Blocked;
        }
        let usable = total - 1;
        let first_min = match split {
            Split::LeftRight => first.minimum_size().columns,
            Split::TopBottom => first.minimum_size().rows,
        };
        let second_min = match split {
            Split::LeftRight => second.minimum_size().columns,
            Split::TopBottom => second.minimum_size().rows,
        };
        let current = match split {
            Split::LeftRight => first_rect.width,
            Split::TopBottom => first_rect.height,
        };
        let wanted = i32::from(current) + delta;
        if wanted < i32::from(first_min) || i32::from(usable) - wanted < i32::from(second_min) {
            return ResizeResult::Blocked;
        }
        *offset = wanted - i32::from(usable / 2);
        ResizeResult::Moved
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ResizeResult {
    Moved,
    Blocked,
    NotFound,
}

fn full_rect(size: Size) -> Rect {
    Rect {
        x: 0,
        y: 0,
        width: size.columns,
        height: size.rows,
    }
}

fn child_rects(
    rect: Rect,
    split: Split,
    offset: i32,
    first: &Layout,
    second: &Layout,
) -> (Rect, Rect) {
    let first_min = first.minimum_size();
    let second_min = second.minimum_size();
    match split {
        Split::LeftRight => {
            let (first_width, second_width) =
                split_lengths(rect.width, offset, first_min.columns, second_min.columns);
            (
                Rect {
                    width: first_width,
                    ..rect
                },
                Rect {
                    x: rect
                        .x
                        .saturating_add(first_width)
                        .saturating_add(if rect.width > 1 { 1 } else { 0 }),
                    width: second_width,
                    ..rect
                },
            )
        }
        Split::TopBottom => {
            let (first_height, second_height) =
                split_lengths(rect.height, offset, first_min.rows, second_min.rows);
            (
                Rect {
                    height: first_height,
                    ..rect
                },
                Rect {
                    y: rect
                        .y
                        .saturating_add(first_height)
                        .saturating_add(if rect.height > 1 { 1 } else { 0 }),
                    height: second_height,
                    ..rect
                },
            )
        }
    }
}

fn split_lengths(total: u16, offset: i32, first_minimum: u16, second_minimum: u16) -> (u16, u16) {
    if total < 2 {
        return (total, 0);
    }
    let usable = total - 1;
    let low = first_minimum.min(usable);
    let high = usable.saturating_sub(second_minimum).max(low);
    let first = (i32::from(usable / 2) + offset).clamp(i32::from(low), i32::from(high)) as u16;
    (first, usable - first)
}

fn direction_rank(from: Rect, to: Rect, direction: Direction) -> Option<(u32, u32, u32)> {
    let (primary, perpendicular) = match direction {
        Direction::Left if right(to) <= u32::from(from.x) => (
            u32::from(from.x) - right(to),
            interval_gap(from.y, from.height, to.y, to.height),
        ),
        Direction::Right if u32::from(to.x) >= right(from) => (
            u32::from(to.x) - right(from),
            interval_gap(from.y, from.height, to.y, to.height),
        ),
        Direction::Up if bottom(to) <= u32::from(from.y) => (
            u32::from(from.y) - bottom(to),
            interval_gap(from.x, from.width, to.x, to.width),
        ),
        Direction::Down if u32::from(to.y) >= bottom(from) => (
            u32::from(to.y) - bottom(from),
            interval_gap(from.x, from.width, to.x, to.width),
        ),
        _ => return None,
    };
    Some((primary, perpendicular, center_distance(from, to)))
}

fn rect_distance(first: Rect, second: Rect) -> (u32, u32) {
    (
        interval_gap(first.x, first.width, second.x, second.width)
            + interval_gap(first.y, first.height, second.y, second.height),
        center_distance(first, second),
    )
}

fn interval_gap(first_start: u16, first_length: u16, second_start: u16, second_length: u16) -> u32 {
    let first_start = u32::from(first_start);
    let first_end = first_start + u32::from(first_length);
    let second_start = u32::from(second_start);
    let second_end = second_start + u32::from(second_length);
    if first_end <= second_start {
        second_start - first_end
    } else {
        first_start.saturating_sub(second_end)
    }
}

fn center_distance(first: Rect, second: Rect) -> u32 {
    let first_x = u32::from(first.x) * 2 + u32::from(first.width);
    let first_y = u32::from(first.y) * 2 + u32::from(first.height);
    let second_x = u32::from(second.x) * 2 + u32::from(second.width);
    let second_y = u32::from(second.y) * 2 + u32::from(second.height);
    first_x.abs_diff(second_x) + first_y.abs_diff(second_y)
}

fn right(rect: Rect) -> u32 {
    u32::from(rect.x) + u32::from(rect.width)
}

fn bottom(rect: Rect) -> u32 {
    u32::from(rect.y) + u32::from(rect.height)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: Size = Size {
        columns: 80,
        rows: 24,
    };

    #[test]
    fn state_transitions_preserve_limits_and_hierarchy() {
        let mut session = Session::new("work".into());
        let mut panes = vec![session.active_pane().unwrap()];
        while panes.len() < MAX_PANES_PER_TAB {
            for pane in panes.clone() {
                session.select_pane(pane);
                panes.push(session.split_active(Split::LeftRight, SIZE).unwrap());
            }
        }
        assert_eq!(session.pane_count(), MAX_PANES_PER_TAB);
        assert_eq!(
            session.split_active(Split::LeftRight, SIZE),
            Err(StateError::PaneLimit)
        );

        while session.tab_count() < MAX_TABS {
            session.create_tab().unwrap();
        }
        assert_eq!(session.create_tab(), Err(StateError::TabLimit));
    }

    #[test]
    fn split_focus_resize_and_close_are_deterministic() {
        let mut session = Session::new("work".into());
        let first = session.active_pane().unwrap();
        let second = session.split_active(Split::LeftRight, SIZE).unwrap();
        assert_eq!(session.focus(Direction::Left, SIZE), Ok(first));

        let before = session.pane_rects(SIZE)[0].1.width;
        session.resize(Direction::Right, SIZE).unwrap();
        assert_eq!(session.pane_rects(SIZE)[0].1.width, before + 1);

        assert_eq!(session.close_active_pane(SIZE), Ok(CloseResult::PaneClosed));
        assert_eq!(session.active_pane(), Some(second));
        assert_eq!(
            session.close_active_pane(SIZE),
            Ok(CloseResult::SessionEmpty)
        );
        assert_eq!(session.active_pane(), None);
    }

    #[test]
    fn failed_split_does_not_change_state() {
        let mut session = Session::new("work".into());
        assert_eq!(
            session.split_active(
                Split::LeftRight,
                Size {
                    columns: 2,
                    rows: 1,
                },
            ),
            Err(StateError::TooSmall)
        );
        assert_eq!(session.pane_count(), 1);
    }

    #[test]
    fn child_exit_can_close_a_non_active_pane() {
        let mut session = Session::new("work".into());
        let first = session.active_pane().unwrap();
        let second = session.split_active(Split::LeftRight, SIZE).unwrap();
        assert!(session.select_pane(first));

        assert_eq!(
            session.close_pane(second, SIZE),
            Ok(CloseResult::PaneClosed)
        );
        assert_eq!(session.active_pane(), Some(first));
        assert_eq!(session.pane_count(), 1);
    }
}
