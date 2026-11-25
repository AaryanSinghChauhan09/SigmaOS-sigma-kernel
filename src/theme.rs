//! Theme system (Omarchy-inspired theme support).
//!
//! Color scheme and font configuration with runtime application.

use alloc::format;
use alloc::string::String;

/// Color encoding helpers.
pub mod color {
    use alloc::string::String;

    /// Convert hex color string (e.g., "#FF0000") to u32 (0xFF0000).
    pub fn parse_hex(hex: &str) -> Option<u32> {
        let hex = hex.trim_start_matches('#');
        if hex.len() != 6 {
            return None;
        }
        u32::from_str_radix(hex, 16).ok()
    }

    /// Convert u32 color to hex string.
    pub fn to_hex(color: u32) -> String {
        alloc::format!("{:06X}", color & 0xFFFFFF)
    }
}

/// Theme configuration.
#[derive(Debug, Clone)]
pub struct Theme {
    pub name: String,
    pub background_color: u32,
    pub foreground_color: u32,
    pub accent_color: u32,
    pub border_color: u32,
    pub font_name: String,
    pub font_size: u32,
}

impl Theme {
    /// Create a new theme with default colors.
    pub fn new(name: &str) -> Self {
        Self {
            name: String::from(name),
            background_color: 0x1a1a2e, // dark blue-purple
            foreground_color: 0xe0e0e0, // light gray
            accent_color: 0x16213e, // deep blue
            border_color: 0x0f3460, // blue border
            font_name: String::from("default"),
            font_size: 12,
        }
    }

    /// Create a dark theme.
    pub fn dark() -> Self {
        Self {
            name: String::from("dark"),
            background_color: 0x000000,
            foreground_color: 0xFFFFFF,
            accent_color: 0x333333,
            border_color: 0x555555,
            font_name: String::from("monospace"),
            font_size: 12,
        }
    }

    /// Create a light theme.
    pub fn light() -> Self {
        Self {
            name: String::from("light"),
            background_color: 0xFFFFFF,
            foreground_color: 0x000000,
            accent_color: 0xDDDDDD,
            border_color: 0xAAAAAA,
            font_name: String::from("sans-serif"),
            font_size: 12,
        }
    }

    /// Parse theme from TOML-like configuration string.
    ///
    /// Format:
    /// ```text
    /// name = "theme-name"
    /// background = "#1a1a2e"
    /// foreground = "#e0e0e0"
    /// accent = "#16213e"
    /// border = "#0f3460"
    /// font = "default"
    /// font_size = 12
    /// ```
    pub fn parse(config: &str) -> Result<Self, String> {
        let mut theme = Self::new("unnamed");
        for line in config.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim();
                let value = value.trim().trim_matches('"');
                match key {
                    "name" => theme.name = String::from(value),
                    "background" => {
                        theme.background_color = color::parse_hex(value)
                            .ok_or_else(|| format!("invalid background color: {}", value))?;
                    }
                    "foreground" => {
                        theme.foreground_color = color::parse_hex(value)
                            .ok_or_else(|| format!("invalid foreground color: {}", value))?;
                    }
                    "accent" => {
                        theme.accent_color = color::parse_hex(value)
                            .ok_or_else(|| format!("invalid accent color: {}", value))?;
                    }
                    "border" => {
                        theme.border_color = color::parse_hex(value)
                            .ok_or_else(|| format!("invalid border color: {}", value))?;
                    }
                    "font" => theme.font_name = String::from(value),
                    "font_size" => {
                        theme.font_size = value.parse::<u32>()
                            .map_err(|_| format!("invalid font_size: {}", value))?;
                    }
                    _ => {} // ignore unknown keys
                }
            }
        }
        Ok(theme)
    }

    /// Serialize theme to configuration string.
    pub fn serialize(&self) -> String {
        alloc::format!(
            "name = \"{}\"\n\
             background = \"{}\"\n\
             foreground = \"{}\"\n\
             accent = \"{}\"\n\
             border = \"{}\"\n\
             font = \"{}\"\n\
             font_size = {}\n",
            self.name,
            color::to_hex(self.background_color),
            color::to_hex(self.foreground_color),
            color::to_hex(self.accent_color),
            color::to_hex(self.border_color),
            self.font_name,
            self.font_size
        )
    }

    /// Apply theme (this would integrate with window manager/compositor).
    pub fn apply(&self) {
        // In a full implementation, this would:
        // - Update window manager colors
        // - Update compositor palette
        // - Update console colors
        // - Trigger a full redraw
        let _ = self; // placeholder
    }
}

/// Default SigmaOS theme.
pub fn default_theme() -> Theme {
    Theme::new("sigma-default")
}

/// Built-in theme registry.
pub struct ThemeRegistry {
    themes: alloc::vec::Vec<Theme>,
}

impl ThemeRegistry {
    /// Create a new theme registry with built-in themes.
    pub fn new() -> Self {
        let mut themes = alloc::vec::Vec::new();
        themes.push(Theme::new("sigma-default"));
        themes.push(Theme::dark());
        themes.push(Theme::light());
        Self { themes }
    }

    /// Register a custom theme.
    pub fn register(&mut self, theme: Theme) {
        self.themes.push(theme);
    }

    /// Get a theme by name.
    pub fn get(&self, name: &str) -> Option<&Theme> {
        self.themes.iter().find(|t| t.name == name)
    }

    /// List all theme names.
    pub fn list(&self) -> alloc::vec::Vec<&str> {
        self.themes.iter().map(|t| t.name.as_str()).collect()
    }
}
