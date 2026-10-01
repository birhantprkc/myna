//! bar — the default HUD indicator (feature 004).
//!
//! This is the `bar` `hud-style`: a single horizontal level bar whose filled
//! portion tracks the calibrated level. The look of `vumeter.png` (the default
//! since 2026-09-03).
//!
//! Colours come from CSS, like the rest of the pill: the bar's `color` is
//! resolved by the theme (`@accent_bg_color`) and read back at snapshot time via
//! [`Widget::color`](gtk4::Widget::color). No hardcoded RGB and no colour
//! probing in the view.
//!
//! The level and state it draws come from [`crate::indicator`].

use std::rc::Rc;

use gtk::glib;
use gtk::graphene;
use gtk::gsk;
use gtk::prelude::*;
use gtk::subclass::prelude::ObjectSubclassIsExt;
use gtk4 as gtk;

use crate::indicator::Indicator;

/// The bar's height: a thin rule under the label, not a tall box. Matches
/// GNOME Shell's OSD level bar (`$osd_levelbar_height: 6px` in `_osd.scss`).
pub const METER_HEIGHT: i32 = 6;

/// The drawn thickness of the bar, in px.
const BAR_THICKNESS: f64 = 6.0;

/// Alpha of the dim track (the unfilled part), as GNOME Shell's `BarLevel`.
const TRACK_ALPHA: f64 = 0.1;

/// The unfilled track: a neutral white groove. Tinting it with the accent
/// made the fill harder to read.
const TRACK_COLOR: gtk::gdk::RGBA = gtk::gdk::RGBA::new(1.0, 1.0, 1.0, TRACK_ALPHA as f32);

mod imp {
    use super::*;
    use gtk::subclass::prelude::*;
    use gtk::subclass::widget::WidgetImpl;

    #[derive(Default)]
    pub struct BarView {
        pub(super) indicator: Indicator,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BarView {
        const NAME: &'static str = "MynaHudBar";
        type Type = super::BarView;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for BarView {}

    impl WidgetImpl for BarView {
        /// Paint the bar via Gsk: a rounded clip over the bar's bounds, then a
        /// dim track and the fill up to the state-driven fraction. The colour
        /// is the widget's CSS-resolved `color` (the accent).
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let w = widget.width() as f64;
            let h = widget.height() as f64;
            if w <= 0.0 || h <= 0.0 {
                return;
            }

            let frame = self.indicator.frame();

            // The theme-resolved accent.
            let color = widget.color();

            let bar_h = h.min(BAR_THICKNESS);
            let bar_y = (h - bar_h) / 2.0;
            let radius = (bar_h / 2.0) as f32;
            let bar_bounds = graphene::Rect::new(0.0, bar_y as f32, w as f32, bar_h as f32);
            let rounded = gsk::RoundedRect::from_rect(bar_bounds, radius);

            snapshot.push_rounded_clip(&rounded);
            let track = graphene::Rect::new(0.0, bar_y as f32, w as f32, bar_h as f32);
            snapshot.append_color(&TRACK_COLOR, &track);

            match frame.state.pulse {
                // Indeterminate activity: a little block travelling back and
                // forth (pong), tinted with the accent at the pulse's alpha
                // (semi-transparent for loading). The block gets its OWN
                // rounded corners — the track clip alone would leave hard
                // vertical edges on it.
                Some(pulse) => {
                    // A pong back-and-forth; the block gets its OWN rounded
                    // corners — the track clip alone would leave hard
                    // vertical edges on it.
                    let centre = crate::hud_logic::pulse_position(
                        frame.state_ms % pulse.period_ms.max(1.0),
                        pulse.period_ms,
                    );
                    let half = pulse.width / 2.0;
                    let x0 = w * (centre - half).clamp(0.0, 1.0);
                    let x1 = w * (centre + half).clamp(0.0, 1.0);
                    let block = graphene::Rect::new(
                        x0 as f32,
                        bar_y as f32,
                        (x1 - x0) as f32,
                        bar_h as f32,
                    );
                    let block_rounded = gsk::RoundedRect::from_rect(block, radius);
                    snapshot.push_rounded_clip(&block_rounded);
                    snapshot.append_color(&color.with_alpha(pulse.alpha as f32), &block);
                    snapshot.pop();
                }
                // A plain level: fraction of the bar.
                None => {
                    let fraction = frame.state.fraction.clamp(0.0, 1.0);
                    if fraction > 0.0 {
                        // Never narrower than the cap diameter, or a quiet
                        // moment draws a sliver with no rounded end.
                        let fill_w = (w * fraction).max(bar_h) as f32;
                        let fill = graphene::Rect::new(0.0, bar_y as f32, fill_w, bar_h as f32);
                        let fill_rounded = gsk::RoundedRect::from_rect(fill, radius);
                        snapshot.push_rounded_clip(&fill_rounded);
                        snapshot.append_color(&color.with_alpha(1.0), &fill);
                        snapshot.pop();
                    }
                }
            }

            snapshot.pop();
        }
    }
}

glib::wrapper! {
    /// A simple horizontal level bar.
    pub struct BarView(ObjectSubclass<imp::BarView>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl BarView {
    /// Build the bar.
    pub fn new() -> Rc<Self> {
        let bar: BarView = glib::Object::builder().build();
        bar.add_css_class("myna-hud-bar");
        bar.set_height_request(METER_HEIGHT);
        bar.set_hexpand(true);
        bar.set_can_focus(false);
        Rc::new(bar)
    }

    /// The bar as a [`gtk::Widget`], to embed in the pill.
    pub fn widget(&self) -> &gtk::Widget {
        self.upcast_ref()
    }

    /// A level push from the publisher. The frame timeline is deliberately
    /// NOT reset: `smooth_level` snaps to the target on a zero dt, which
    /// would jump the fill on every push instead of easing toward it.
    pub fn push_level(&self, rms: f64, peak: f64) {
        self.imp().indicator.push_level(rms, peak);
        self.queue_draw();
    }

    /// Set the current dictation state (drives the state animation). The
    /// pill calls this on every state change.
    pub fn set_state(
        &self,
        key: crate::states::DictationState,
        severity: Option<crate::states::Severity>,
    ) {
        self.imp().indicator.set_state(key, severity);
        self.queue_draw();
    }

    /// Set the reduce-animation preference (a slower pulse).
    pub fn set_reduced_motion(&self, reduced: bool) {
        if self.imp().indicator.set_reduced_motion(reduced) {
            self.queue_draw();
        }
    }
}
