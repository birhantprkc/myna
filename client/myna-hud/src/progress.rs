//! progress — a plain `GtkProgressBar` HUD indicator (feature 004).
//!
//! This is the `progress` `hud-style`: the simplest possible view, a stock
//! `gtk::ProgressBar`. Its `fraction` is driven by the current state through
//! [`crate::hud_logic::indicator_state`] — a static level while
//! recording, a bounce while loading/transcribing, a settle while finalizing,
//! and a full, gentle pulse on a `notice`. Colour comes from CSS: the bar's
//! `progress` sub-node is styled `@accent_bg_color`, and the recoverable class
//! switches it to the warning colour. No hardcoded RGB, no cairo.

use std::rc::Rc;

use gtk::glib;
use gtk::prelude::*;
use gtk4 as gtk;

use crate::indicator::Indicator;

/// The bar's height, matching the other indicator views, so the `hud-style`
/// options occupy the same footprint.
pub const METER_HEIGHT: i32 = 8;

/// A plain progress-bar level indicator, self-driving on the pill's clock.
pub struct ProgressView {
    bar: gtk::ProgressBar,
    indicator: Indicator,
}

impl ProgressView {
    /// Build the bar and start its frame clock.
    pub fn new() -> Rc<Self> {
        let bar = gtk::ProgressBar::new();
        bar.add_css_class("myna-hud-progress");
        bar.set_height_request(METER_HEIGHT);
        bar.set_hexpand(true);
        bar.set_show_text(false);
        bar.set_fraction(0.0);

        let this = Rc::new(Self {
            bar,
            indicator: Indicator::default(),
        });
        this.connect_clock();
        this
    }

    /// The bar as a [`gtk::Widget`], to embed in the pill.
    pub fn widget(&self) -> &gtk::Widget {
        self.bar.upcast_ref()
    }

    /// A level push from the publisher. The bar tracks this while in a plain
    /// (recording/active) state.
    pub fn push_level(&self, rms: f64, peak: f64) {
        self.indicator.push_level(rms, peak);
        self.indicator.restart_easing();
        self.update_fraction();
    }

    /// Set the current dictation state (drives the fraction animation and the
    /// `notice` warning CSS class). The pill calls this on every state change.
    pub fn set_state(
        &self,
        key: crate::states::DictationState,
        severity: Option<crate::states::Severity>,
    ) {
        self.indicator.set_state(self.widget(), key, severity);
        self.update_fraction();
    }

    /// Set the reduce-animation preference (a slower pulse).
    pub fn set_reduced_motion(&self, reduced: bool) {
        if self.indicator.set_reduced_motion(reduced) {
            self.update_fraction();
        }
    }

    /// Queue a redraw (no-op beyond the widget's own repaint).
    pub fn queue_draw(&self) {
        self.bar.queue_draw();
    }

    /// Recompute and apply the fraction from the current state + level: a plain
    /// level (or full warning fill) sets the fraction; a pulse state advances
    /// the stock `GtkProgressBar`'s indeterminate block.
    fn update_fraction(&self) {
        let state = self.indicator.frame().state;
        match state.pulse {
            // A little block travelling back and forth (GTK's pulse). The
            // step controls the block width; GTK animates it internally.
            Some(pulse) => {
                self.bar.set_pulse_step(pulse.width.clamp(0.01, 1.0));
                self.bar.pulse();
            }
            None => {
                self.bar.set_fraction(state.fraction.clamp(0.0, 1.0));
            }
        }
        self.bar.queue_draw();
    }

    /// Drive the fraction from the frame clock while visible, so the bounce
    /// states animate and everything else holds a static value. Hidden (idle)
    /// costs nothing.
    fn connect_clock(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.bar.add_tick_callback(move |_widget, _clock| {
            let Some(this) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if !this.bar.is_visible() {
                return glib::ControlFlow::Continue;
            }
            this.update_fraction();
            glib::ControlFlow::Continue
        });
    }
}
