//! Graphical rendering of an [Item]
use crate::data::{Module,ModuleContext,ItemReference,IterationItem,Value};
use crate::event::EventSink;
use crate::font::{render_font,render_font_item};
use crate::icon;
use crate::render::{Render,Align,Width};
use crate::state::Runtime;
#[cfg(feature="dbus")]
use crate::tray;
use log::{debug,warn,error};
use std::borrow::Cow;
use std::rc::Rc;
use tiny_skia::{Color,Point,Transform};

/// A visible item in a bar
#[derive(Debug)]
pub struct Item {
    pub format : ItemFormat,
    pub data : Module,
    events : EventSink,
}

/// Formatting information for a visible bar item
#[derive(Debug,Default)]
pub struct ItemFormat {
    markup : bool,
    oneline: bool,
    cfg : Option<toml::Value>,
}

impl ItemFormat {
    pub fn from_toml(config : &toml::Value) -> Self {
        let mut rv = Self::default();
        rv.markup = config.get("markup").and_then(|v| v.as_bool()).unwrap_or(false);
        rv.oneline = config.get("oneline").and_then(|v| v.as_bool()).unwrap_or(false);

        rv.cfg = config.as_table()
            .map(|t| t.iter()
                .filter(|(k,_)| match &***k {
                    "align" |
                    "bg" |
                    "bg-alpha" |
                    "border" |
                    "border-alpha" |
                    "border-color" |
                    "fg" |
                    "fg-alpha" |
                    "font" |
                    "halign" |
                    "margin" |
                    "max-width" |
                    "min-width" |
                    "padding" |
                    "text-outline" |
                    "text-outline-alpha" |
                    "text-outline-width" |
                    "valign" => true,
                    _ => false,
                })
                .map(|(k,v)| (k.clone(), v.clone()))
                .collect::<toml::map::Map<_,_>>())
            .filter(|m| !m.is_empty())
            .map(toml::Value::Table);

        rv
    }

    pub fn is_trivial(&self) -> bool {
        self.cfg.is_none()
    }

    pub fn setup_ctx<'a, 'p : 'a, 'c>(&self, ctx : &'a mut Render<'p, 'c>) -> (Formatting, Render<'a, 'c>) {
        let z = toml::Value::Integer(0);
        let config = self.cfg.as_ref().unwrap_or(&z);
        let fmt = Formatting::expand(config, ctx.runtime);
        let runtime = &ctx.runtime;
        let get = |key| {
            config.get(key).and_then(|v| match v.as_str() {
                Some(fmt) => runtime.format(&fmt).or_else(|e| {
                    warn!("Error expanding '{}' when rendering: {}", fmt, e);
                    Err(())
                }).ok().map(Value::into_text),
                None => Some(v.to_string().into())
            })
        };

        let get_f32 = |key| {
            config.get(key).and_then(|v| match v.as_str() {
                Some(fmt) => runtime.format(&fmt).or_else(|e| {
                    warn!("Error expanding '{}' when rendering: {}", fmt, e);
                    Err(())
                }).ok().and_then(|v| v.parse_f32()),
                None => v.as_float().map(|v| v as f32).or_else(|| v.as_integer().map(|i| i as f32)),
            })
        };

        let mut align = Align {
            horiz : get("halign").and_then(Align::parse_hv),
            vert : get("valign").and_then(Align::parse_hv),
        };
        align.from_name(get("align"));

        let (font, font_size) = get("font").map_or((None, None), |font| {
            let mut size = None::<f32>;
            let font = match font.rsplit_once(' ') {
                Some((name, ssize)) if {
                    size = ssize.parse().ok();
                    size.is_some()
                } => name,
                _ => &*font,
            };
            let font = runtime.fonts.iter().find(|f| f.name == font);
            (font, size)
        });

        let fg_rgba = Formatting::parse_rgba(get("fg"), get_f32("fg-alpha"));
        let stroke_rgba = Formatting::parse_rgba(get("text-outline"), get_f32("text-outline-alpha"));
        let stroke_size = get_f32("text-outline-width");

        let render = Render {
            canvas : &mut *ctx.canvas,
            align : ctx.align.merge(&align),
            font : font.unwrap_or(&ctx.font),
            font_size : font_size.unwrap_or(ctx.font_size),
            font_color : fg_rgba.unwrap_or(ctx.font_color),
            text_stroke : stroke_rgba.or(ctx.text_stroke),
            text_stroke_size : stroke_size.or(ctx.text_stroke_size),
            ..*ctx
        };
        (fmt, render)
    }
}

/// Formatting that must be applied after rendering an item
#[derive(Debug,Clone,Default,PartialEq)]
pub struct Formatting {
    bg_rgba : Option<Color>,
    border : Option<(f32, f32, f32, f32)>,
    border_rgba : Option<Color>,
    min_width : Option<Width>,
    max_width : Option<Width>,
    margin : Option<(f32, f32, f32, f32)>,
    padding : Option<(f32, f32, f32, f32)>,
}

impl Formatting {
    fn expand(config : &toml::Value, runtime : &Runtime) -> Self {
        let get = |key| {
            config.get(key).and_then(|v| match v.as_str() {
                Some(fmt) => runtime.format(&fmt).or_else(|e| {
                    warn!("Error expanding '{}' when rendering: {}", fmt, e);
                    Err(())
                }).ok().map(Value::into_text),
                None => Some(v.to_string().into())
            })
        };

        let get_f32 = |key| {
            config.get(key).and_then(|v| match v.as_str() {
                Some(fmt) => runtime.format(&fmt).or_else(|e| {
                    warn!("Error expanding '{}' when rendering: {}", fmt, e);
                    Err(())
                }).ok().and_then(|v| v.parse_f32()),
                None => v.as_float().map(|v| v as f32).or_else(|| v.as_integer().map(|i| i as f32)),
            })
        };
        let min_width = get("min-width").and_then(Width::from_str);
        let max_width = get("max-width").and_then(Width::from_str);

        let margin = get("margin").and_then(Formatting::parse_trbl);
        let border = get("border").and_then(Formatting::parse_trbl);
        let padding = get("padding").and_then(Formatting::parse_trbl);

        let bg_rgba = Formatting::parse_rgba(get("bg"), get_f32("bg-alpha"));
        let border_rgba = Formatting::parse_rgba(get("border-color"), get_f32("border-alpha"));

        Self {
            bg_rgba,
            border,
            border_rgba,
            min_width,
            max_width,
            margin,
            padding,
        }
    }

    fn parse_trbl(v : Cow<str>) -> Option<(f32, f32, f32, f32)> {
        let mut rv = (0.0, 0.0, 0.0, 0.0);
        for (i,x) in v.split_whitespace().enumerate() {
            match (i, x.parse()) {
                (0, Ok(v)) => {
                    rv = (v, v, v, v);
                }
                (1, Ok(v)) => {
                    rv.1 = v; rv.3 = v;
                }
                (2, Ok(v)) => {
                    rv.2 = v;
                }
                (3, Ok(v)) => {
                    rv.3 = v;
                }
                _ => return None,
            }
        }
        Some(rv)
    }

    pub fn parse_rgba(color : Option<impl AsRef<str>>, alpha : Option<f32>) -> Option<Color> {
        if color.is_none() && alpha.is_none() {
            return None;
        }
        let color = color.as_ref().map_or("black", |v| v.as_ref());
        let (r,g,b,mut a);
        let alpha_f = alpha.unwrap_or(1.0) * 65535.0;
        a = f32::min(65535.0, f32::max(0.0, alpha_f)) as u64;
        if color.starts_with('#') {
            let v = u64::from_str_radix(&color[1..], 16);
            match (v, color.len()) {
                (Ok(v), 4) => {
                    r = ((v >> 8) & 0xF) * 0x1111;
                    g = ((v >> 4) & 0xF) * 0x1111;
                    b = ((v >> 0) & 0xF) * 0x1111;
                }
                (Ok(v), 5) => {
                    r = ((v >> 12) & 0xF) * 0x1111;
                    g = ((v >> 8) & 0xF) * 0x1111;
                    b = ((v >> 4) & 0xF) * 0x1111;
                    a = ((v >> 0) & 0xF) * 0x1111;
                }
                (Ok(v), 7) => {
                    r = ((v >> 16) & 0xFF) * 0x101;
                    g = ((v >> 8) & 0xFF) * 0x101;
                    b = ((v >> 0) & 0xFF) * 0x101;
                }
                (Ok(v), 9) => {
                    r = ((v >> 24) & 0xFF) * 0x101;
                    g = ((v >> 16) & 0xFF) * 0x101;
                    b = ((v >> 8) & 0xFF) * 0x101;
                    a = ((v >> 0) & 0xFF) * 0x101;
                }
                (Ok(v), 13) => {
                    r = (v >> 32) & 0xFFFF;
                    g = (v >> 16) & 0xFFFF;
                    b = (v >> 0) & 0xFFFF;
                }
                (Ok(v), 17) => {
                    r = (v >> 48) & 0xFFFF;
                    g = (v >> 32) & 0xFFFF;
                    b = (v >> 16) & 0xFFFF;
                    a = (v >> 0) & 0xFFFF;
                }
                _ => {
                    debug!("Could not parse color '{}'", color);
                    r = 0; g = 0; b = 0;
                }
            }
        } else {
            match color {
                "black" => { r = 0; g = 0; b = 0; }
                "red" => { r = 0xFFFF; g = 0; b = 0; }
                "yellow" => { r = 0xFFFF; g = 0xFFFF; b = 0; }
                "green" => { r = 0; g = 0xFFFF; b = 0; }
                "blue" => { r = 0; g = 0; b = 0xFFFF; }
                "gray" => { r = 0x7FFF; g = 0x7FFF; b = 0x7FFF; }
                "white" => { r = 0xFFFF; g = 0xFFFF; b = 0xFFFF; }
                _ => {
                    debug!("Unknown color '{}'", color);
                    r = 0; g = 0; b = 0;
                }
            }
        }
        Color::from_rgba(
            r as f32 / 65535.0,
            g as f32 / 65535.0,
            b as f32 / 65535.0,
            a as f32 / 65535.0,
        )
    }

    fn get_shrink(&self) -> Option<(f32, f32, f32, f32)> {
        let mut rv = (0.0, 0.0, 0.0, 0.0);
        if self.padding == None && self.margin == None {
            return None;
        }
        for &i in &[self.padding, self.margin, self.border] {
            if let Some((t,r,b,l)) = i {
                rv.0 += t;
                rv.1 += r;
                rv.2 += b;
                rv.3 += l;
            }
        }
        Some(rv)
    }

    fn is_boring(&self) -> bool {
        *self == Self::default()
    }
}

impl From<Module> for Item {
    fn from(data : Module) -> Self {
        Self {
            format : ItemFormat::default(),
            events : EventSink::default(),
            data
        }
    }
}

impl Item {
    pub fn none() -> Self {
        Self {
            format : ItemFormat::default(),
            events : EventSink::default(),
            data : Module::parse_error(""),
        }
    }

    pub fn new_bar(cfg : toml::Value) -> Self {
        let left = Rc::new(cfg.get("left").map_or_else(Item::none, Item::from_toml_ref));
        let right = Rc::new(cfg.get("right").map_or_else(Item::none, Item::from_toml_ref));
        let center = Rc::new(cfg.get("center").map_or_else(Item::none, Item::from_toml_ref));

        Item {
            events : EventSink::from_toml(&cfg),
            format : ItemFormat::from_toml(&cfg),
            data : Module::Bar {
                left, center, right,
                config : cfg,
            },
        }
    }

    pub fn from_toml_ref(value : &toml::Value) -> Self {
        if value.as_str().is_some() {
            return Module::from_toml_in(value, ModuleContext::Source).into();
        }

        Self::from_item_list("<ref>", value)
    }

    pub fn from_toml_format(value : &toml::Value) -> Self {
        Self::from_item_list("<ref>", value)
    }

    pub fn from_item_list(key : &str, value : &toml::Value) -> Self {
        if let Some(array) = value.as_array() {
            return Module::Group {
                items : array.iter().map(Item::from_toml_ref).map(Rc::new).collect(),
                condition : None,
                tooltip : None,
                spacing : "".into(),
                vertical: false,
            }.into();
        }

        let data = Module::from_toml_in(value, ModuleContext::Item);
        if let Module::ParseError { msg } = &data {
            error!("Error parsing {key}: {msg}");
        }
        Item {
            events : EventSink::from_toml(value),
            format : ItemFormat::from_toml(value),
            data,
        }
    }

    pub fn render(self : &Rc<Self>, parent_ctx : &mut Render) -> EventSink {
        // skip rendering if we are outside the clip bounds
        if !parent_ctx.render_flex && parent_ctx.render_pos.x > parent_ctx.render_extents.1.x {
            return EventSink::default();
        }

        let mut rv = self.events.clone();

        if self.format.is_trivial() {
            self.render_inner(parent_ctx, &mut rv);
            return rv;
        }

        let (format, mut ctx) = self.format.setup_ctx(parent_ctx);
        if format.is_boring() {
            self.render_inner(&mut ctx, &mut rv);
            let pos = ctx.render_pos;
            parent_ctx.render_pos = pos;
            return rv;
        }

        let outer_clip = ctx.render_extents;
        let mut start_pos = ctx.render_pos;
        let mut inner_clip = outer_clip;

        let shrink = format.get_shrink();
        if (shrink, format.max_width) != (None, None) {
            match shrink {
                Some((t, r, b, l)) => {
                    inner_clip.0.x += l;
                    inner_clip.0.y += t;
                    start_pos.x += l;
                    start_pos.y += t;
                    inner_clip.1.x -= r;
                    inner_clip.1.y -= b;
                }
                None => {}
            }
            match format.max_width {
                Some(Width::Pixels(n)) => {
                    let clip_at = start_pos.x + n;
                    if inner_clip.1.x > clip_at {
                        ctx.render_flex = false;
                        inner_clip.1.x = clip_at;
                    }
                }
                Some(Width::Fraction(f)) => {
                    let parent_width = outer_clip.1.x - outer_clip.0.x;
                    inner_clip.1.x = inner_clip.1.x.min(start_pos.x + parent_width * f);
                }
                None => {}
            }
        }

        ctx.render_pos = start_pos;
        ctx.render_extents = inner_clip;
        self.render_inner(&mut ctx, &mut rv);

        let mut end_pos = ctx.render_pos;

        let child_render_width = end_pos.x - start_pos.x;
        let mut min_width = match format.min_width {
            None => 0.0,
            Some(Width::Pixels(n)) => n,
            Some(Width::Fraction(f)) => f * (outer_clip.1.x - outer_clip.0.x),
        };
        if min_width > inner_clip.1.x - start_pos.x {
            // clamp the minimum to only the available region
            min_width = inner_clip.1.x - start_pos.x;
        }

        let inner_x_offset;
        if child_render_width < min_width {
            // child is smaller than the box; align it
            let expand = min_width - child_render_width;
            match ctx.align.horiz {
                Some(f) => {
                    inner_x_offset = expand * f;
                    let x0 = start_pos.x.floor() as usize;
                    let x1 = end_pos.x.ceil() as usize;
                    let ilen = x1 - x0;
                    let wlen = min_width.ceil() as usize;
                    if wlen > ilen {
                        // Align by rotating the pixels of the inner clip region to the right.  The
                        // right part of the clip region should just be blank pixels at this point,
                        // which is what we want to put on the left.
                        let rlen = (wlen - ilen) * 4;
                        let stride = ctx.canvas.width() as usize * 4;
                        let h = ctx.canvas.height() as usize;
                        for y in 0..h {
                            let x0 = y * stride + x0 * 4;
                            let x2 = y * stride + (x0 + wlen) * 4;
                            if let Some(buf) = ctx.canvas.data_mut().get_mut(x0..x2) {
                                buf.rotate_right(rlen);
                            }
                        }
                    }
                }
                _ => { // defaults to left align
                    inner_x_offset = 0.0;
                }
            }
        } else {
            inner_x_offset = 0.0;
        }

        let shrink_r_width = shrink.map_or(0.0, |s| s.1);
        let shrink_b_height = shrink.map_or(0.0, |s| s.2);
        if !ctx.render_flex {
            // clip to the allowed size
            end_pos.x = end_pos.x.min(inner_clip.1.x);
        }
        let outer_pos = end_pos + Point { x: shrink_r_width, y: shrink_b_height };

        rv.offset_clamp(inner_x_offset, start_pos.x, end_pos.x);

        if format.bg_rgba.is_some() || format.border.is_some() {
            use tiny_skia::Rect;
            let mut bg_clip = (start_pos, end_pos);
            if let Some((t, r, b, l)) = format.padding {
                bg_clip.0.x -= l;
                bg_clip.0.y -= t;
                bg_clip.1.x += r;
                bg_clip.1.y += b;
            }

            if let Some(rgba) = format.bg_rgba {
                if let Some(rect) = Rect::from_ltrb(bg_clip.0.x, bg_clip.0.y, bg_clip.1.x, bg_clip.1.y) {
                    let paint = tiny_skia::Paint {
                        shader: tiny_skia::Shader::SolidColor(rgba),
                        anti_alias: true,
                        // background is painted "underneath"
                        blend_mode : tiny_skia::BlendMode::DestinationOver,
                        ..tiny_skia::Paint::default()
                    };
                    ctx.canvas.fill_rect(rect, &paint, ctx.render_xform, None);
                }
            }

            if let Some((t, r, b, l)) = format.border {
                let rgba = format.border_rgba.unwrap_or(ctx.font_color);
                let paint = tiny_skia::Paint {
                    shader: tiny_skia::Shader::SolidColor(rgba),
                    anti_alias: true,
                    ..tiny_skia::Paint::default()
                };

                bg_clip.0.y -= t;
                if let Some(rect) = Rect::from_xywh(bg_clip.0.x, bg_clip.0.y, bg_clip.1.x - bg_clip.0.x, t) {
                    // top edge, no corners
                    ctx.canvas.fill_rect(rect, &paint, ctx.render_xform, None);
                }

                bg_clip.0.x -= l;
                if let Some(rect) = Rect::from_xywh(bg_clip.0.x, bg_clip.0.y, l, bg_clip.1.y - bg_clip.0.y) {
                    // left edge + top-left corner
                    ctx.canvas.fill_rect(rect, &paint, ctx.render_xform, None);
                }

                if let Some(rect) = Rect::from_xywh(bg_clip.1.x, bg_clip.0.y, r, bg_clip.1.y - bg_clip.0.y) {
                    // right edge + top-right corner
                    ctx.canvas.fill_rect(rect, &paint, ctx.render_xform, None);
                }

                bg_clip.1.x += r;
                if let Some(rect) = Rect::from_xywh(bg_clip.0.x, bg_clip.1.y, bg_clip.1.x - bg_clip.0.x, b) {
                    // bottom edge + both corners
                    ctx.canvas.fill_rect(rect, &paint, ctx.render_xform, None);
                }
            }
        }

        parent_ctx.render_pos = outer_pos;

        rv
    }

    pub fn render_clamped(self : &Rc<Self>, ctx : &mut Render, ev : &mut EventSink) {
        let x0 = ctx.render_pos.x;
        let mut rv = self.render(ctx);
        let x1 = ctx.render_pos.x;
        rv.offset_clamp(0.0, x0, x1);
        ev.merge(rv);
    }

    pub fn render_clamped_item(self : &Rc<Self>, ctx : &mut Render, ev : &mut EventSink, item : &IterationItem) {
        let item_var = ctx.runtime.get_item_var();
        let prev = item_var.replace(Some(item.clone()));
        let origin = ctx.render_pos;
        let mut rv = self.render(ctx);
        let x1 = ctx.render_pos.x;
        ctx.render_pos.y = origin.y;
        rv.offset_clamp(0.0, origin.x, x1);
        rv.set_item(item);
        ev.merge(rv);
        item_var.set(prev);
    }

    /// Render the block contents to the given context.
    ///
    /// Your item starts at the context's current point.  When you are done rendering, you should
    /// adjust the point to be offset by the size of your rendered item.
    ///
    /// You may use the current clip area to determine sizes.  By default, the clip area is set to
    /// the size of the entire bar; however, any max_width specifiers in a parent item will reduce
    /// this.
    ///
    /// Note that the coordinates you use to render may not match the final coordinates in the
    /// buffer; if your item is not left-aligned, it will likely be shifted right before the final
    /// render.
    fn render_inner(self : &Rc<Self>, ctx : &mut Render, rv : &mut EventSink) {
        match &self.data {
            Module::ItemReference { value } => {
                ItemReference::with(value, &ctx.runtime, |item| match item {
                    Some(item) => rv.merge(item.render(ctx)),
                    None => {}
                });
            }
            Module::Group { condition, items, tooltip, spacing, vertical } => {
                if let Some(cond) = condition {
                    if !cond.is_empty() {
                        match ctx.runtime.format(cond) {
                            Ok(v) if v.as_bool() => {},
                            Ok(_) => return,
                            Err(e) => {
                                warn!("Error evaluating condition '{}': {}", cond, e);
                            }
                        }
                    }
                }
                let origin = ctx.render_pos;
                let mut bounds = origin;
                let spacing = ctx.runtime.format(spacing).ok().and_then(|s| s.parse_f32()).unwrap_or(0.0);
                for item in items {
                    item.render_clamped(ctx, rv);

                    if *vertical {
                        if ctx.render_pos.x > bounds.x {
                            bounds.x = ctx.render_pos.x;
                        }
                        ctx.render_pos.x = origin.x;
                        bounds.y = ctx.render_pos.y;
                        if spacing > 0.0 {
                            ctx.render_pos.y = (ctx.render_pos.y + spacing).ceil();
                        }
                    } else {
                        bounds.x = ctx.render_pos.x;
                        if ctx.render_pos.y > bounds.y {
                            bounds.y = ctx.render_pos.y;
                        }
                        ctx.render_pos.y = origin.y;
                        if spacing > 0.0 {
                            ctx.render_pos.x = (ctx.render_pos.x + spacing).ceil();
                        }
                    }
                }
                ctx.render_pos = bounds;
                if let Some(item) = tooltip {
                    rv.add_tooltip(PopupDesc::RenderItem {
                        item : item.clone(),
                        iter : ctx.runtime.copy_item_var(),
                    });
                }
            }
            Module::FocusList { source, others, focused, spacing } => {
                let spacing = ctx.runtime.format(spacing).ok().and_then(|s| s.parse_f32()).unwrap_or(0.0);
                let item_var = ctx.runtime.get_item_var();
                let origin = ctx.render_pos;
                let prev = item_var.replace(None);
                source.read_focus_list(ctx.runtime, |focus, item| {
                    item_var.set(Some(item.clone()));
                    let x0 = ctx.render_pos.x;
                    let mut ev = if focus {
                        focused.render(ctx)
                    } else {
                        others.render(ctx)
                    };
                    let x1 = ctx.render_pos.x;
                    ev.offset_clamp(0.0, x0, x1);
                    ev.set_item(&item);
                    rv.merge(ev);
                    ctx.render_pos.x += spacing;
                    ctx.render_pos.y = origin.y;
                });
                let xpos = ctx.render_pos.x - spacing;
                ctx.render_pos.x = ctx.render_pos.x.min(xpos);
                item_var.set(prev);
            }
            Module::Bar { left, center, right, .. } => {
                let clip = ctx.render_extents;
                let xform = ctx.render_xform;
                let width = clip.1.x - ctx.render_pos.x;
                let height = (clip.1.y - ctx.render_pos.y).ceil();
                let mut canvas_size = tiny_skia::Point { x: width, y: height };
                let render_extents = (Point::zero(), canvas_size);
                xform.map_points(std::slice::from_mut(&mut canvas_size));
                let mut canvas = tiny_skia::Pixmap::new(canvas_size.x as u32, canvas_size.y as u32)
                    .unwrap_or_else(|| tiny_skia::Pixmap::new(1,1).unwrap());
                let mut canvas = canvas.as_mut();

                let mut left_ev = left.render(ctx);
                let left_size = ctx.render_pos.x.ceil();
                left_ev.offset_clamp(0.0, 0.0, left_size);
                rv.merge(left_ev);

                let mut group = Render {
                    canvas : &mut canvas,
                    cache : &ctx.cache,
                    render_extents,
                    render_xform: ctx.render_xform,
                    render_pos: Point::zero(),
                    render_flex : ctx.render_flex,

                    font : ctx.font,
                    font_size : ctx.font_size,
                    font_color : ctx.font_color,
                    text_stroke : ctx.text_stroke,
                    text_stroke_size : ctx.text_stroke_size,

                    align : ctx.align,
                    err_name : "bar",
                    runtime : ctx.runtime,
                };

                let mut right_ev = right.render(&mut group);
                let right_width = group.render_pos.x.ceil();

                let right_offset = clip.1.x - right_width;
                ctx.canvas.draw_pixmap(
                    0, 0,
                    group.canvas.as_ref(),
                    &tiny_skia::PixmapPaint::default(),
                    Transform::from_translate(right_offset * ctx.render_xform.sx, 0.0),
                    None);

                right_ev.offset_clamp(right_offset, right_offset, clip.1.x);
                rv.merge(right_ev);

                group.canvas.fill(tiny_skia::Color::TRANSPARENT);
                group.render_pos = Point::zero();

                let mut cent_ev = center.render(&mut group);
                let cent_size = group.render_pos.x.ceil();

                let max_side = (width - cent_size) / 2.0;
                let total_room = width - (left_size + right_width + cent_size);
                let cent_offset;
                if total_room < 0.0 {
                    // TODO maybe we should have cropped it?
                    return;
                } else if left_size > max_side {
                    // left side is too long to properly center; put it just to the right of that
                    cent_offset = left_size;
                } else if right_width > max_side {
                    // right side is too long to properly center; put it just to the left of that
                    cent_offset = clip.1.x - right_width - cent_size;
                } else {
                    // Actually center the center module
                    cent_offset = max_side;
                }
                ctx.canvas.draw_pixmap(
                    0, 0,
                    group.canvas.as_ref(),
                    &tiny_skia::PixmapPaint::default(),
                    Transform::from_translate(cent_offset * ctx.render_xform.sx, 0.0),
                    None);
                cent_ev.offset_clamp(cent_offset, cent_offset, cent_offset + cent_size);
                rv.merge(cent_ev);

                ctx.render_pos.x = clip.1.x;
            }
            Module::Icon { name, fallback, tooltip } => {
                let markup = self.format.markup;
                let name = ctx.runtime.format_or(name, ctx.err_name).into_text();
                match icon::render(ctx, &name) {
                    Ok(()) => {},
                    Err(()) => {
                        let value = ctx.runtime.format_or(fallback, ctx.err_name).into_owned();
                        let mut item : Item = Module::new_value(value).into();
                        item.format.markup = markup;
                        Rc::new(item).render(ctx);
                    }
                }
                if !tooltip.is_empty() {
                    rv.add_tooltip(PopupDesc::TextItem {
                        source : self.clone(),
                        iter : ctx.runtime.copy_item_var(),
                    });
                }
            },
            Module::SwayTree(tree) => {
                tree.render(ctx, rv);
            }
            #[cfg(feature="dbus")]
            Module::Tray { passive, active, urgent } => {
                tray::show(ctx, rv, [passive, active, urgent])
            }

            // All other modules are rendered as text
            _ => {
                let markup = self.format.markup;
                let oneline = self.format.oneline;
                let mut text = self.data.read_to_owned(ctx.err_name, "text", &ctx.runtime).into_text();
                if oneline && text.contains('\n') {
                    text = text.replace('\n', " ").into();
                }

                render_font_item(ctx, &text, markup);

                match &self.data {
                    Module::Formatted { tooltip : Some(item), .. } => {
                        rv.add_tooltip(PopupDesc::RenderItem {
                            item : item.clone(),
                            iter : ctx.runtime.copy_item_var(),
                        });
                    }
                    Module::Formatted { tooltip : None, .. } => {}
                    _ => {
                        let tt = self.data.read_to_owned(ctx.err_name, "tooltip", &ctx.runtime).into_text();
                        if !tt.is_empty() {
                            rv.add_tooltip(PopupDesc::TextItem {
                                source : self.clone(),
                                iter : ctx.runtime.copy_item_var(),
                            });
                        }
                    }
                }
            }
        }
    }
}

#[derive(Debug,Clone)]
pub enum PopupDesc {
    RenderItem {
        item : Rc<Item>,
        iter : Option<IterationItem>,
    },
    TextItem {
        source : Rc<Item>,
        iter : Option<IterationItem>,
    },
    #[cfg(feature="dbus")]
    Tray(tray::TrayPopup),
}

impl PartialEq for PopupDesc {
    fn eq(&self, rhs : &Self) -> bool {
        match (self, rhs) {
            (PopupDesc::RenderItem { item : a, iter : ai }, PopupDesc::RenderItem { item : b, iter : bi }) => {
                Rc::ptr_eq(a,b) && ai == bi
            }
            (PopupDesc::TextItem { source : a, iter : ai }, PopupDesc::TextItem { source : b, iter : bi }) => {
                Rc::ptr_eq(a,b) && ai == bi
            }
            #[cfg(feature="dbus")]
            (PopupDesc::Tray(a), PopupDesc::Tray(b)) => a == b,
            _ => false,
        }
    }
}

impl PopupDesc {
    pub fn render_popup(&mut self, runtime : &Runtime, target : &mut tiny_skia::PixmapMut, scale: i32) -> (i32, i32) {
        target.fill(tiny_skia::Color::BLACK);
        let font = &runtime.fonts[0];
        let render_extents = (Point::zero(), Point { x: target.width() as f32, y: target.height() as f32 });

        let mut ctx = Render {
            canvas : target,
            cache : &runtime.cache,
            font,
            font_size : 16.0,
            font_color : Color::WHITE,
            align : Align::bar_default(),
            render_extents,
            render_xform: Transform::from_scale(scale as f32, scale as f32),
            render_pos : tiny_skia::Point { x: 2.0, y: 2.0 },
            render_flex : true,
            err_name: "popup",
            text_stroke : None,
            text_stroke_size : None,
            runtime,
        };

        self.render(&mut ctx);
        (ctx.render_pos.x as i32, ctx.render_pos.y as i32)
    }

    fn render(&mut self, ctx : &mut Render) {
        match self {
            PopupDesc::RenderItem { item, iter } => {
                let item_var = ctx.runtime.get_item_var();
                item_var.set(iter.clone());
                item.render(ctx);
                item_var.set(None);
            }
            PopupDesc::TextItem { source, iter } => {
                let item_var = ctx.runtime.get_item_var();
                item_var.set(iter.clone());
                let value = source.data.read_to_owned("tooltip", "tooltip", ctx.runtime).into_text();
                item_var.set(None);

                if value.is_empty() {
                    return;
                }

                let markup = source.format.markup;

                let (width, height) = render_font(ctx, (2.0, 2.0), &value, markup);
                ctx.render_pos.x = width + 4.0;
                ctx.render_pos.y = height + 4.0;
            }
            #[cfg(feature="dbus")]
            PopupDesc::Tray(tray) => tray.render(ctx),
        }
    }

    pub fn button(&mut self, x : f64, y : f64, button : u32, runtime : &mut Runtime) {
        match self {
            PopupDesc::RenderItem { item, iter } => {
                if let Some(ii) = iter.as_ref() {
                    let mut events = item.events.clone();
                    events.set_item(ii);
                    events.button(x as f32, y as f32, button, runtime);
                } else {
                    item.events.button(x as f32, y as f32, button, runtime);
                }
            }
            PopupDesc::TextItem { .. } => { }
            #[cfg(feature="dbus")]
            PopupDesc::Tray(tray) => tray.button(x, y, button, runtime),
        }
    }
}
