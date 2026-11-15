//! The logo: the wordmark, and the mark drawn in half blocks, for the TUI and for stderr.

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

use crate::theme::Theme;

const MARK: [&str; 16] = [
    "   ..........   ",
    " .............. ",
    " .....#........ ",
    ".....##.........",
    ".....##.........",
    "...#######......",
    "...#######......",
    ".....##.........",
    ".....##....@@...",
    ".....##....@@...",
    ".....##....@@...",
    "......####.@@...",
    ".......###.@@...",
    " .............. ",
    " .............. ",
    "   ..........   ",
];
pub const MARK_WIDTH: usize = 16;
pub const MARK_HEIGHT: usize = MARK.len() / 2;
const GAP: &str = "   ";

pub fn wordmark(t: &Theme) -> [Span<'static>; 2] {
    [
        Span::styled("tmprl", t.fg.add_modifier(Modifier::BOLD)),
        Span::styled("▌", t.brand_cursor()),
    ]
}

fn mark(colors: [Color; 3]) -> Vec<Line<'static>> {
    let pixel = |b: u8| match b {
        b'.' => Some(colors[0]),
        b'#' => Some(colors[1]),
        b'@' => Some(colors[2]),
        _ => None,
    };
    MARK.chunks(2)
        .map(|pair| {
            let cells = pair[0].bytes().zip(pair[1].bytes());
            Line::from(
                cells
                    .map(|(top, bottom)| match (pixel(top), pixel(bottom)) {
                        (None, None) => Span::raw(" "),
                        (Some(top), None) => Span::styled("▀", Style::new().fg(top)),
                        (None, Some(bottom)) => Span::styled("▄", Style::new().fg(bottom)),
                        (Some(top), Some(bottom)) => {
                            Span::styled("▀", Style::new().fg(top).bg(bottom))
                        }
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect()
}

/// The mark with `side` beside it from its second row, or `side` alone without colour.
pub fn lockup(t: &Theme, side: Vec<Line<'static>>) -> Vec<Line<'static>> {
    let Some(colors) = t.mark_colors() else {
        return side;
    };
    let mark = mark(colors);
    let rows = MARK_HEIGHT.max(side.len() + 1);
    let mut side = side.into_iter();
    (0..rows)
        .map(|row| {
            let mut spans = match mark.get(row) {
                Some(line) => line.spans.clone(),
                None => vec![Span::raw(" ".repeat(MARK_WIDTH))],
            };
            if row > 0
                && let Some(line) = side.next()
            {
                spans.push(Span::raw(GAP));
                spans.extend(line.spans);
            }
            Line::from(spans)
        })
        .collect()
}

pub fn lockup_width(side: &[Line]) -> usize {
    MARK_WIDTH + GAP.len() + side.iter().map(Line::width).max().unwrap_or(0)
}

/// What stderr shows before the TUI starts, and its width: `None` with no colour for the mark.
pub fn splash(
    t: &Theme,
    place: &str,
    status: Vec<Line<'static>>,
    columns: usize,
) -> Option<(usize, Vec<String>)> {
    t.mark_colors()?;
    let room = columns.saturating_sub(MARK_WIDTH + GAP.len() + 1).max(20);
    let mut side = vec![
        Line::from(wordmark(t).to_vec()),
        Line::from(Span::styled(place.to_string(), t.faint)),
        Line::default(),
    ];
    for line in status {
        match line.spans.as_slice() {
            [span] if line.width() > room => side.extend(
                super::wrap(&span.content, room)
                    .into_iter()
                    .map(|part| Line::from(Span::styled(part, span.style))),
            ),
            _ => side.push(line),
        }
    }
    let width = lockup_width(&side);
    Some((width, lockup(t, side).iter().map(ansi).collect()))
}

fn ansi(line: &Line) -> String {
    let mut out = String::new();
    for span in &line.spans {
        let mut codes: Vec<String> = Vec::new();
        if span.style.add_modifier.contains(Modifier::BOLD) {
            codes.push("1".into());
        }
        if span.style.add_modifier.contains(Modifier::DIM) {
            codes.push("2".into());
        }
        codes.extend(span.style.fg.and_then(|c| sgr(c, 30)));
        codes.extend(span.style.bg.and_then(|c| sgr(c, 40)));
        if codes.is_empty() {
            out.push_str(&span.content);
        } else {
            out.push_str(&format!("\x1b[{}m{}\x1b[0m", codes.join(";"), span.content));
        }
    }
    out
}

fn sgr(color: Color, base: u8) -> Option<String> {
    let named = |n: u8| Some((base + n).to_string());
    let bright = |n: u8| Some((base + 60 + n).to_string());
    match color {
        Color::Reset => None,
        Color::Black => named(0),
        Color::Red => named(1),
        Color::Green => named(2),
        Color::Yellow => named(3),
        Color::Blue => named(4),
        Color::Magenta => named(5),
        Color::Cyan => named(6),
        Color::Gray => named(7),
        Color::DarkGray => bright(0),
        Color::LightRed => bright(1),
        Color::LightGreen => bright(2),
        Color::LightYellow => bright(3),
        Color::LightBlue => bright(4),
        Color::LightMagenta => bright(5),
        Color::LightCyan => bright(6),
        Color::White => bright(7),
        Color::Indexed(i) => Some(format!("{};5;{i}", base + 8)),
        Color::Rgb(r, g, b) => Some(format!("{};2;{r};{g};{b}", base + 8)),
    }
}
