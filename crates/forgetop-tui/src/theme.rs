//! Colour themes and status colours.
//!
//! We use **256-colour indexed** palettes (`Color::Indexed`) rather than 24-bit
//! `Color::Rgb`. Truecolor isn't supported by every terminal (notably macOS
//! Terminal.app), and when it isn't, RGB backgrounds collapse to a muddy wash.
//! Indexed colours render correctly on any 256-colour terminal and adapt to the
//! user's palette, so the UI stays crisp and colourful everywhere.

use forgetop_core::domain::{CheckStatus, PipelineRunStatus};
use ratatui::style::Color;

pub const THEMES: [&str; 4] = ["slate", "dark", "light", "matrix"];

pub struct Theme {
    pub name: &'static str,
    pub bg: Color,
    pub panel: Color,
    pub fg: Color,
    pub dim: Color,
    pub sel_bg: Color,
    pub accent: Color,
    pub green: Color,
    pub red: Color,
    pub blue: Color,
    pub yellow: Color,
    pub magenta: Color,
    pub cyan: Color,
}

impl Theme {
    pub fn by_name(name: &str) -> Theme {
        match name {
            "dark" => Theme {
                name: "dark",
                bg: Color::Indexed(232),
                panel: Color::Indexed(234),
                fg: Color::Indexed(253),
                dim: Color::Indexed(244),
                sel_bg: Color::Indexed(238),
                accent: Color::Indexed(39),
                green: Color::Indexed(84),
                red: Color::Indexed(203),
                blue: Color::Indexed(39),
                yellow: Color::Indexed(221),
                magenta: Color::Indexed(177),
                cyan: Color::Indexed(80),
            },
            "light" => Theme {
                name: "light",
                bg: Color::Indexed(255),
                panel: Color::Indexed(254),
                fg: Color::Indexed(236),
                dim: Color::Indexed(245),
                sel_bg: Color::Indexed(252),
                accent: Color::Indexed(26),
                green: Color::Indexed(28),
                red: Color::Indexed(160),
                blue: Color::Indexed(26),
                yellow: Color::Indexed(136),
                magenta: Color::Indexed(90),
                cyan: Color::Indexed(30),
            },
            "matrix" => Theme {
                name: "matrix",
                bg: Color::Indexed(16),
                panel: Color::Indexed(233),
                fg: Color::Indexed(46),
                dim: Color::Indexed(28),
                sel_bg: Color::Indexed(22),
                accent: Color::Indexed(46),
                green: Color::Indexed(46),
                red: Color::Indexed(203),
                blue: Color::Indexed(48),
                yellow: Color::Indexed(190),
                magenta: Color::Indexed(85),
                cyan: Color::Indexed(51),
            },
            // slate — default, a calm dark theme
            _ => Theme {
                name: "slate",
                bg: Color::Indexed(234),
                panel: Color::Indexed(236),
                fg: Color::Indexed(253),
                dim: Color::Indexed(245),
                sel_bg: Color::Indexed(239),
                accent: Color::Indexed(75),
                green: Color::Indexed(114),
                red: Color::Indexed(210),
                blue: Color::Indexed(75),
                yellow: Color::Indexed(222),
                magenta: Color::Indexed(176),
                cyan: Color::Indexed(80),
            },
        }
    }

    pub fn next(name: &str) -> &'static str {
        let idx = THEMES.iter().position(|t| *t == name).unwrap_or(0);
        THEMES[(idx + 1) % THEMES.len()]
    }

    /// Green = succeeded, blue = actively running, red = failed (needs a look), grey =
    /// waiting/neutral (queued, canceled). Yellow is the one partial-success exception.
    pub fn pipeline_color(&self, status: PipelineRunStatus) -> Color {
        match status {
            PipelineRunStatus::Succeeded => self.green,
            PipelineRunStatus::Running => self.blue,
            PipelineRunStatus::Failed => self.red,
            PipelineRunStatus::PartiallySucceeded => self.yellow,
            PipelineRunStatus::Queued | PipelineRunStatus::Canceled => self.dim,
        }
    }

    pub fn check_color(&self, status: CheckStatus) -> Color {
        match status {
            CheckStatus::Passed => self.green,
            CheckStatus::Failed => self.red,
            CheckStatus::Pending => self.yellow,
            CheckStatus::None => self.dim,
        }
    }
}

pub fn pipeline_icon(status: PipelineRunStatus) -> &'static str {
    match status {
        PipelineRunStatus::Succeeded => "✓",
        PipelineRunStatus::Running => "◐",
        PipelineRunStatus::Queued => "◔",
        PipelineRunStatus::Failed => "✗",
        PipelineRunStatus::PartiallySucceeded => "▲",
        PipelineRunStatus::Canceled => "⊘",
    }
}

/// Frames of the "running" spinner — a rotating disc, advanced by the animation tick.
pub const SPINNER: [&str; 4] = ["◐", "◓", "◑", "◒"];

/// Like [`pipeline_icon`], but a *running* pipeline spins through [`SPINNER`] using
/// `frame` (the app's animation counter), so it reads as live rather than static.
pub fn pipeline_glyph(status: PipelineRunStatus, frame: usize) -> &'static str {
    match status {
        PipelineRunStatus::Running => SPINNER[frame % SPINNER.len()],
        other => pipeline_icon(other),
    }
}

pub fn check_icon(status: CheckStatus) -> &'static str {
    match status {
        CheckStatus::Passed => "✓",
        CheckStatus::Failed => "✗",
        CheckStatus::Pending => "◐",
        CheckStatus::None => "·",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_pipeline_spins_others_are_static() {
        // Running cycles through the spinner frames and wraps.
        assert_eq!(pipeline_glyph(PipelineRunStatus::Running, 0), SPINNER[0]);
        assert_eq!(pipeline_glyph(PipelineRunStatus::Running, 1), SPINNER[1]);
        assert_eq!(pipeline_glyph(PipelineRunStatus::Running, SPINNER.len()), SPINNER[0]);
        assert_ne!(
            pipeline_glyph(PipelineRunStatus::Running, 0),
            pipeline_glyph(PipelineRunStatus::Running, 1),
            "the frame actually changes"
        );
        // Non-running statuses ignore the frame and match the static icon.
        assert_eq!(pipeline_glyph(PipelineRunStatus::Failed, 3), pipeline_icon(PipelineRunStatus::Failed));
        assert_eq!(pipeline_glyph(PipelineRunStatus::Succeeded, 7), pipeline_icon(PipelineRunStatus::Succeeded));
    }

    #[test]
    fn pipeline_colours_follow_the_green_red_grey_model() {
        let t = Theme::by_name("slate");
        assert_eq!(t.pipeline_color(PipelineRunStatus::Succeeded), t.green);
        assert_eq!(t.pipeline_color(PipelineRunStatus::Failed), t.red);
        assert_eq!(t.pipeline_color(PipelineRunStatus::Running), t.blue, "actively running is blue");
        // Waiting / neutral states are grey.
        assert_eq!(t.pipeline_color(PipelineRunStatus::Queued), t.dim);
        assert_eq!(t.pipeline_color(PipelineRunStatus::Canceled), t.dim);
    }
}
