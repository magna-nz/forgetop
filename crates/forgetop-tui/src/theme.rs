//! True-colour themes and status colours.

use forgetop_core::domain::{CheckStatus, PipelineRunStatus};
use ratatui::style::Color;

pub const THEMES: [&str; 4] = ["slate", "dark", "light", "matrix"];

pub struct Theme {
    pub name: &'static str,
    pub bg: Color,
    pub fg: Color,
    pub dim: Color,
    pub sel_bg: Color,
    pub sel_fg: Color,
    pub accent: Color,
    pub green: Color,
    pub red: Color,
    pub blue: Color,
    pub yellow: Color,
}

impl Theme {
    pub fn by_name(name: &str) -> Theme {
        match name {
            "dark" => Theme {
                name: "dark",
                bg: Color::Rgb(0x10, 0x10, 0x14),
                fg: Color::Rgb(0xd0, 0xd0, 0xd0),
                dim: Color::Rgb(0x70, 0x70, 0x7a),
                sel_bg: Color::Rgb(0x2a, 0x2a, 0x36),
                sel_fg: Color::Rgb(0xff, 0xff, 0xff),
                accent: Color::Rgb(0x7a, 0xa2, 0xf7),
                green: Color::Rgb(0x9e, 0xce, 0x6a),
                red: Color::Rgb(0xf7, 0x76, 0x8e),
                blue: Color::Rgb(0x7a, 0xa2, 0xf7),
                yellow: Color::Rgb(0xe0, 0xaf, 0x68),
            },
            "light" => Theme {
                name: "light",
                bg: Color::Rgb(0xec, 0xef, 0xf4),
                fg: Color::Rgb(0x1f, 0x24, 0x30),
                dim: Color::Rgb(0x6b, 0x72, 0x80),
                sel_bg: Color::Rgb(0xcf, 0xd8, 0xe3),
                sel_fg: Color::Rgb(0x1f, 0x24, 0x30),
                accent: Color::Rgb(0x1e, 0x66, 0xf5),
                green: Color::Rgb(0x40, 0xa0, 0x2b),
                red: Color::Rgb(0xd2, 0x0f, 0x39),
                blue: Color::Rgb(0x1e, 0x66, 0xf5),
                yellow: Color::Rgb(0xdf, 0x8e, 0x1d),
            },
            "matrix" => Theme {
                name: "matrix",
                bg: Color::Rgb(0x00, 0x00, 0x00),
                fg: Color::Rgb(0x39, 0xff, 0x14),
                dim: Color::Rgb(0x1f, 0x6f, 0x1f),
                sel_bg: Color::Rgb(0x0f, 0x3d, 0x0f),
                sel_fg: Color::Rgb(0xa9, 0xff, 0x9a),
                accent: Color::Rgb(0x39, 0xff, 0x14),
                green: Color::Rgb(0x39, 0xff, 0x14),
                red: Color::Rgb(0xff, 0x5c, 0x5c),
                blue: Color::Rgb(0x5c, 0xff, 0xd4),
                yellow: Color::Rgb(0xd4, 0xff, 0x5c),
            },
            // slate (Catppuccin-ish) — default
            _ => Theme {
                name: "slate",
                bg: Color::Rgb(0x1e, 0x1e, 0x2e),
                fg: Color::Rgb(0xcd, 0xd6, 0xf4),
                dim: Color::Rgb(0x6c, 0x70, 0x86),
                sel_bg: Color::Rgb(0x31, 0x32, 0x44),
                sel_fg: Color::Rgb(0xcd, 0xd6, 0xf4),
                accent: Color::Rgb(0x89, 0xb4, 0xfa),
                green: Color::Rgb(0xa6, 0xe3, 0xa1),
                red: Color::Rgb(0xf3, 0x8b, 0xa8),
                blue: Color::Rgb(0x89, 0xb4, 0xfa),
                yellow: Color::Rgb(0xf9, 0xe2, 0xaf),
            },
        }
    }

    pub fn next(name: &str) -> &'static str {
        let idx = THEMES.iter().position(|t| *t == name).unwrap_or(0);
        THEMES[(idx + 1) % THEMES.len()]
    }

    pub fn pipeline_color(&self, status: PipelineRunStatus) -> Color {
        match status {
            PipelineRunStatus::Succeeded => self.green,
            PipelineRunStatus::Running | PipelineRunStatus::Queued => self.blue,
            PipelineRunStatus::Failed => self.red,
            PipelineRunStatus::PartiallySucceeded => self.yellow,
            PipelineRunStatus::Canceled => self.dim,
        }
    }

    pub fn check_color(&self, status: CheckStatus) -> Color {
        match status {
            CheckStatus::Passed => self.green,
            CheckStatus::Failed => self.red,
            CheckStatus::Pending => self.blue,
            CheckStatus::None => self.dim,
        }
    }
}

pub fn pipeline_icon(status: PipelineRunStatus) -> &'static str {
    match status {
        PipelineRunStatus::Succeeded => "✓",
        PipelineRunStatus::Running | PipelineRunStatus::Queued => "●",
        PipelineRunStatus::Failed => "✗",
        PipelineRunStatus::PartiallySucceeded => "▲",
        PipelineRunStatus::Canceled => "⊘",
    }
}

pub fn check_icon(status: CheckStatus) -> &'static str {
    match status {
        CheckStatus::Passed => "✓",
        CheckStatus::Failed => "✗",
        CheckStatus::Pending => "●",
        CheckStatus::None => "·",
    }
}
