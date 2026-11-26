//! Tiling window manager (Hyprland-like).
//!
//! Grid-based tiling layout with focus management. Integrates with
//! the compositor to position windows automatically.

use crate::compositor::{Compositor, PixelFormat, Rect};
use crate::KernelError;
use alloc::vec::Vec;

/// Tiling layout mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TilingMode {
    /// Split screen horizontally (left/right).
    Horizontal,
    /// Split screen vertically (top/bottom).
    Vertical,
    /// Grid layout (2x2, 3x2, etc.).
    Grid,
}

/// Window manager state.
pub struct WindowManager {
    /// Reference to the compositor.
    pub compositor: Compositor,
    /// Current tiling mode.
    pub mode: TilingMode,
    /// Currently focused window id.
    pub focused_id: Option<u32>,
    /// Window IDs in focus order (for cycling).
    pub focus_order: alloc::vec::Vec<u32>,
}

impl WindowManager {
    /// Create a new window manager with a compositor.
    pub fn new(screen_w: u32, screen_h: u32) -> Self {
        Self {
            compositor: Compositor::new(screen_w, screen_h),
            mode: TilingMode::Grid,
            focused_id: None,
            focus_order: alloc::vec::Vec::new(),
        }
    }

    /// Create a new window and add it to the tiling layout.
    pub fn create_window(
        &mut self,
        title: &str,
        width: u32,
        height: u32,
    ) -> Result<u32, KernelError> {
        let id = self
            .compositor
            .add(title, Rect::new(0, 0, width, height), PixelFormat::Xrgb8888);
        self.focus_order.push(id);
        self.focused_id = Some(id);
        self.tile_windows();
        Ok(id)
    }

    /// Remove a window from the layout.
    pub fn remove_window(&mut self, id: u32) {
        self.compositor.remove(id);
        self.focus_order.retain(|&x| x != id);
        if self.focused_id == Some(id) {
            self.focused_id = self.focus_order.last().copied();
        }
        self.tile_windows();
    }

    /// Tile windows in the current layout mode.
    pub fn tile_windows(&mut self) {
        let screen = self.compositor.screen;
        let count = self.compositor.len();

        if count == 0 {
            return;
        }

        // Collect window IDs to avoid borrow conflicts
        let window_ids: Vec<u32> = self.compositor.iter().map(|w| w.id).collect();

        match self.mode {
            TilingMode::Horizontal => {
                // Split screen into equal vertical columns
                let col_width = screen.w / count as u32;
                for (i, &id) in window_ids.iter().enumerate() {
                    let x = i as u32 * col_width;
                    let _ = self.compositor.resize(id, col_width, screen.h);
                    self.compositor.move_window(id, x as i64, 0);
                }
            }
            TilingMode::Vertical => {
                // Split screen into equal horizontal rows
                let row_height = screen.h / count as u32;
                for (i, &id) in window_ids.iter().enumerate() {
                    let y = i as u32 * row_height;
                    let _ = self.compositor.resize(id, screen.w, row_height);
                    self.compositor.move_window(id, 0, y as i64);
                }
            }
            TilingMode::Grid => {
                // Calculate grid dimensions (aim for roughly square) using integer math
                let cols = {
                    let mut c = 1;
                    while c * c < count as u32 {
                        c += 1;
                    }
                    c
                };
                let rows = (count as u32 + cols - 1) / cols;
                let col_width = screen.w / cols;
                let row_height = screen.h / rows;

                for (i, &id) in window_ids.iter().enumerate() {
                    let col = (i as u32) % cols;
                    let row = (i as u32) / cols;
                    let x = col * col_width;
                    let y = row * row_height;
                    let _ = self.compositor.resize(id, col_width, row_height);
                    self.compositor.move_window(id, x as i64, y as i64);
                }
            }
        }
    }

    /// Switch focus to the next window in focus order.
    pub fn focus_next(&mut self) {
        if self.focus_order.is_empty() {
            return;
        }
        if let Some(current) = self.focused_id {
            if let Some(pos) = self.focus_order.iter().position(|&x| x == current) {
                let next_pos = (pos + 1) % self.focus_order.len();
                self.focused_id = Some(self.focus_order[next_pos]);
                self.compositor.focus_window(self.focus_order[next_pos]);
            }
        } else if let Some(&first) = self.focus_order.first() {
            self.focused_id = Some(first);
            self.compositor.focus_window(first);
        }
    }

    /// Switch focus to the previous window in focus order.
    pub fn focus_prev(&mut self) {
        if self.focus_order.is_empty() {
            return;
        }
        if let Some(current) = self.focused_id {
            if let Some(pos) = self.focus_order.iter().position(|&x| x == current) {
                let len = self.focus_order.len();
                let prev_pos = if pos == 0 { len - 1 } else { pos - 1 };
                self.focused_id = Some(self.focus_order[prev_pos]);
                self.compositor.focus_window(self.focus_order[prev_pos]);
            }
        } else if let Some(&last) = self.focus_order.last() {
            self.focused_id = Some(last);
            self.compositor.focus_window(last);
        }
    }

    /// Set the tiling mode and re-tile.
    pub fn set_mode(&mut self, mode: TilingMode) {
        self.mode = mode;
        self.tile_windows();
    }

    /// Get the currently focused window id.
    pub fn focused_window(&self) -> Option<u32> {
        self.focused_id
    }

    /// Get the compositor for direct access.
    pub fn get_compositor(&self) -> &Compositor {
        &self.compositor
    }

    /// Get mutable compositor for direct access.
    pub fn get_compositor_mut(&mut self) -> &mut Compositor {
        &mut self.compositor
    }
}
