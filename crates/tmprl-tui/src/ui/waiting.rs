//! The card in front of the screen while `gf` waits on the codec or the resolver.

use ratatui::Frame;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use super::brand;
use crate::app::SourceWait;
use crate::theme::Theme;

const DOTS: [&str; 3] = ["●○○", "○●○", "○○●"];
pub const FRAME_MS: u64 = 250;

pub(super) fn dots(elapsed_ms: i64) -> &'static str {
    DOTS[(elapsed_ms.max(0) / FRAME_MS as i64 % 3) as usize]
}

pub fn render(frame: &mut Frame, wait: &SourceWait, now_ms: i64, t: &Theme) {
    let elapsed = (now_ms - wait.since_ms).max(0);
    let secs = elapsed / 1000;
    let dots = dots(elapsed);
    let stage = match &wait.asking {
        None => "decoding the payloads".to_string(),
        Some(program) => format!("asking {program}"),
    };
    let side = vec![
        Line::from(brand::wordmark(t).to_vec()),
        Line::from(vec![
            Span::styled("finding the source of ", t.dim),
            Span::styled(format!("`{}`", wait.subject), t.fg),
        ]),
        Line::default(),
        Line::from(vec![
            Span::styled(dots, t.accent),
            Span::styled(format!("  {stage}"), t.fg),
            Span::styled(format!("  {secs}s"), t.faint),
        ]),
        Line::default(),
        Line::from(Span::styled("Esc to stop", t.faint)),
    ];
    let width = brand::lockup_width(&side) as u16 + 6;
    let lines: Vec<Line> = brand::lockup(t, side)
        .into_iter()
        .map(|mut l| {
            l.spans.insert(0, Span::raw("  "));
            l
        })
        .collect();
    let area = super::centered(frame.area(), width, lines.len() as u16 + 2);
    if area.height < 3 || area.width < 20 {
        return;
    }
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .borders(Borders::ALL)
                .title(Span::styled(
                    " source ",
                    t.accent.add_modifier(Modifier::BOLD),
                ))
                .border_style(t.accent),
        ),
        area,
    );
}
