//! A small, self-contained fuzzy picker built directly on crossterm.
//!
//! It runs inside a `tmux display-popup` (which provides a pty) and offers two
//! modes. It opens in **Search** mode, so you can start typing immediately.
//!
//! * **Search** — type to fuzzy-filter; the arrow keys still move the
//!   selection.
//! * **Normal** — move with `j`/`k` or the arrow keys, press `/` to search
//!   again, `q` to quit.
//!
//! `Esc` always backs out exactly one level rather than quitting outright:
//! Search → Normal → clear the filter → quit. That way it can never close the
//! picker out from under a search. `Ctrl-c` quits from anywhere.

use std::io::{self, Write as _};
use std::path::Path;

use crossterm::cursor::{Hide, MoveTo, Show};
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::style::{
    Attribute, Color, Print, ResetColor, SetAttribute, SetBackgroundColor, SetForegroundColor,
};
use crossterm::terminal::{
    Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
    enable_raw_mode, size,
};
use crossterm::{execute, queue};
use serde::{Deserialize, Serialize};

/// A single row rendered by the picker. Mirrors the display columns of a
/// `ListEntry` plus the identifiers needed to act on the choice.
#[derive(Clone, Serialize, Deserialize)]
pub struct PickItem {
    pub target: String,
    pub session_id: String,
    pub state: String,
    pub mode: String,
    pub active_tasks: u32,
    pub active_agents: u32,
    pub summary: String,
    pub cwd: String,
    pub session_name: String,
}

const SEARCH_FIELDS: usize = 5;

/// Indices into [`PickItem::search_fields`], used to pull a column's match
/// positions back out of [`Highlights`].
const FIELD_SUMMARY: usize = 0;
const FIELD_CWD: usize = 1;
const FIELD_SESSION: usize = 2;
const FIELD_MODE: usize = 3;
const FIELD_STATE: usize = 4;

impl PickItem {
    /// The fields a query is matched against, in the order `search_text`
    /// concatenates them. [`Highlights`] maps match positions back through this
    /// same order, so the two must stay in step.
    fn search_fields(&self) -> [&str; SEARCH_FIELDS] {
        [
            &self.summary,
            &self.cwd,
            &self.session_name,
            &self.mode,
            &self.state,
        ]
    }

    fn search_text(&self) -> String {
        self.search_fields().join(" ")
    }
}

/// Where a query matched, split per search field.
///
/// Filtering scores the joined [`PickItem::search_text`], so a query can match
/// across a field boundary — `dw` hitting the `d` of a summary and the `w` of a
/// cwd. Matching each column on its own would find nothing in either and render
/// a matching row with no highlight, so positions are computed once and split.
struct Highlights {
    per_field: [Vec<usize>; SEARCH_FIELDS],
}

impl Highlights {
    fn compute(item: &PickItem, query: &str) -> Self {
        let fields = item.search_fields();
        let hits = fuzzy_indices(&item.search_text(), query).unwrap_or_default();

        // `search_text` joins with a single space, so each field starts one
        // character past the end of the previous one.
        let mut starts = [0_usize; SEARCH_FIELDS];
        let mut cursor = 0_usize;
        for (start, field) in starts.iter_mut().zip(fields.iter()) {
            *start = cursor;
            cursor += field.chars().count() + 1;
        }

        let per_field = std::array::from_fn(|idx| {
            let (Some(&start), Some(field)) = (starts.get(idx), fields.get(idx)) else {
                return Vec::new();
            };
            let len = field.chars().count();
            hits.iter()
                .filter_map(|&pos| pos.checked_sub(start).filter(|offset| *offset < len))
                .collect()
        });

        Self { per_field }
    }

    /// Match positions within a single field, relative to that field's start.
    fn field(&self, index: usize) -> &[usize] {
        self.per_field.get(index).map_or(&[], Vec::as_slice)
    }
}

/// Serialize `items` to `path` as JSON so a child process can read them back.
///
/// # Errors
/// Returns an error if serialization or writing the file fails.
pub fn write_items(path: &Path, items: &[PickItem]) -> anyhow::Result<()> {
    let json = serde_json::to_vec(items)?;
    std::fs::write(path, json)?;
    Ok(())
}

/// Read the JSON produced by [`write_items`] back into a vector.
///
/// # Errors
/// Returns an error if the file cannot be read or parsed.
pub fn read_items(path: &Path) -> anyhow::Result<Vec<PickItem>> {
    let bytes = std::fs::read(path)?;
    let items = serde_json::from_slice(&bytes)?;
    Ok(items)
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,
    Search,
}

enum Action {
    Continue,
    Select,
    Quit,
}

/// Mutable picker state, threaded through rendering and input handling.
struct State<'items> {
    items: &'items [PickItem],
    mode: Mode,
    query: String,
    /// Indices into `items`, in display order (best matches first).
    filtered: Vec<usize>,
    /// Cursor position within `filtered`.
    selected: usize,
    /// Index of the first visible row within `filtered`.
    offset: usize,
}

impl<'items> State<'items> {
    fn new(items: &'items [PickItem]) -> Self {
        let filtered = (0..items.len()).collect();
        Self {
            items,
            // Opens ready to type, like fzf.
            mode: Mode::Search,
            query: String::new(),
            filtered,
            selected: 0,
            offset: 0,
        }
    }

    /// Recompute the filtered set from the current query, keeping the highest
    /// scoring matches first and resetting the cursor to the top match.
    fn refilter(&mut self) {
        let mut scored: Vec<(usize, i32)> = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(idx, item)| {
                fuzzy_score(&item.search_text(), &self.query).map(|score| (idx, score))
            })
            .collect();
        scored.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        self.filtered = scored.into_iter().map(|(idx, _)| idx).collect();
        self.selected = 0;
        self.offset = 0;
    }

    const fn move_down(&mut self) {
        let last = self.filtered.len().saturating_sub(1);
        if self.selected < last {
            self.selected += 1;
        }
    }

    const fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    fn clear_query(&mut self) {
        self.query.clear();
        self.refilter();
    }

    /// The `items` index currently under the cursor, if any.
    fn current(&self) -> Option<usize> {
        self.filtered.get(self.selected).copied()
    }
}

/// Run the picker over `items`. Returns the chosen index into `items`, or
/// `None` if the user cancelled.
///
/// # Errors
/// Returns an error if the terminal cannot be put into raw mode or if reading
/// input fails.
pub fn run(items: &[PickItem]) -> anyhow::Result<Option<usize>> {
    if items.is_empty() {
        return Ok(None);
    }

    let _guard = TerminalGuard::enter()?;
    let mut state = State::new(items);

    loop {
        render(&mut state)?;
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        match handle_key(&mut state, key) {
            Action::Continue => {}
            Action::Quit => return Ok(None),
            Action::Select => return Ok(state.current()),
        }
    }
}

/// Restores the terminal to its original state on drop, even on early return.
struct TerminalGuard;

impl TerminalGuard {
    fn enter() -> anyhow::Result<Self> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, Hide)?;
        Ok(Self)
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        drop(execute!(io::stdout(), LeaveAlternateScreen, Show));
        drop(disable_raw_mode());
    }
}

#[allow(clippy::wildcard_enum_match_arm)]
fn handle_key(state: &mut State, key: KeyEvent) -> Action {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return match key.code {
            KeyCode::Char('c') => Action::Quit,
            KeyCode::Char('y') => Action::Select,
            KeyCode::Char('n') => {
                state.move_down();
                Action::Continue
            }
            KeyCode::Char('p') => {
                state.move_up();
                Action::Continue
            }
            KeyCode::Char('u') => {
                state.clear_query();
                Action::Continue
            }
            _ => Action::Continue,
        };
    }
    match state.mode {
        Mode::Normal => handle_normal(state, key.code),
        Mode::Search => handle_search(state, key.code),
    }
}

#[allow(clippy::wildcard_enum_match_arm)]
fn handle_normal(state: &mut State, code: KeyCode) -> Action {
    match code {
        KeyCode::Char('j') | KeyCode::Down => {
            state.move_down();
            Action::Continue
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.move_up();
            Action::Continue
        }
        KeyCode::Char('g') | KeyCode::Home => {
            state.selected = 0;
            Action::Continue
        }
        KeyCode::Char('G') | KeyCode::End => {
            state.selected = state.filtered.len().saturating_sub(1);
            Action::Continue
        }
        KeyCode::Char('/') => {
            state.mode = Mode::Search;
            Action::Continue
        }
        KeyCode::Enter => Action::Select,
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Esc => {
            if state.query.is_empty() {
                Action::Quit
            } else {
                state.clear_query();
                Action::Continue
            }
        }
        _ => Action::Continue,
    }
}

#[allow(clippy::wildcard_enum_match_arm)]
fn handle_search(state: &mut State, code: KeyCode) -> Action {
    match code {
        KeyCode::Char(ch) => {
            state.query.push(ch);
            state.refilter();
            Action::Continue
        }
        KeyCode::Backspace => {
            let _ = state.query.pop();
            state.refilter();
            Action::Continue
        }
        KeyCode::Down => {
            state.move_down();
            Action::Continue
        }
        KeyCode::Up => {
            state.move_up();
            Action::Continue
        }
        KeyCode::Enter => Action::Select,
        KeyCode::Esc => {
            state.mode = Mode::Normal;
            Action::Continue
        }
        _ => Action::Continue,
    }
}

/// Number of rows reserved above (prompt + header) and below (help) the list.
const CHROME_ROWS: u16 = 3;

fn render(state: &mut State) -> io::Result<()> {
    let (cols, rows) = size().unwrap_or((80, 24));
    let width = usize::from(cols);
    let list_height = usize::from(rows.saturating_sub(CHROME_ROWS));

    adjust_offset(state, list_height);

    let mut out = io::stdout();
    queue!(out, Clear(ClearType::All), MoveTo(0, 0))?;

    draw_prompt(&mut out, state, width)?;
    queue!(out, MoveTo(0, 1))?;
    draw_header(&mut out, width)?;
    draw_list(&mut out, state, width, list_height)?;
    draw_help(&mut out, state, rows, width)?;

    out.flush()
}

/// Scroll the visible window so the selected row stays on screen.
const fn adjust_offset(state: &mut State, list_height: usize) {
    if list_height == 0 {
        state.offset = state.selected;
        return;
    }
    if state.selected < state.offset {
        state.offset = state.selected;
    } else if state.selected >= state.offset + list_height {
        state.offset = state.selected + 1 - list_height;
    }
}

/// Column widths, and the gap rendered between them.
const COL_STATE: usize = 7;
const COL_MODE: usize = 11;
const COL_TASKS: usize = 5;
const COL_AGENTS: usize = 6;
const COL_SUMMARY: usize = 40;
const COL_CWD: usize = 25;
const GAP: &str = "  ";
/// Width of the selection gutter drawn at the start of every row.
const GUTTER: &str = "  ";

const MATCH_FG: Color = Color::Green;
const POINTER_FG: Color = Color::Cyan;
const SELECTED_BG: Color = Color::DarkGrey;
/// Foreground for de-emphasised text. Brightened on the selected row so it
/// stays legible against `SELECTED_BG`.
const MUTED: Color = Color::DarkGrey;

#[derive(Clone, Copy)]
struct Style {
    fg: Color,
    bold: bool,
}

impl Style {
    const fn plain(fg: Color) -> Self {
        Self { fg, bold: false }
    }
}

/// A run of text sharing one style. Rows are assembled from these so a single
/// line can mix per-column colours with match highlighting.
struct Span {
    text: String,
    style: Style,
}

impl Span {
    fn new(text: impl Into<String>, style: Style) -> Self {
        Self {
            text: text.into(),
            style,
        }
    }
}

/// Keep muted text readable once the row background is applied.
const fn adapt(color: Color, selected: bool) -> Color {
    if selected && matches!(color, Color::DarkGrey) {
        Color::White
    } else {
        color
    }
}

fn state_color(state: &str, selected: bool) -> Color {
    let base = if state == "active" {
        Color::Green
    } else {
        MUTED
    };
    adapt(base, selected)
}

fn mode_color(mode: &str, selected: bool) -> Color {
    let base = match mode {
        "plan" => Color::Blue,
        "acceptEdits" => Color::Yellow,
        "yolo" => Color::Red,
        _ => MUTED,
    };
    adapt(base, selected)
}

const fn count_color(count: u32, selected: bool) -> Color {
    if count == 0 {
        adapt(MUTED, selected)
    } else {
        Color::Reset
    }
}

/// Write `spans` as one full-width line, truncating to `width` and padding the
/// remainder so the selected row's background extends across the screen.
fn draw_spans(
    out: &mut io::Stdout,
    spans: &[Span],
    width: usize,
    selected: bool,
) -> io::Result<()> {
    queue!(out, ResetColor)?;
    if selected {
        queue!(out, SetBackgroundColor(SELECTED_BG))?;
    }

    let mut used = 0_usize;
    for span in spans {
        if used >= width {
            break;
        }
        let text = cut(&span.text, width - used);
        used += text.chars().count();
        queue!(out, SetForegroundColor(span.style.fg))?;
        if span.style.bold {
            queue!(out, SetAttribute(Attribute::Bold))?;
        }
        queue!(out, Print(text))?;
        if span.style.bold {
            queue!(out, SetAttribute(Attribute::NormalIntensity))?;
        }
    }
    if used < width {
        queue!(out, Print(" ".repeat(width - used)))?;
    }
    queue!(out, ResetColor)
}

fn push_fixed(spans: &mut Vec<Span>, text: &str, width: usize, style: Style) {
    let shown = truncate(text, width);
    let len = shown.chars().count();
    spans.push(Span::new(shown, style));
    if len < width {
        spans.push(Span::new(" ".repeat(width - len), style));
    }
}

/// Append `text` as a fixed-width column, styling the characters the query
/// matched so hits stand out the way they do in fzf.
///
/// `hits` are character positions within the untruncated `text`; any that fall
/// outside the visible width are dropped.
fn push_matched(
    spans: &mut Vec<Span>,
    text: &str,
    width: usize,
    hits: &[usize],
    base: Style,
    selected: bool,
) {
    let shown = truncate(text, width);
    // The ellipsis `truncate` adds is not part of `text`, so it is never a hit.
    let visible = if text.chars().count() > width {
        width.saturating_sub(1)
    } else {
        shown.chars().count()
    };
    let hit_style = Style {
        fg: adapt(MATCH_FG, selected),
        bold: true,
    };

    let mut run = String::new();
    let mut run_is_hit = false;
    for (idx, ch) in shown.chars().enumerate() {
        let is_hit = idx < visible && hits.contains(&idx);
        if is_hit != run_is_hit && !run.is_empty() {
            let style = if run_is_hit { hit_style } else { base };
            spans.push(Span::new(std::mem::take(&mut run), style));
        }
        run_is_hit = is_hit;
        run.push(ch);
    }
    if !run.is_empty() {
        let style = if run_is_hit { hit_style } else { base };
        spans.push(Span::new(run, style));
    }

    let len = shown.chars().count();
    if len < width {
        spans.push(Span::new(" ".repeat(width - len), base));
    }
}

fn row_spans(item: &PickItem, query: &str, selected: bool) -> Vec<Span> {
    let hits = Highlights::compute(item, query);
    let mut spans = Vec::new();
    let gutter = if selected { "\u{258C} " } else { GUTTER };
    spans.push(Span::new(
        gutter,
        Style {
            fg: POINTER_FG,
            bold: true,
        },
    ));

    push_matched(
        &mut spans,
        &item.state,
        COL_STATE,
        hits.field(FIELD_STATE),
        Style::plain(state_color(&item.state, selected)),
        selected,
    );
    spans.push(Span::new(GAP, Style::plain(Color::Reset)));
    push_matched(
        &mut spans,
        &item.mode,
        COL_MODE,
        hits.field(FIELD_MODE),
        Style::plain(mode_color(&item.mode, selected)),
        selected,
    );
    spans.push(Span::new(GAP, Style::plain(Color::Reset)));
    push_fixed(
        &mut spans,
        &format!("{:>COL_TASKS$}", item.active_tasks),
        COL_TASKS,
        Style::plain(count_color(item.active_tasks, selected)),
    );
    spans.push(Span::new(GAP, Style::plain(Color::Reset)));
    push_fixed(
        &mut spans,
        &format!("{:>COL_AGENTS$}", item.active_agents),
        COL_AGENTS,
        Style::plain(count_color(item.active_agents, selected)),
    );
    spans.push(Span::new(GAP, Style::plain(Color::Reset)));
    push_matched(
        &mut spans,
        &item.summary,
        COL_SUMMARY,
        hits.field(FIELD_SUMMARY),
        Style::plain(Color::Reset),
        selected,
    );
    spans.push(Span::new(GAP, Style::plain(Color::Reset)));
    push_matched(
        &mut spans,
        &item.cwd,
        COL_CWD,
        hits.field(FIELD_CWD),
        Style::plain(adapt(Color::Blue, selected)),
        selected,
    );
    spans.push(Span::new(GAP, Style::plain(Color::Reset)));
    push_matched(
        &mut spans,
        &item.session_name,
        item.session_name.chars().count(),
        hits.field(FIELD_SESSION),
        Style::plain(adapt(Color::Magenta, selected)),
        selected,
    );

    spans
}

fn draw_prompt(out: &mut io::Stdout, state: &State, width: usize) -> io::Result<()> {
    let (badge, accent) = match state.mode {
        Mode::Normal => (" NORMAL ", Color::Blue),
        Mode::Search => (" SEARCH ", Color::Green),
    };
    let cursor = if state.mode == Mode::Search {
        "\u{2588}"
    } else {
        ""
    };
    let shown = if state.filtered.is_empty() {
        0
    } else {
        state.selected.saturating_add(1)
    };
    let counter = format!(" {shown}/{} ", state.filtered.len());

    queue!(
        out,
        ResetColor,
        SetBackgroundColor(accent),
        SetForegroundColor(Color::Black),
        SetAttribute(Attribute::Bold),
        Print(badge),
        SetAttribute(Attribute::Reset),
        ResetColor,
        SetForegroundColor(accent),
        Print(" \u{276F} "),
        SetForegroundColor(Color::Reset),
        SetAttribute(Attribute::Bold),
        Print(&state.query),
        SetAttribute(Attribute::Reset),
        SetForegroundColor(accent),
        Print(cursor),
        ResetColor,
    )?;

    let used = badge.chars().count() + 3 + state.query.chars().count() + cursor.chars().count();
    let tail = width.saturating_sub(used);
    let counter_len = counter.chars().count();
    if tail > counter_len {
        queue!(out, Print(" ".repeat(tail - counter_len)))?;
    }
    queue!(
        out,
        SetForegroundColor(MUTED),
        Print(cut(&counter, tail)),
        ResetColor
    )
}

fn draw_header(out: &mut io::Stdout, width: usize) -> io::Result<()> {
    let header = format!(
        "{GUTTER}{:<COL_STATE$}{GAP}{:<COL_MODE$}{GAP}{:>COL_TASKS$}{GAP}{:>COL_AGENTS$}{GAP}{:<COL_SUMMARY$}{GAP}{:<COL_CWD$}{GAP}{}",
        "STATE", "MODE", "TASKS", "AGENTS", "SUMMARY", "CWD", "SESSION"
    );
    queue!(
        out,
        ResetColor,
        SetForegroundColor(MUTED),
        SetAttribute(Attribute::Bold),
        Print(cut(&header, width)),
        SetAttribute(Attribute::Reset),
        ResetColor
    )
}

fn draw_list(
    out: &mut io::Stdout,
    state: &State,
    width: usize,
    list_height: usize,
) -> io::Result<()> {
    if state.filtered.is_empty() {
        queue!(
            out,
            MoveTo(0, 2),
            SetForegroundColor(MUTED),
            SetAttribute(Attribute::Italic),
            Print(cut("  no matches", width)),
            SetAttribute(Attribute::Reset),
            ResetColor
        )?;
        return Ok(());
    }

    for (row, &item_idx) in state
        .filtered
        .iter()
        .enumerate()
        .skip(state.offset)
        .take(list_height)
    {
        let screen_row = u16::try_from(row - state.offset + 2).unwrap_or(u16::MAX);
        queue!(out, MoveTo(0, screen_row))?;
        let Some(item) = state.items.get(item_idx) else {
            continue;
        };
        let selected = row == state.selected;
        let spans = row_spans(item, &state.query, selected);
        draw_spans(out, &spans, width, selected)?;
    }
    Ok(())
}

/// Help hints, rendered as dim text with the key names picked out.
fn draw_help(out: &mut io::Stdout, state: &State, rows: u16, width: usize) -> io::Result<()> {
    // What `esc` does depends on whether there is a filter left to clear, so
    // the hint changes with it.
    let hints: &[(&str, &str)] = match (state.mode, state.query.is_empty()) {
        (Mode::Normal, true) => &[
            ("j/k", "move"),
            ("/", "search"),
            ("enter", "switch"),
            ("esc", "quit"),
        ],
        (Mode::Normal, false) => &[
            ("j/k", "move"),
            ("/", "search"),
            ("enter", "switch"),
            ("esc", "clear"),
            ("q", "quit"),
        ],
        (Mode::Search, _) => &[
            ("type", "filter"),
            ("\u{2191}\u{2193}", "move"),
            ("enter", "switch"),
            ("esc", "normal"),
            ("^u", "clear"),
        ],
    };

    let mut spans = Vec::new();
    spans.push(Span::new(GUTTER, Style::plain(Color::Reset)));
    for (idx, &(key, label)) in hints.iter().enumerate() {
        if idx > 0 {
            spans.push(Span::new("  \u{00b7}  ", Style::plain(MUTED)));
        }
        spans.push(Span::new(
            key,
            Style {
                fg: Color::Reset,
                bold: true,
            },
        ));
        spans.push(Span::new(format!(" {label}"), Style::plain(MUTED)));
    }

    queue!(out, MoveTo(0, rows.saturating_sub(1)))?;
    draw_spans(out, &spans, width, false)
}

/// Truncate `text` to at most `max_chars` characters, adding an ellipsis.
fn truncate(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_owned();
    }
    let keep = max_chars.saturating_sub(1);
    let mut short: String = text.chars().take(keep).collect();
    short.push('\u{2026}');
    short
}

/// Hard-truncate `text` to `max_chars` with no ellipsis, for text that has
/// already been shaped to fit its column.
fn cut(text: &str, max_chars: usize) -> String {
    text.chars().take(max_chars).collect()
}

/// Lowercase `text` one character at a time so the result stays index-aligned
/// with the original, which match highlighting relies on.
fn lower_chars(text: &str) -> Vec<char> {
    text.chars()
        .map(|ch| ch.to_lowercase().next().unwrap_or(ch))
        .collect()
}

/// Character positions in `haystack` that `needle` matched, or `None` if it
/// does not match at all. An empty needle yields no positions.
#[must_use]
pub fn fuzzy_indices(haystack: &str, needle: &str) -> Option<Vec<usize>> {
    if needle.is_empty() {
        return Some(Vec::new());
    }
    let hay = lower_chars(haystack);
    let need = lower_chars(needle);

    let mut hits = Vec::with_capacity(need.len());
    let mut cursor = 0_usize;
    for &wanted in &need {
        let relative = hay.iter().skip(cursor).position(|&ch| ch == wanted)?;
        let pos = cursor + relative;
        hits.push(pos);
        cursor = pos + 1;
    }
    Some(hits)
}

/// Score how well `needle` fuzzy-matches `haystack`, or `None` if any needle
/// character is missing. Higher is better. An empty needle matches everything.
#[must_use]
pub fn fuzzy_score(haystack: &str, needle: &str) -> Option<i32> {
    if needle.is_empty() {
        return Some(0);
    }
    let hay = lower_chars(haystack);
    let need = lower_chars(needle);

    let mut score: i32 = 0;
    let mut cursor: usize = 0;
    let mut previous: Option<usize> = None;

    for &wanted in &need {
        let relative = hay.iter().skip(cursor).position(|&ch| ch == wanted)?;
        let pos = cursor + relative;

        score += 10;
        if let Some(prev) = previous {
            if pos == prev + 1 {
                score += 15;
            } else {
                let gap = (pos - prev - 1).min(10);
                score -= i32::try_from(gap).unwrap_or(10);
            }
        }
        if is_boundary(&hay, pos) {
            score += 8;
        }

        previous = Some(pos);
        cursor = pos + 1;
    }

    Some(score)
}

/// Whether the character at `pos` begins a "word" (start of string or preceded
/// by a common separator), which earns a scoring bonus.
fn is_boundary(hay: &[char], pos: usize) -> bool {
    pos.checked_sub(1)
        .is_none_or(|before| matches!(hay.get(before), Some(' ' | '/' | '-' | '_' | '.')))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(summary: &str, cwd: &str, session: &str) -> PickItem {
        PickItem {
            target: "0:0.0".to_owned(),
            session_id: "sess".to_owned(),
            state: "active".to_owned(),
            mode: "default".to_owned(),
            active_tasks: 0,
            active_agents: 0,
            summary: summary.to_owned(),
            cwd: cwd.to_owned(),
            session_name: session.to_owned(),
        }
    }

    #[test]
    fn fuzzy_empty_needle_matches() {
        assert_eq!(fuzzy_score("anything", ""), Some(0));
    }

    #[test]
    fn fuzzy_missing_char_returns_none() {
        assert_eq!(fuzzy_score("hello", "hxz"), None);
    }

    #[test]
    fn fuzzy_is_case_insensitive() {
        assert!(fuzzy_score("Hello World", "hw").is_some());
    }

    #[test]
    fn fuzzy_subsequence_matches() {
        assert!(fuzzy_score("src/main.rs", "smr").is_some());
    }

    #[test]
    fn fuzzy_consecutive_beats_scattered() {
        let consecutive = fuzzy_score("abcdef", "abc").expect("match");
        let scattered = fuzzy_score("a_b_c_", "abc").expect("match");
        assert!(consecutive > scattered);
    }

    #[test]
    fn fuzzy_boundary_bonus_applies() {
        let boundary = fuzzy_score("foo bar", "b").expect("match");
        let mid = fuzzy_score("foobar", "b").expect("match");
        assert!(boundary > mid);
    }

    #[test]
    fn refilter_excludes_non_matches_and_orders_by_score() {
        let items = vec![
            item("debug parser", "~/dev", "dev"),
            item("write docs", "~/docs", "docs"),
            item("deploy pipeline", "~/work", "ops"),
        ];
        let mut state = State::new(&items);
        state.selected = 2;
        state.query = "dep".to_owned();
        state.refilter();

        // "write docs" has no 'dep' subsequence, so it is dropped.
        assert_eq!(state.filtered.len(), 2);
        // Cursor snaps back to the top match after re-filtering.
        assert_eq!(state.selected, 0);
        // "deploy" matches "dep" consecutively; "debug parser" only scattered.
        let top = state.filtered.first().copied().expect("match");
        assert_eq!(items.get(top).expect("item").summary, "deploy pipeline");
    }

    fn ctrl(ch: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(ch), KeyModifiers::CONTROL)
    }

    #[test]
    fn starts_in_search_mode() {
        let items = vec![item("a", "a", "a")];
        let state = State::new(&items);
        assert!(state.mode == Mode::Search);
    }

    #[test]
    fn esc_in_search_returns_to_normal_keeping_filter() {
        let items = vec![item("deploy", "~/w", "ops"), item("write", "~/d", "docs")];
        let mut state = State::new(&items);
        state.query = "dep".to_owned();
        state.refilter();

        let action = handle_search(&mut state, KeyCode::Esc);
        assert!(matches!(action, Action::Continue));
        assert!(state.mode == Mode::Normal);
        assert_eq!(state.query, "dep");
        assert_eq!(state.filtered.len(), 1);
    }

    #[test]
    fn esc_in_normal_clears_filter_before_quitting() {
        let items = vec![item("deploy", "~/w", "ops"), item("write", "~/d", "docs")];
        let mut state = State::new(&items);
        state.mode = Mode::Normal;
        state.query = "dep".to_owned();
        state.refilter();

        // First esc clears the filter rather than quitting.
        let first = handle_normal(&mut state, KeyCode::Esc);
        assert!(matches!(first, Action::Continue));
        assert!(state.query.is_empty());
        assert_eq!(state.filtered.len(), 2);

        // Only once there is nothing left to undo does esc quit.
        let second = handle_normal(&mut state, KeyCode::Esc);
        assert!(matches!(second, Action::Quit));
    }

    #[test]
    fn q_quits_immediately_even_with_a_filter() {
        let items = vec![item("deploy", "~/w", "ops")];
        let mut state = State::new(&items);
        state.mode = Mode::Normal;
        state.query = "dep".to_owned();
        assert!(matches!(
            handle_normal(&mut state, KeyCode::Char('q')),
            Action::Quit
        ));
    }

    #[test]
    fn ctrl_y_selects() {
        let items = vec![item("a", "a", "a")];
        let mut state = State::new(&items);
        assert!(matches!(handle_key(&mut state, ctrl('y')), Action::Select));
    }

    #[test]
    fn ctrl_n_and_p_move_in_search_mode() {
        let items = vec![item("a", "a", "a"), item("b", "b", "b")];
        let mut state = State::new(&items);
        assert!(state.mode == Mode::Search);

        let _ = handle_key(&mut state, ctrl('n'));
        assert_eq!(state.selected, 1);
        let _ = handle_key(&mut state, ctrl('p'));
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn ctrl_u_clears_query_without_leaving_search() {
        let items = vec![item("deploy", "~/w", "ops"), item("write", "~/d", "docs")];
        let mut state = State::new(&items);
        state.query = "dep".to_owned();
        state.refilter();

        let _ = handle_key(&mut state, ctrl('u'));
        assert!(state.query.is_empty());
        assert!(state.mode == Mode::Search);
        assert_eq!(state.filtered.len(), 2);
    }

    #[test]
    fn move_clamps_at_bounds() {
        let items = vec![item("a", "a", "a"), item("b", "b", "b")];
        let mut state = State::new(&items);
        state.move_up();
        assert_eq!(state.selected, 0);
        state.move_down();
        state.move_down();
        state.move_down();
        assert_eq!(state.selected, 1);
    }

    #[test]
    fn adjust_offset_scrolls_into_view() {
        let items: Vec<PickItem> = (0..20).map(|_| item("x", "x", "x")).collect();
        let mut state = State::new(&items);
        state.selected = 15;
        adjust_offset(&mut state, 5);
        assert!(state.offset <= 15);
        assert!(15 < state.offset + 5);
    }

    #[test]
    fn truncate_adds_ellipsis() {
        let out = truncate("hello world", 8);
        assert_eq!(out.chars().count(), 8);
        assert!(out.ends_with('\u{2026}'));
    }

    #[test]
    fn cut_clamps_without_ellipsis() {
        assert_eq!(cut("hello", 3), "hel");
        assert_eq!(cut("hi", 5), "hi");
    }

    #[test]
    fn fuzzy_indices_are_positions_in_the_haystack() {
        let hits = fuzzy_indices("deploy", "dpy").expect("match");
        assert_eq!(hits, vec![0, 2, 5]);
    }

    #[test]
    fn fuzzy_indices_none_when_absent() {
        assert!(fuzzy_indices("deploy", "dz").is_none());
    }

    #[test]
    fn fuzzy_indices_empty_query_has_no_hits() {
        assert_eq!(fuzzy_indices("deploy", ""), Some(Vec::new()));
    }

    #[test]
    fn fuzzy_indices_align_with_uppercase_source() {
        // Indices must address the original characters, not a re-cased copy.
        // Matching is greedy, so 'p' resolves to "De[p]loy", not "[P]ipe".
        let hits = fuzzy_indices("DeployPipe", "dp").expect("match");
        assert_eq!(hits, vec![0, 2]);
    }

    #[test]
    fn push_matched_splits_hits_into_runs() {
        let mut spans = Vec::new();
        push_matched(
            &mut spans,
            "deploy",
            6,
            &[0, 1, 2],
            Style::plain(Color::Reset),
            false,
        );
        let rendered: String = spans.iter().map(|sp| sp.text.as_str()).collect();
        assert_eq!(rendered, "deploy");
        // "dep" is one contiguous hit run, then the remainder.
        assert_eq!(spans.len(), 2);
        assert_eq!(spans.first().expect("span").text, "dep");
        assert!(spans.first().expect("span").style.bold);
        assert!(!spans.get(1).expect("span").style.bold);
    }

    #[test]
    fn highlights_span_a_field_boundary() {
        // "dw" spans two fields: 'd' in the summary, 'w' in the cwd.
        let it = item("deploy", "~/work", "ops");
        assert!(
            fuzzy_score(&it.search_text(), "dw").is_some(),
            "row is shown"
        );

        let hits = Highlights::compute(&it, "dw");
        assert_eq!(hits.field(FIELD_SUMMARY), [0]);
        assert_eq!(hits.field(FIELD_CWD), [2]);
        assert!(hits.field(FIELD_SESSION).is_empty());
    }

    #[test]
    fn highlights_are_local_to_each_field() {
        // Positions are field-relative: the 'o' of "ops" sits at index 14 of
        // `search_text` but index 0 here.
        let hits = Highlights::compute(&item("deploy", "~/work", "ops"), "o");
        assert_eq!(hits.field(FIELD_SUMMARY), [4]);
        assert!(hits.field(FIELD_SESSION).is_empty());
    }

    #[test]
    fn highlights_empty_for_non_matching_query() {
        let hits = Highlights::compute(&item("deploy", "~/work", "ops"), "zzz");
        assert!(hits.field(FIELD_SUMMARY).is_empty());
        assert!(hits.field(FIELD_STATE).is_empty());
    }

    #[test]
    fn push_matched_drops_hits_past_the_truncation_point() {
        // A hit under the ellipsis must not highlight, or shift what is visible.
        let mut spans = Vec::new();
        push_matched(
            &mut spans,
            "abcdefghij",
            5,
            &[0, 9],
            Style::plain(Color::Reset),
            false,
        );
        let rendered: String = spans.iter().map(|sp| sp.text.as_str()).collect();
        assert_eq!(rendered, "abcd\u{2026}");
        assert_eq!(spans.first().expect("span").text, "a");
        assert!(spans.first().expect("span").style.bold);
        assert!(!spans.get(1).expect("span").style.bold);
    }

    #[test]
    fn roundtrip_items() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("items.json");
        let items = vec![item("summary", "~/p", "sess")];
        write_items(&path, &items).expect("write");
        let read = read_items(&path).expect("read");
        assert_eq!(read.len(), 1);
        assert_eq!(read.first().expect("item").summary, "summary");
    }
}
