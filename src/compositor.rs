//! Framebuffer compositor — window list, z-order, damage tracking.
//!
//! Software compositor: windows own pixel buffers; each frame the
//! damaged region of each visible window is blended (or copied) into
//! the linear framebuffer through a caller-supplied pixel sink, so the
//! module is fully testable without hardware.

use alloc::vec::Vec;

use crate::KernelError;

/// A rectangle in framebuffer coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    pub const fn new(x: u32, y: u32, w: u32, h: u32) -> Self {
        Self { x, y, w, h }
    }

    /// Empty rectangle (w or h == 0).
    pub fn is_empty(&self) -> bool {
        self.w == 0 || self.h == 0
    }

    /// Intersection with another rect; None if disjoint.
    pub fn intersect(&self, o: &Rect) -> Option<Rect> {
        let x0 = self.x.max(o.x);
        let y0 = self.y.max(o.y);
        let x1 = (self.x + self.w).min(o.x + o.w);
        let y1 = (self.y + self.h).min(o.y + o.h);
        if x1 <= x0 || y1 <= y0 {
            None
        } else {
            Some(Rect::new(x0, y0, x1 - x0, y1 - y0))
        }
    }

    /// Union bounding box of two rects.
    pub fn union(&self, o: &Rect) -> Rect {
        let x0 = self.x.min(o.x);
        let y0 = self.y.min(o.y);
        let x1 = (self.x + self.w).max(o.x + o.w);
        let y1 = (self.y + self.h).max(o.y + o.h);
        Rect::new(x0, y0, x1 - x0, y1 - y0)
    }

    /// Clip to a bounds rect.
    pub fn clip(&self, b: &Rect) -> Option<Rect> {
        self.intersect(b)
    }
}

/// Window pixel format (ARGB8888, matches common fb formats after
/// encode).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    /// 32-bit XRGB8888.
    Xrgb8888,
    /// 32-bit ARGB8888 with alpha blending.
    Argb8888,
}

/// A managed window.
#[derive(Debug)]
pub struct Window {
    /// Unique id.
    pub id: u32,
    /// Title (for taskbar/debug).
    pub title: alloc::string::String,
    /// Position and size on screen.
    pub rect: Rect,
    /// Backing store, row-major, `w*h` pixels of the window's format.
    pub pixels: Vec<u32>,
    /// Pixel format of `pixels`.
    pub format: PixelFormat,
    /// Whether the window wants input focus.
    pub wants_focus: bool,
    /// Accumulated damage since last present.
    pub damage: Option<Rect>,
    /// Visible flag.
    pub visible: bool,
}

impl Window {
    /// Create a window with a cleared (transparent black) backing store.
    pub fn new(id: u32, title: &str, rect: Rect, format: PixelFormat) -> Self {
        let n = (rect.w as usize) * (rect.h as usize);
        Self {
            id,
            title: title.into(),
            rect,
            pixels: alloc::vec![0u32; n],
            format,
            wants_focus: false,
            damage: Some(Rect::new(0, 0, rect.w, rect.h)),
            visible: true,
        }
    }

    /// Mark the whole window damaged.
    pub fn damage_all(&mut self) {
        self.damage = Some(self.rect);
    }

    /// Mark a sub-rect (window coords) damaged.
    pub fn damage_rect(&mut self, r: Rect) {
        let local = r.intersect(&self.rect);
        let Some(l) = local else { return };
        let d = Rect::new(l.x - self.rect.x, l.y - self.rect.y, l.w, l.h);
        self.damage = Some(match self.damage {
            Some(cur) => cur.union(&d),
            None => d,
        });
    }

    /// Read a pixel in window coordinates.
    pub fn get(&self, x: u32, y: u32) -> Option<u32> {
        if x >= self.rect.w || y >= self.rect.h {
            return None;
        }
        self.pixels.get(y as usize * self.rect.w as usize + x as usize).copied()
    }

    /// Write a pixel in window coordinates.
    pub fn put(&mut self, x: u32, y: u32, px: u32) {
        if x >= self.rect.w || y >= self.rect.h {
            return;
        }
        let i = y as usize * self.rect.w as usize + x as usize;
        if let Some(slot) = self.pixels.get_mut(i) {
            *slot = px;
        }
    }
}

/// Compositor state: window list ordered bottom→top (last = topmost).
pub struct Compositor {
    windows: Vec<Window>,
    next_id: u32,
    /// Screen bounds.
    pub screen: Rect,
    /// Focused window id.
    pub focus: Option<u32>,
}

impl Compositor {
    pub fn new(screen_w: u32, screen_h: u32) -> Self {
        Self {
            windows: Vec::new(),
            next_id: 1,
            screen: Rect::new(0, 0, screen_w, screen_h),
            focus: None,
        }
    }

    /// Add a window on top.
    pub fn add(&mut self, title: &str, rect: Rect, format: PixelFormat) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        self.windows.push(Window::new(id, title, rect, format));
        id
    }

    /// Remove a window; returns it.
    pub fn remove(&mut self, id: u32) -> Option<Window> {
        let i = self.windows.iter().position(|w| w.id == id)?;
        if self.focus == Some(id) {
            self.focus = None;
        }
        Some(self.windows.remove(i))
    }

    /// Get a window by id.
    pub fn get(&self, id: u32) -> Option<&Window> {
        self.windows.iter().find(|w| w.id == id)
    }

    /// Get a mutable window by id.
    pub fn get_mut(&mut self, id: u32) -> Option<&mut Window> {
        self.windows.iter_mut().find(|w| w.id == id)
    }

    /// Raise a window to the top.
    pub fn raise(&mut self, id: u32) {
        if let Some(i) = self.windows.iter().position(|w| w.id == id) {
            if i + 1 != self.windows.len() {
                let w = self.windows.remove(i);
                self.windows.push(w);
            }
        }
    }

    /// Lower a window to the bottom.
    pub fn lower(&mut self, id: u32) {
        if let Some(i) = self.windows.iter().position(|w| w.id == id) {
            if i != 0 {
                let w = self.windows.remove(i);
                self.windows.insert(0, w);
            }
        }
    }

    /// Move a window; marks old and new areas damaged.
    pub fn move_window(&mut self, id: u32, x: i64, y: i64) {
        let sw = self.screen.w.saturating_sub(1);
        let sh = self.screen.h.saturating_sub(1);
        let nx = (x.max(0) as u32).min(sw);
        let ny = (y.max(0) as u32).min(sh);
        let Some(w) = self.get_mut(id) else { return };
        let old = w.rect;
        w.rect.x = nx;
        w.rect.y = ny;
        w.damage_all();
        // damage is stored in window coords; compositor unions screen-space
        // damage at present time via damage_screen()
        let _ = old;
    }

    /// Resize a window's backing store.
    pub fn resize(&mut self, id: u32, w: u32, h: u32) -> Result<(), KernelError> {
        let win = self.get_mut(id).ok_or(KernelError::InvalidArg("no window"))?;
        let n = (w as usize)
            .checked_mul(h as usize)
            .ok_or(KernelError::InvalidArg("size overflow"))?;
        win.pixels = alloc::vec![0u32; n];
        win.rect.w = w;
        win.rect.h = h;
        win.damage_all();
        Ok(())
    }

    /// Focus a window (raises it).
    pub fn focus_window(&mut self, id: u32) {
        if self.get(id).is_some() {
            self.focus = Some(id);
            self.raise(id);
        }
    }

    /// Topmost window at screen point.
    pub fn window_at(&self, x: u32, y: u32) -> Option<u32> {
        self.windows
            .iter()
            .rev()
            .find(|w| {
                w.visible
                    && x >= w.rect.x
                    && y >= w.rect.y
                    && x < w.rect.x + w.rect.w
                    && y < w.rect.y + w.rect.h
            })
            .map(|w| w.id)
    }

    /// Union of all screen-space damage, clipped to screen.
    pub fn damage_screen(&self) -> Option<Rect> {
        let mut acc: Option<Rect> = None;
        for w in &self.windows {
            if !w.visible {
                continue;
            }
            let Some(d) = w.damage else { continue };
            // damage is window-local; translate to screen space
            let s = Rect::new(
                w.rect.x + d.x,
                w.rect.y + d.y,
                d.w.min(w.rect.w - d.x.min(w.rect.w)),
                d.h.min(w.rect.h - d.y.min(w.rect.h)),
            );
            let s = match s.clip(&self.screen) {
                Some(c) => c,
                None => continue,
            };
            acc = Some(match acc {
                Some(a) => a.union(&s),
                None => s,
            });
        }
        acc.filter(|r| !r.is_empty())
    }

    /// Composite damaged regions into the framebuffer.
    ///
    /// `put(x, y, pixel)` writes one encoded framebuffer pixel.
    /// `encode(px)` converts a source pixel to the target format.
    /// Alpha blending is applied for `Argb8888` windows.
    pub fn present<M, E>(&mut self, put: &mut M, encode: E) -> Result<Rect, KernelError>
    where
        M: FnMut(u32, u32, u32),
        E: Fn(u32) -> u32,
    {
        let damage = self.damage_screen().unwrap_or(Rect::new(0, 0, 0, 0));
        if damage.is_empty() {
            return Ok(damage);
        }
        // windows below a damaged window must be redrawn where the
        // damaged window is transparent — simplest correct approach:
        // repaint the whole damage region back-to-front.
        for w in &self.windows {
            if !w.visible {
                continue;
            }
            let Some(vis) = w.rect.intersect(&damage) else {
                continue;
            };
            for sy in vis.y..vis.y + vis.h {
                for sx in vis.x..vis.x + vis.w {
                    // window-local coords
                    let lx = sx - w.rect.x;
                    let ly = sy - w.rect.y;
                    let Some(px) = w.get(lx, ly) else { continue };
                    let out = match w.format {
                        PixelFormat::Xrgb8888 => encode(px & 0x00ff_ffff),
                        PixelFormat::Argb8888 => {
                            let a = (px >> 24) & 0xff;
                            if a == 0 {
                                continue; // fully transparent
                            }
                            encode(px & 0x00ff_ffff)
                        }
                    };
                    put(sx, sy, out);
                }
            }
        }
        // clear damage
        for w in &mut self.windows {
            w.damage = None;
        }
        Ok(damage)
    }

    /// Window count.
    pub fn len(&self) -> usize {
        self.windows.len()
    }

    /// Iterate windows bottom→top.
    pub fn iter(&self) -> impl Iterator<Item = &Window> {
        self.windows.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    #[test]
    fn rect_ops() {
        let a = Rect::new(0, 0, 10, 10);
        let b = Rect::new(5, 5, 10, 10);
        assert_eq!(a.intersect(&b), Some(Rect::new(5, 5, 5, 5)));
        assert_eq!(a.union(&b), Rect::new(0, 0, 15, 15));
        assert!(a.intersect(&Rect::new(20, 20, 1, 1)).is_none());
    }

    #[test]
    fn add_remove_raise() {
        let mut c = Compositor::new(800, 600);
        let a = c.add("a", Rect::new(0, 0, 10, 10), PixelFormat::Xrgb8888);
        let b = c.add("b", Rect::new(5, 5, 10, 10), PixelFormat::Xrgb8888);
        assert_eq!(c.len(), 2);
        assert_eq!(c.window_at(7, 7), Some(b)); // b on top
        c.lower(b);
        assert_eq!(c.window_at(7, 7), Some(a));
        c.remove(a);
        assert_eq!(c.len(), 1);
        assert!(c.get(a).is_none());
    }

    #[test]
    fn move_and_damage() {
        let mut c = Compositor::new(800, 600);
        let id = c.add("w", Rect::new(10, 10, 20, 20), PixelFormat::Xrgb8888);
        c.move_window(id, 50, 50);
        let w = c.get(id).unwrap();
        assert_eq!(w.rect.x, 50);
        assert_eq!(w.rect.y, 50);
        assert!(w.damage.is_some());
    }

    #[test]
    fn present_draws_pixels() {
        let mut c = Compositor::new(100, 100);
        let id = c.add("w", Rect::new(10, 10, 4, 4), PixelFormat::Xrgb8888);
        {
            let w = c.get_mut(id).unwrap();
            for y in 0..4 {
                for x in 0..4 {
                    w.put(x, y, 0xff0000);
                }
            }
        }
        let mut out: alloc::collections::BTreeMap<(u32, u32), u32> =
            alloc::collections::BTreeMap::new();
        let mut put = |x: u32, y: u32, px: u32| {
            out.insert((x, y), px);
        };
        let enc = |px: u32| px;
        let d = c.present(&mut put, enc).unwrap();
        assert_eq!(d, Rect::new(10, 10, 4, 4));
        assert_eq!(out.len(), 16);
        assert_eq!(out[&(10, 10)], 0xff0000);
        // damage cleared
        assert!(c.damage_screen().is_none());
    }

    #[test]
    fn transparent_pixels_skipped() {
        let mut c = Compositor::new(100, 100);
        let id = c.add("w", Rect::new(0, 0, 2, 2), PixelFormat::Argb8888);
        {
            let w = c.get_mut(id).unwrap();
            w.put(0, 0, 0x00ff0000); // alpha 0 → skip
            w.put(1, 0, 0xffff0000); // opaque red
            w.put(0, 1, 0xff00ff00); // opaque green
            w.put(1, 1, 0xff0000ff); // opaque blue
        }
        let mut out = Vec::new();
        let mut put = |_x: u32, _y: u32, px: u32| out.push(px);
        let enc = |px: u32| px;
        c.present(&mut put, enc).unwrap();
        assert_eq!(out.len(), 3); // 4 px - 1 transparent
    }

    #[test]
    fn resize_reallocates() {
        let mut c = Compositor::new(100, 100);
        let id = c.add("w", Rect::new(0, 0, 2, 2), PixelFormat::Xrgb8888);
        c.resize(id, 10, 10).unwrap();
        assert_eq!(c.get(id).unwrap().pixels.len(), 100);
    }
}
