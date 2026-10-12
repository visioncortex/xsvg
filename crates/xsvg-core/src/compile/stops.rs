//! Caret stops on outlined text (docs/Incremental.md §6; consumer: xsvg-kit).
//!
//! Outlined text — an `<x:textbox outline="true">`, warped or not, or an
//! `<x:textpath>` with a geometric effect — compiles to path data with no glyph
//! mapping, so an editor has nothing to place a caret on. Rather than rebuilding
//! the compile's placement and fields a second time, the compile itself records
//! the stops while it runs: the outline branch notes each traced line, and every
//! field that bakes the paths ([`super::pathops::emit_warp`], the text-on-path
//! effects) maps the recorded stops through the same `Field` it maps the `d`s
//! through. The recorder is `None` on an ordinary compile — nothing is recorded
//! and the output is untouched — and lives only for a [`super::CompileSession::caret_stops`]
//! query, which compiles the element's top-level fragment into a scratch buffer.

use super::*;
use crate::kurbo::Point;
use crate::warp::{path_effect_field, Field};

/// One caret position between two outlined characters, in the user space of the
/// `d` the compile emitted for its run — the same space the host maps with the
/// keyed element's CTM.
#[derive(Clone, Debug, PartialEq)]
pub struct CaretStop {
    /// The caret's head: `(x, baseline − ascent)` mapped through the run's fields.
    pub top: (f64, f64),
    /// Its foot: `(x, baseline + descent)` mapped the same way.
    pub bottom: (f64, f64),
}

/// The stops of one outlined run: a textbox line, or a textpath's whole text.
#[derive(Clone, Debug, PartialEq)]
pub struct StopRun {
    /// Exactly the characters the run outlined.
    pub text: String,
    /// `text.chars().count() + 1` entries: stop `i` sits before scalar `i`, the
    /// last one after the run. Stop `0` is the pen origin, not the first glyph's
    /// ink edge; consecutive stops differ by the outline font's advances.
    pub stops: Vec<CaretStop>,
}

impl StopRun {
    /// Map every stop through `field` — the compile calls this with the field it
    /// baked the run's path through, so stops and glyphs stay in step.
    pub(super) fn map(&mut self, field: &dyn Field) {
        let f = |(x, y): (f64, f64)| {
            let p = field.map(Point::new(x, y));
            (p.x, p.y)
        };
        for s in &mut self.stops {
            s.top = f(s.top);
            s.bottom = f(s.bottom);
        }
    }
}

/// The caret stops of one outlined text element, runs in output order.
#[derive(Clone, Debug, PartialEq)]
pub struct CaretStops {
    pub runs: Vec<StopRun>,
}

/// What a `caret_stops` query collects while its scratch compile runs: each
/// outlined run keyed by the source span of the element that owns it.
#[derive(Default)]
pub(super) struct StopRecorder {
    pub(super) runs: Vec<((usize, usize), StopRun)>,
}

/// Recording paused for a scope; dropping it resumes. Scratch serializations
/// that harvest geometry (a referenced `x:` target, a boolean operand, a plugin's
/// children, a linked file) take one: their output is not the element's own
/// rendering, or its spans belong to another document.
pub(super) struct StopsSuspended<'c, 'a> {
    ctx: &'c Ctx<'a>,
    prev: Option<StopRecorder>,
}

impl Drop for StopsSuspended<'_, '_> {
    fn drop(&mut self) {
        *self.ctx.stops.borrow_mut() = self.prev.take();
    }
}

impl<'a> Ctx<'a> {
    /// Whether a `caret_stops` query is listening — the outline branches only
    /// walk prefix advances when it is.
    pub(super) fn recording_stops(&self) -> bool {
        self.stops.borrow().is_some()
    }

    pub(super) fn record_stops(&self, span: (usize, usize), run: StopRun) {
        if let Some(r) = self.stops.borrow_mut().as_mut() {
            r.runs.push((span, run));
        }
    }

    /// How many runs are recorded so far — a field's `emit` takes one before
    /// lowering its children and maps everything recorded after it.
    pub(super) fn stops_mark(&self) -> usize {
        self.stops.borrow().as_ref().map_or(0, |r| r.runs.len())
    }

    /// Map every run recorded since `mark` through `field`.
    pub(super) fn map_stops_from(&self, mark: usize, field: &dyn Field) {
        if let Some(r) = self.stops.borrow_mut().as_mut() {
            for (_, run) in r.runs.iter_mut().skip(mark) {
                run.map(field);
            }
        }
    }

    pub(super) fn suspend_stops(&self) -> StopsSuspended<'_, 'a> {
        StopsSuspended {
            ctx: self,
            prev: self.stops.borrow_mut().take(),
        }
    }
}

/// An element's source byte span, the recorder's key (the same pair
/// [`TextLayoutPara::span`] reports).
pub(super) fn span_of(node: roxmltree::Node) -> (usize, usize) {
    let r = node.range();
    (r.start, r.end)
}

/// The stops of one flat run outlined at `(start_x, baseline)`: the pen origin,
/// then the outline font's advance after each scalar — the same face that drew
/// the glyphs, so stops sit between the outlined letters. `None` when the
/// outliner cannot measure (no font bytes): then nothing is recorded.
pub(super) fn stop_run(
    text: &str,
    style: &TextStyle,
    size: f64,
    start_x: f64,
    baseline: f64,
    metrics: &FontMetrics,
    outliner: &dyn GlyphOutliner,
) -> Option<StopRun> {
    let mut stops = Vec::with_capacity(text.chars().count() + 1);
    let mut push = |x: f64| {
        stops.push(CaretStop {
            top: (x, baseline - metrics.ascent),
            bottom: (x, baseline + metrics.descent),
        })
    };
    push(start_x);
    for (i, c) in text.char_indices() {
        let adv = outliner.advance_width(&text[..i + c.len_utf8()], style, size)?;
        if !adv.is_finite() {
            return None;
        }
        push(start_x + adv);
    }
    Some(StopRun {
        text: text.to_string(),
        stops,
    })
}

/// Record a text-on-path run's stops: the flat run at the origin, mapped through
/// the same frame and effect field [`warp_text_on_path`] baked its outline with.
#[allow(clippy::too_many_arguments)]
pub(super) fn record_textpath_stops(
    text: &str,
    style: &TextStyle,
    fx: &crate::PathEffect,
    path_d: &str,
    advance: f64,
    span: (usize, usize),
    ctx: &Ctx,
) {
    let tolerance = ctx.quality.text_tolerance().max(1e-3);
    let Some(frame) = PathFrame::new(path_d, tolerance) else {
        return;
    };
    let Some(field) = path_effect_field(&frame, fx, advance) else {
        return;
    };
    let metrics = ctx.m.font_metrics(style, style.size);
    if let Some(mut run) = stop_run(text, style, style.size, 0.0, 0.0, &metrics, ctx.outliner) {
        run.map(field.as_ref());
        ctx.record_stops(span, run);
    }
}
