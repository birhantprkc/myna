// examples/render_check.rs — the HUD render check (feature 004, T121/T133).
//
// The unit tests prove what the indicators *should* draw; this proves GTK
// actually paints it. A real pill is driven through a recording session at a
// known level, once per `hud-style`, and the active indicator is rasterised
// with the window's own GSK renderer and read back. Failure modes that only
// show as a wrong overlay:
//
//   1. the indicator draws nothing (hidden, zero-sized, or a colour that
//      resolved to transparent);
//   2. the level does not reach the fill (the bar is empty or pinned full, the
//      meter lights no segment or all of them);
//   3. the colour is not the theme's (a grey accent bar, an uncoloured meter);
//   4. the style switch leaves the other indicator painting too.
//
// Run with:  xvfb-run -a -s "-screen 0 640x480x24" \
//                cargo run -p myna-hud --example render_check
// Exit code 0 = every style rendered.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gtk::prelude::*;
use gtk::{glib, graphene};
use gtk4 as gtk;
use libadwaita as adw;

use myna_hud::hud_logic::HudStyle;
use myna_hud::pill::Pill;
use myna_hud::segmented_meter::BAR_COUNT;
use myna_hud::simulator::envelope_to_levels;
use myna_hud::states::{state_to_descriptor, wire};

/// The level the check drives, as the lab's slider would.
const ENVELOPE: f64 = 0.5;

/// Long enough, once the indicator is on screen, for the eased level to settle.
const SETTLE: Duration = Duration::from_millis(600);

/// How long an indicator may take to reach the screen on a loaded runner.
const MAP_DEADLINE: Duration = Duration::from_secs(20);

/// A rasterised widget: premultiplied BGRA rows, as `gdk::Texture::download`
/// writes them.
struct Frame {
    width: usize,
    height: usize,
    pixels: Vec<u8>,
}

impl Frame {
    /// `(r, g, b, a)` at a pixel, un-premultiplied.
    fn rgba(&self, x: usize, y: usize) -> (u8, u8, u8, u8) {
        let i = (y * self.width + x) * 4;
        let [b, g, r, a] = [
            self.pixels[i],
            self.pixels[i + 1],
            self.pixels[i + 2],
            self.pixels[i + 3],
        ];
        let un = |c: u8| match a {
            0 => 0,
            _ => ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8,
        };
        (un(r), un(g), un(b), a)
    }
}

/// Paint `widget` the way its window would and read the pixels back. `None`
/// when it paints nothing at all.
fn render(widget: &gtk::Widget) -> Option<Frame> {
    let (width, height) = (widget.width(), widget.height());
    if width <= 0 || height <= 0 {
        return None;
    }
    let snapshot = gtk::Snapshot::new();
    gtk::WidgetPaintable::new(Some(widget)).snapshot(&snapshot, width as f64, height as f64);
    let node = snapshot.to_node()?;
    let renderer = widget.native()?.renderer()?;
    let bounds = graphene::Rect::new(0.0, 0.0, width as f32, height as f32);
    let texture = renderer.render_texture(node, Some(&bounds));
    let (width, height) = (texture.width() as usize, texture.height() as usize);
    let mut pixels = vec![0; width * height * 4];
    texture.download(&mut pixels, width * 4);
    Some(Frame {
        width,
        height,
        pixels,
    })
}

/// A saturated colour rather than a grey: what an accent or a VU zone is.
fn chromatic((r, g, b, _): (u8, u8, u8, u8)) -> bool {
    r.max(g).max(b) - r.min(g).min(b) > 40
}

/// The bar: an opaque accent fill from the left edge, about as long as the
/// level, over a faint track.
fn check_bar(frame: &Frame, problems: &mut Vec<String>) {
    let row = frame.height / 2;
    let lit: Vec<bool> = (0..frame.width)
        .map(|x| frame.rgba(x, row).3 > 200)
        .collect();
    let fill = lit.iter().take_while(|&&on| on).count();
    let fraction = fill as f64 / frame.width as f64;
    println!(
        "render-check: bar {}x{} fill {fill}px = {:.0}%",
        frame.width,
        frame.height,
        fraction * 100.0
    );
    if fill == 0 {
        problems.push("bar: no fill from the left edge".into());
        return;
    }
    if lit[fill..].iter().any(|&on| on) {
        problems.push("bar: the fill is not one run from the left edge".into());
    }
    if !(0.25..=0.75).contains(&fraction) {
        problems.push(format!(
            "bar: fill is {fraction:.2} of the width for a {ENVELOPE} level"
        ));
    }
    if !chromatic(frame.rgba(fill / 2, row)) {
        problems.push(format!(
            "bar: the fill is not the accent colour: {:?}",
            frame.rgba(fill / 2, row)
        ));
    }
    let track = frame.rgba(frame.width - 2, row).3;
    if !(8..=64).contains(&track) {
        problems.push(format!(
            "bar: the track's alpha is {track}, not a faint groove"
        ));
    }
}

/// The meter: segments lit from the left up to about the level, the first
/// one green, the unlit ones dim.
fn check_meter(frame: &Frame, problems: &mut Vec<String>) {
    let row = frame.height / 2;
    let gap = frame.width as f64 / BAR_COUNT as f64;
    let centre = |i: usize| ((i as f64 + 0.5) * gap) as usize;
    let alpha: Vec<u8> = (0..BAR_COUNT)
        .map(|i| frame.rgba(centre(i), row).3)
        .collect();
    let lit = alpha.iter().take_while(|&&a| a > 200).count();
    println!(
        "render-check: vumeter {}x{} lit {lit} of {BAR_COUNT}",
        frame.width, frame.height
    );
    if !(BAR_COUNT / 4..=BAR_COUNT * 3 / 4).contains(&lit) {
        problems.push(format!(
            "vumeter: {lit} of {BAR_COUNT} segments lit for a {ENVELOPE} level"
        ));
    }
    if alpha[lit..].iter().any(|&a| !(8..=80).contains(&a)) {
        problems.push(format!(
            "vumeter: the segments past the level are not dim: {alpha:?}"
        ));
    }
    let (r, g, b, _) = frame.rgba(centre(0), row);
    if !(g > r && g > b && chromatic((r, g, b, 255))) {
        problems.push(format!(
            "vumeter: the first segment is not green: {:?}",
            (r, g, b)
        ));
    }
}

/// Render the style's own indicator and check it, and require the other one
/// to paint nothing.
fn check(pill: &Pill, style: HudStyle, problems: &mut Vec<String>) {
    let (shown, hidden) = match style {
        HudStyle::Bar => (pill.bar(), pill.meter()),
        HudStyle::Vumeter => (pill.meter(), pill.bar()),
    };
    if render(hidden).is_some() {
        problems.push(format!("{style:?}: the other indicator still paints"));
    }
    match (render(shown), style) {
        (None, _) => problems.push(format!("{style:?}: nothing was drawn")),
        (Some(frame), HudStyle::Bar) => check_bar(&frame, problems),
        (Some(frame), HudStyle::Vumeter) => check_meter(&frame, problems),
    }
}

/// Run `then` `SETTLE` after `widget`, mapped and allocated, has painted: a
/// widget's paintable replays its last painted frame, so until one is painted
/// it renders nothing. Neither a fixed delay nor a later frame of the clock
/// proves that (a frame can pass before the widget draws), so this waits
/// until it renders. `then` gets false if the deadline passed first.
fn when_on_screen(widget: gtk::Widget, then: impl FnOnce(bool) + 'static) {
    let started = std::time::Instant::now();
    let mut then = Some(then);
    glib::timeout_add_local(Duration::from_millis(20), move || {
        let ready = widget.is_mapped() && render(&widget).is_some();
        if !ready && started.elapsed() < MAP_DEADLINE {
            return glib::ControlFlow::Continue;
        }
        let then = then.take().expect("runs once");
        glib::timeout_add_local_once(SETTLE, move || then(ready));
        glib::ControlFlow::Break
    });
}

fn main() {
    let app = adw::Application::builder()
        .application_id("com.canonical.Myna.HudRenderCheck")
        .build();

    app.connect_activate(|app| {
        let pill = Pill::new();
        let window = gtk::ApplicationWindow::new(app);
        window.set_title(Some("myna render-check"));
        window.set_child(Some(pill.widget()));
        window.present();

        pill.apply_descriptor(state_to_descriptor(Some(wire::RECORDING), "Listening"));

        // Keep the level fresh, as the publisher does; a stale one decays.
        let (rms, peak) = envelope_to_levels(ENVELOPE);
        let feed = pill.clone();
        glib::timeout_add_local(Duration::from_millis(50), move || {
            feed.push_level(rms, peak);
            glib::ControlFlow::Continue
        });

        let problems: Rc<RefCell<Vec<String>>> = Rc::default();
        let app = app.clone();
        when_on_screen(pill.bar().clone(), move |ready| {
            if !ready {
                problems
                    .borrow_mut()
                    .push("Bar: never reached the screen".into());
            }
            check(&pill, HudStyle::Bar, &mut problems.borrow_mut());
            pill.set_hud_style(HudStyle::Vumeter);
            when_on_screen(pill.meter().clone(), move |ready| {
                if !ready {
                    problems
                        .borrow_mut()
                        .push("Vumeter: never reached the screen".into());
                }
                check(&pill, HudStyle::Vumeter, &mut problems.borrow_mut());
                let problems = problems.borrow();
                for p in problems.iter() {
                    eprintln!("render-check: FAIL — {p}");
                }
                if problems.is_empty() {
                    println!("render-check: OK — every style rendered");
                }
                app.quit();
                std::process::exit(i32::from(!problems.is_empty()));
            });
        });
    });

    std::process::exit(app.run().get() as i32);
}
