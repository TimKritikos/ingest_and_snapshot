use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style, Modifier};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};
use ratatui::buffer::Buffer;
use super::tui_dialog_widgets;
use super::{Transfer, TransferStatus};

const ITEM_HEIGHT: u16 = 4; // 1 title row + 3 chart rows
const ITEM_GAP: u16 = 1;

pub fn render(frame: &mut Frame, area: Rect, transfers: &[Transfer]) {
    let in_progress = transfers.iter().filter(|t| matches!(t.status, TransferStatus::InProgress)).count();
    let finished    = transfers.iter().filter(|t| matches!(t.status, TransferStatus::Finished)).count();
    let waiting     = transfers.iter().filter(|t| matches!(t.status, TransferStatus::NotStarted)).count();

    let title = if transfers.is_empty() {
        "Transfers".to_string()
    } else {
        let mut parts: Vec<String> = Vec::new();
        if in_progress > 0 { parts.push(format!("{} in progress", in_progress)); }
        if waiting > 0     { parts.push(format!("{} waiting",     waiting));     }
        if finished > 0    { parts.push(format!("{} finished",    finished));    }
        format!("Transfers — {}", parts.join("  |  "))
    };

    let block = tui_dialog_widgets::DialogBlock::default().title(&title);
    frame.render_widget(block.clone(), area);
    let inner = block.inner(area);

    if inner.height == 0 { return; }

    let list_y      = inner.y;
    let list_height = inner.height;

    // Incomplete transfers (InProgress, then NotStarted) float to the top
    let mut sorted: Vec<&Transfer> = transfers.iter().collect();
    sorted.sort_by_key(|t| match t.status {
        TransferStatus::InProgress => 0,
        TransferStatus::NotStarted => 1,
        TransferStatus::Finished   => 2,
    });

    for (i, transfer) in sorted.iter().enumerate() {
        let y_offset = i as u16 * (ITEM_HEIGHT + ITEM_GAP);
        if y_offset >= list_height { break; }
        let item_h = ITEM_HEIGHT.min(list_height - y_offset);
        frame.render_widget(
            TransferItem { transfer },
            Rect { x: inner.x, y: list_y + y_offset, width: inner.width, height: item_h },
        );
    }
}

struct TransferItem<'a> {
    transfer: &'a Transfer,
}

impl Widget for TransferItem<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 { return; }

        let t = self.transfer;
        let bytes_done = t.samples.last().map(|&(_, b)| b).unwrap_or(0);
        let bg = Color::DarkGray;

        // Status badge: [ ] brackets in white, inner text in status colour.
        // Inner text padded to "not-started" width (11 chars) so all badges are the same size.
        let (badge_inner, badge_fg) = match t.status {
            TransferStatus::NotStarted => ("not-started", Color::Yellow),
            TransferStatus::InProgress => ("  running  ", Color::Green),
            TransferStatus::Finished   => ("   done    ", Color::LightBlue),
        };

        let bracket_style = Style::default().fg(Color::White).bg(bg).add_modifier(Modifier::BOLD);
        let badge_style   = Style::default().fg(badge_fg).bg(bg).add_modifier(Modifier::BOLD);
        let name_style    = Style::default().fg(Color::White).bg(bg).add_modifier(Modifier::BOLD);
        let stats_style   = Style::default().fg(Color::White).bg(bg);
        let sep_style     = Style::default().bg(bg);

        let mut spans = vec![
            Span::styled("[", bracket_style),
            Span::styled(badge_inner, badge_style),
            Span::styled("]", bracket_style),
            Span::styled(" ", sep_style),
            Span::styled(t.camera_name.as_str(), name_style),
        ];

        if matches!(t.status, TransferStatus::InProgress) {
            let pct   = if t.bytes_total > 0 { bytes_done * 100 / t.bytes_total } else { 0 };
            let speed = derive_current_speed(&t.samples);
            spans.push(Span::styled(
                format!("  {}%  {}/{}  @ {}/s",
                    pct,
                    format_bytes(bytes_done),
                    format_bytes(t.bytes_total),
                    format_bytes(speed)),
                stats_style,
            ));
        } else if matches!(t.status, TransferStatus::Finished) {
            let overall = derive_overall_speed(&t.samples);
            spans.push(Span::styled(
                format!("  avg {}/s", format_bytes(overall)),
                stats_style,
            ));
        }

        Paragraph::new(Line::from(spans)).style(Style::default().bg(bg))
            .render(Rect { x: area.x, y: area.y, width: area.width, height: 1 }, buf);

        if area.height > 1 {
            let chart_area = Rect { x: area.x, y: area.y + 1, width: area.width, height: area.height - 1 };
            render_braille_chart(buf, chart_area, &t.samples, t.bytes_total, &t.status);
        }
    }
}

/// Renders a braille bar chart that doubles as a progress bar.
///
/// The chart width is divided into filled (progress zone) and empty (remaining zone).
/// The filled zone covers `bytes_done / bytes_total` of the total width and contains
/// the speed-over-time braille bars.  A finished transfer fills the whole width.
///
/// Each terminal character = 2 braille columns × 4 braille rows (8 dot positions).
/// Multiple samples may fall in a single braille column (averaged) or a single sample
/// may cover many columns (nearest-neighbor fill) — both cases are handled correctly.
fn render_braille_chart(buf: &mut Buffer, area: Rect, samples: &[(u64, u64)], bytes_total: u64, status: &TransferStatus) {
    let width  = area.width  as usize;
    let height = area.height as usize;
    if width == 0 || height == 0 { return; }

    let total_braille_cols = width * 2;
    let total_braille_rows = height * 4;

    let bytes_done = samples.last().map(|&(_, b)| b).unwrap_or(0);

    // Number of braille columns in the "filled" progress zone.
    // Finished uses the same bytes_done/bytes_total ratio as InProgress — the bar only
    // reaches the full width when all bytes were actually transferred.  The cyan colour
    // already communicates completion without needing to force a stretch.
    let progress_cols = match status {
        TransferStatus::Finished | TransferStatus::InProgress => {
            if bytes_total > 0 {
                ((bytes_done as u128 * total_braille_cols as u128 / bytes_total as u128) as usize)
                    .min(total_braille_cols)
            } else { 0 }
        }
        TransferStatus::NotStarted => 0,
    };

    // Fill the entire chart area with black
    for row in 0..height {
        for col in 0..width {
            buf[(area.x + col as u16, area.y + row as u16)]
                .set_char(' ')
                .set_style(Style::default().bg(Color::Black));
        }
    }

    if samples.len() < 2 || progress_cols == 0 { return; }

    // Compute speed intervals keyed by bytes position rather than timestamp.
    // Each interval is (mid_bytes_done, speed_bps).  Bucketing by bytes_total
    // (which is fixed) means column boundaries never shift as new samples arrive,
    // keeping past data points stable (no wiggling).
    let intervals: Vec<(u64, u64)> = samples.windows(2).map(|pair| {
        let dt_ms     = pair[1].0.saturating_sub(pair[0].0);
        let db        = pair[1].1.saturating_sub(pair[0].1);
        let speed     = if dt_ms > 0 { db * 1000 / dt_ms } else { 0 };
        let mid_bytes = pair[0].1 / 2 + pair[1].1 / 2;
        (mid_bytes, speed)
    }).collect();

    if intervals.is_empty() { return; }

    let max_speed = intervals.iter().map(|&(_, s)| s).max().unwrap_or(1).max(1);

    // Height (braille rows from bottom) for each braille column in the progress zone.
    // Each column maps to a fixed byte range [bc*bytes_total/W, (bc+1)*bytes_total/W).
    let mut heights = vec![0usize; total_braille_cols];
    for bc in 0..progress_cols {
        let b_from = bc as u64 * bytes_total / total_braille_cols as u64;
        let b_to   = (bc + 1) as u64 * bytes_total / total_braille_cols as u64;
        let b_mid  = b_from / 2 + b_to / 2;

        let mut sum   = 0u64;
        let mut count = 0u32;
        for &(mb, speed) in &intervals {
            if mb >= b_from && mb < b_to {
                sum   += speed;
                count += 1;
            }
        }

        // Nearest-neighbor fallback when no interval falls in this bucket
        let speed = if count > 0 {
            sum / count as u64
        } else {
            intervals.iter()
                .min_by_key(|&&(mb, _)| mb.abs_diff(b_mid))
                .map(|&(_, s)| s)
                .unwrap_or(0)
        };

        heights[bc] = ((speed as u128 * total_braille_rows as u128) / max_speed as u128)
            .min(total_braille_rows as u128) as usize;
    }

    // Braille dot bit patterns, indexed by dots filled from the bottom of one column (0–4)
    // Left column:  dot7(0x40), dot3(0x04), dot2(0x02), dot1(0x01) — accumulated bottom-up
    // Right column: dot8(0x80), dot6(0x20), dot5(0x10), dot4(0x08) — accumulated bottom-up
    const LEFT_BITS:  [u8; 5] = [0x00, 0x40, 0x44, 0x46, 0x47];
    const RIGHT_BITS: [u8; 5] = [0x00, 0x80, 0xA0, 0xB0, 0xB8];

    let bar_fg = match status {
        TransferStatus::Finished   => Color::LightBlue,
        TransferStatus::InProgress => Color::Green,
        TransferStatus::NotStarted => Color::Yellow,
    };

    for row in 0..height {
        let y = area.y + row as u16;
        // Braille row index (from chart bottom) of the bottom-most dot in this character row
        let braille_row_start = (height - 1 - row) * 4;

        for col in 0..width {
            let bc_left  = col * 2;
            let bc_right = col * 2 + 1;

            if bc_left >= progress_cols { break; }

            let fl = heights[bc_left].saturating_sub(braille_row_start).min(4);
            let fr = if bc_right < progress_cols {
                heights[bc_right].saturating_sub(braille_row_start).min(4)
            } else { 0 };

            let bits = LEFT_BITS[fl] | RIGHT_BITS[fr];
            if bits == 0 { continue; }

            let ch = char::from_u32(0x2800 + bits as u32).unwrap_or(' ');
            buf[(area.x + col as u16, y)]
                .set_char(ch)
                .set_style(Style::default().fg(bar_fg).bg(Color::Black));
        }
    }
}

fn derive_current_speed(samples: &[(u64, u64)]) -> u64 {
    if samples.len() < 2 { return 0; }
    let (t0, b0) = samples[samples.len() - 2];
    let (t1, b1) = samples[samples.len() - 1];
    let dt_ms = t1.saturating_sub(t0);
    if dt_ms > 0 { b1.saturating_sub(b0) * 1000 / dt_ms } else { 0 }
}

fn derive_overall_speed(samples: &[(u64, u64)]) -> u64 {
    if samples.len() < 2 { return 0; }
    let (t_start, _) = samples[0];
    let (t_end, bytes_done) = samples[samples.len() - 1];
    let dt_ms = t_end.saturating_sub(t_start);
    if dt_ms > 0 { bytes_done * 1000 / dt_ms } else { 0 }
}

fn derive_peak_speed(samples: &[(u64, u64)]) -> u64 {
    samples.windows(2).map(|pair| {
        let dt_ms = pair[1].0.saturating_sub(pair[0].0);
        if dt_ms > 0 { pair[1].1.saturating_sub(pair[0].1) * 1000 / dt_ms } else { 0 }
    }).max().unwrap_or(0)
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.1}GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.1}MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1}KB", bytes as f64 / 1024.0)
    } else {
        format!("{}B", bytes)
    }
}
