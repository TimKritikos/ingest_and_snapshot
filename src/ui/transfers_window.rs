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

    //Generare the window and title
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

    if inner.height == 0 { return; } //TODO: actually handle too small windows

    let mut transfers_sorted: Vec<&Transfer> = transfers.iter().collect();
    transfers_sorted.sort_by_key(|t| match t.status {
        TransferStatus::InProgress => 0,
        TransferStatus::NotStarted => 1,
        TransferStatus::Finished   => 2,
    });

    for (i, transfer) in transfers_sorted.iter().enumerate() {
        let y_offset = i as u16 * (ITEM_HEIGHT + ITEM_GAP);
        if y_offset >= inner.height { break; }
        let item_h = ITEM_HEIGHT.min(inner.height - y_offset);
        frame.render_widget(
            TransferItem { transfer },
            Rect { x: inner.x, y: inner.y + y_offset, width: inner.width, height: item_h },
        );
    }
}

struct TransferItem<'a> {
    transfer: &'a Transfer,
}

impl Widget for TransferItem<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 { return; }

        // Setting up constants
        let bytes_done = self.transfer.samples.last().map(|s| s.bytes_done).unwrap_or(0);
        let bg = Color::DarkGray;

        //Create title
        let (badge_inner, badge_fg) = match self.transfer.status {
            TransferStatus::NotStarted => ("not-started", Color::Yellow),
            TransferStatus::InProgress => ("  running  ", Color::Green),
            TransferStatus::Finished   => ("   done    ", Color::LightBlue),
        };

        let bracket_style = Style::default().fg(Color::White).bg(bg).add_modifier(Modifier::BOLD);
        let badge_style   = Style::default().fg(badge_fg).bg(bg).add_modifier(Modifier::BOLD);
        let name_style    = Style::default().fg(Color::White).bg(bg).add_modifier(Modifier::BOLD);
        let stats_style   = Style::default().fg(Color::White).bg(bg).add_modifier(Modifier::BOLD);
        let sep_style     = Style::default().bg(bg);

        let presentage = if self.transfer.bytes_total > 0 {
            bytes_done * 100 / self.transfer.bytes_total
        } else { 0 };

        let mut spans = vec![
            Span::styled("[", bracket_style),
            Span::styled(badge_inner, badge_style),
            Span::styled("]", bracket_style),
            Span::styled(" ", sep_style),
            Span::styled(format!("{:>3}% | ", presentage), stats_style),
            Span::styled(self.transfer.camera_name.as_str(), name_style),
        ];

        let current_speed = derive_current_speed(&self.transfer.samples);
        let average_speed = derive_overall_speed(&self.transfer.samples);
        let maximum_speed = derive_peak_speed(&self.transfer.samples);

        let total_transfer_size = format_bytes(self.transfer.bytes_total);
        let current_transfer_size  = format_bytes(bytes_done);
        let progress = format!(" {:>8} /{:>8} |", current_transfer_size, total_transfer_size);

        // Remaining width after the left spans (badge + camera name)
        let left_width = spans.iter().map(|s| s.width()).sum::<usize>();
        let remaining  = (area.width as usize).saturating_sub(left_width);

        let speed_stats = if matches!(self.transfer.status, TransferStatus::InProgress) {
            let full_stat_str  = format!("  avg {:>8}/s  max {:>8}/s  cur {:>8}/s",
                format_bytes(average_speed), format_bytes(maximum_speed), format_bytes(current_speed));

            if remaining >= progress.len() + full_stat_str.len() {
                full_stat_str
            } else {
                let short_stat_str = format!("  cur {:>8}/s", format_bytes(current_speed));

                if remaining >= progress.len() + short_stat_str.len() {
                    short_stat_str
                } else {
                    String::new()
                }
            }
        } else {
            let stat_str = format!("  avg {:>8}/s", format_bytes(average_speed));
            if remaining >= progress.len() + stat_str.len() {
                stat_str
            } else {
                String::new()
            }
        };

        let right_stats = format!("{}{} ", progress, speed_stats);
        spans.push(Span::styled(format!("{:>width$}", right_stats, width = remaining.max(right_stats.len())), stats_style));

        Paragraph::new(Line::from(spans)).style(Style::default().bg(bg))
            .render(Rect { x: area.x, y: area.y, width: area.width, height: 1 }, buf);

        //Create the graph
        if area.height > 1 {
            let chart_area = Rect { x: area.x, y: area.y + 1, width: area.width, height: area.height - 1 };
            render_braille_chart(buf, chart_area, &self.transfer.samples, self.transfer.bytes_total, &self.transfer.status);
        }
    }
}

fn render_braille_chart(buf: &mut Buffer, area: Rect, samples: &[crate::ui::TransferSample], bytes_total: u64, status: &TransferStatus) {

    //Calculate usefule values
    let width  = area.width  as usize;
    let height = area.height as usize;

    if width == 0 || height == 0 { return; }

    let total_braille_cols = width * 2;
    let total_braille_rows = height * 4;

    let bytes_done = samples.last().map(|s| s.bytes_done).unwrap_or(0);


    let progress_dot_width = match status {
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

    // Convert the sample data to data that maps byte transfer positions to transfer speed
    let intervals: Vec<(u64, u64)> = samples.windows(2).map(|pair| {
        let dt_ms     = pair[1].timestamp_ms.saturating_sub(pair[0].timestamp_ms);
        let db        = pair[1].bytes_done.saturating_sub(pair[0].bytes_done);
        let speed     = if dt_ms > 0 { db * 1000 / dt_ms } else { 0 };
        let mid_bytes = pair[0].bytes_done / 2 + pair[1].bytes_done / 2;
        (mid_bytes, speed)
    }).collect();

    if intervals.is_empty() { return; }

    let max_speed = intervals.iter().map(|&(_, s)| s).max().unwrap_or(1).max(1);

    // Calculate the height of each breille column regardless of if it represents more or less than one sample
    let mut heights = vec![0usize; total_braille_cols];
    for bc in 0..progress_dot_width {

        // Calculate the byte (and by extension precentage) range this dot represents.
        let byte_from = bc as u64 * bytes_total / total_braille_cols as u64;
        let byte_to   = (bc + 1) as u64 * bytes_total / total_braille_cols as u64;

        // Try to get the data to average the samples that fall into this dot
        let mut speed_sum = 0u64;
        let mut sample_count = 0u32;
        for &(mid_byte, speed) in &intervals {
            if mid_byte >= byte_from && mid_byte < byte_to {
                speed_sum += speed;
                sample_count += 1;
            }
        }

        // Nearest-neighbor fallback when no interval falls in this bucket
        let final_speed = if sample_count > 0 {
            speed_sum / sample_count as u64
        } else {
            let byte_mid  = byte_from / 2 + byte_to / 2;

            intervals.iter()
                .min_by_key(|&&(mb, _)| mb.abs_diff(byte_mid))
                .map(|&(_, s)| s)
                .unwrap_or(0)
        };

        heights[bc] = ((final_speed as u128 * total_braille_rows as u128) / max_speed as u128)
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

        // how many dots high is the current character
        let current_braille_dots_height = (height - 1 - row) * 4;

        for character_x in 0..width {
            let braille_dot_left  = character_x * 2;
            let braille_dot_right = character_x * 2 + 1;

            if braille_dot_left >= progress_dot_width { break; }

            let current_char_dot_to_enable_left = heights[braille_dot_left].saturating_sub(current_braille_dots_height).min(4);
            let current_char_dot_to_enable_right = if braille_dot_right < progress_dot_width {
                heights[braille_dot_right].saturating_sub(current_braille_dots_height).min(4)
            } else { 0 };

            let bits = LEFT_BITS[current_char_dot_to_enable_left] | RIGHT_BITS[current_char_dot_to_enable_right];

            if bits == 0 {
                continue;
            }

            let character = char::from_u32(0x2800 + bits as u32).unwrap_or(' ');

            buf[(area.x + character_x as u16, y)]
                .set_char(character)
                .set_style(Style::default().fg(bar_fg).bg(Color::Black));
        }
    }
}

//TODO: If there are enough samples withing a predefined window average those
fn derive_current_speed(samples: &[crate::ui::TransferSample]) -> u64 {
    if samples.len() < 2 {
        return 0;
    }
    let second_last_sample = &samples[samples.len() - 2];
    let last_sample = &samples[samples.len() - 1];
    let time_difference_ms = last_sample.timestamp_ms.saturating_sub(second_last_sample.timestamp_ms);
    if time_difference_ms > 0 {
        last_sample.bytes_done.saturating_sub(second_last_sample.bytes_done) * 1000 / time_difference_ms
    }else{ 0 }
}

fn derive_overall_speed(samples: &[crate::ui::TransferSample]) -> u64 {
    if samples.len() < 2 {
        return 0;
    }
    let transfer_time_window_ms = samples.last().unwrap().timestamp_ms.saturating_sub(samples[0].timestamp_ms);
    let transfer_byte_count = samples.last().unwrap().bytes_done;
    if transfer_time_window_ms > 0 {
        transfer_byte_count * 1000 / transfer_time_window_ms
    }else{ 0 }
}

fn derive_peak_speed(samples: &[crate::ui::TransferSample]) -> u64 {
    samples.windows(2).map(|pair| {
        let time_difference_ms = pair[1].timestamp_ms.saturating_sub(pair[0].timestamp_ms);
        if time_difference_ms > 0 {
            pair[1].bytes_done.saturating_sub(pair[0].bytes_done) * 1000 / time_difference_ms
        } else { 0 }
    }).max().unwrap_or(0)
}

fn format_bytes(bytes: u64) -> String {
    crate::ui::format_bytes(bytes)
}
