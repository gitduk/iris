use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Position};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::tui::App;

/// Convert `tui_markdown` output (ratatui-core types) into ratatui 0.29 types.
fn md_to_lines(content: &str) -> Vec<Line<'static>> {
    let rendered = tui_markdown::from_str(content);
    rendered
        .lines
        .into_iter()
        .map(|line| {
            let spans: Vec<Span<'static>> = line
                .spans
                .into_iter()
                .map(|s| Span::styled(s.content.into_owned(), convert_style(s.style)))
                .collect();
            Line::from(spans)
        })
        .collect()
}

fn convert_style(s: ratatui_core::style::Style) -> Style {
    let mut out = Style::default();
    if let Some(c) = s.fg {
        out.fg = Some(convert_color(c));
    }
    if let Some(c) = s.bg {
        out.bg = Some(convert_color(c));
    }
    out.add_modifier = Modifier::from_bits_truncate(s.add_modifier.bits());
    out.sub_modifier = Modifier::from_bits_truncate(s.sub_modifier.bits());
    out
}

fn convert_color(c: ratatui_core::style::Color) -> Color {
    use ratatui_core::style::Color as C;
    match c {
        C::Reset => Color::Reset,
        C::Black => Color::Black,
        C::Red => Color::Red,
        C::Green => Color::Green,
        C::Yellow => Color::Yellow,
        C::Blue => Color::Blue,
        C::Magenta => Color::Magenta,
        C::Cyan => Color::Cyan,
        C::Gray => Color::Gray,
        C::DarkGray => Color::DarkGray,
        C::LightRed => Color::LightRed,
        C::LightGreen => Color::LightGreen,
        C::LightYellow => Color::LightYellow,
        C::LightBlue => Color::LightBlue,
        C::LightMagenta => Color::LightMagenta,
        C::LightCyan => Color::LightCyan,
        C::White => Color::White,
        C::Rgb(r, g, b) => Color::Rgb(r, g, b),
        C::Indexed(i) => Color::Indexed(i),
    }
}

const SPINNER: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub fn draw(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(f.area());

    draw_chat(f, app, chunks[0]);
    draw_status(f, app, chunks[1]);
}

fn draw_status(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let a = &app.status.affect;
    let text = format!(
        " {}  |  energy {:.0}%  valence {:.0}%  arousal {:.0}%",
        app.status.mode,
        a.energy * 100.0,
        a.valence * 100.0,
        a.arousal * 100.0
    );

    let para = Paragraph::new(Line::from(Span::styled(
        text,
        Style::default().fg(Color::DarkGray),
    )));
    f.render_widget(para, area);
}

fn draw_chat(f: &mut Frame, app: &App, area: ratatui::layout::Rect) {
    let mut lines: Vec<Line> = Vec::new();
    for msg in &app.messages {
        // Blank line before You messages (separates from previous Iris reply)
        if !lines.is_empty() && msg.role == "You" {
            lines.push(Line::default());
        }
        if msg.role == "You" {
            lines.push(Line::from(vec![Span::raw("> "), Span::raw(&msg.content)]));
        } else {
            lines.extend(md_to_lines(&msg.content));
        }
    }
    if app.thinking {
        let frame = SPINNER[app.anim_frame % SPINNER.len()];
        lines.push(Line::from(Span::styled(
            format!("{frame} thinking..."),
            Style::default().dim(),
        )));
    }

    // Current input line
    if !lines.is_empty() {
        lines.push(Line::default());
    }
    let input_prefix = "> ";
    lines.push(Line::from(vec![
        Span::raw(input_prefix),
        Span::raw(&app.input),
    ]));

    // Inner width = area minus left/right borders
    let inner_w = area.width.saturating_sub(2) as usize;

    // Count wrapped visual rows for all lines
    let wrapped_total: u16 = lines.iter().map(|l| wrapped_line_count(l, inner_w)).sum();
    let visible = area.height.saturating_sub(2); // top/bottom border
    let scroll = wrapped_total.saturating_sub(visible);
    let scroll = scroll.saturating_sub(app.scroll_offset);

    let block = Block::default().borders(Borders::ALL).title(" iris ");
    let para = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((scroll, 0));
    f.render_widget(para, area);

    // Cursor position accounting for wrap
    let before_cursor = &app.input[..app.cursor];
    let cursor_visual_w = input_prefix.width() + before_cursor.width();
    let cursor_row_in_input = if inner_w > 0 {
        cursor_visual_w / inner_w
    } else {
        0
    };
    let cursor_col_in_input = if inner_w > 0 {
        cursor_visual_w % inner_w
    } else {
        0
    };

    // Row of the input line's first wrapped row (after scroll)
    let input_line_str = format!("{}{}", input_prefix, app.input);
    let input_first_row = wrapped_total.saturating_sub(greedy_wrap_rows(&input_line_str, inner_w));
    let abs_row = input_first_row + cursor_row_in_input as u16;
    let vis_row = abs_row.saturating_sub(scroll);

    f.set_cursor_position(Position::new(
        area.x + 1 + cursor_col_in_input as u16,
        area.y + 1 + vis_row,
    ));
}

/// How many visual rows a Line occupies when wrapped to `width` columns.
/// Simulates ratatui's greedy word-wrap by advancing char-by-char.
fn wrapped_line_count(line: &Line, width: usize) -> u16 {
    if width == 0 {
        return 1;
    }
    let full: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    full.split('\n')
        .map(|sub| greedy_wrap_rows(sub, width))
        .sum()
}

/// Count visual rows for a single unwrapped string segment using greedy wrap.
/// Each character is placed on the current row; if it doesn't fit, a new row starts.
fn greedy_wrap_rows(s: &str, width: usize) -> u16 {
    if width == 0 {
        return 1;
    }
    let mut rows: u16 = 1;
    let mut col: usize = 0;
    for ch in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        if cw == 0 {
            continue;
        }
        if col + cw > width {
            rows += 1;
            col = cw;
        } else {
            col += cw;
        }
    }
    rows
}
