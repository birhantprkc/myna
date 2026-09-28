//! A wrapping label whose lines come out balanced, as CSS `text-wrap:
//! balance` does: it is given the narrowest width that keeps the line count
//! it has at its full width, centred in that width. The width follows from
//! the label's own measurements, so it holds for any font and translation.

use gtk::glib;
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use gtk4 as gtk;

/// The narrowest width in `minimum..=full` at which the text is no taller
/// than at `full`, given its height at each width.
pub fn balanced_width(minimum: i32, full: i32, height_at: impl Fn(i32) -> i32) -> i32 {
    let target = height_at(full);
    let (mut low, mut high) = (minimum.max(1).min(full), full);
    while low < high {
        let middle = low + (high - low) / 2;
        if height_at(middle) <= target {
            high = middle;
        } else {
            low = middle + 1;
        }
    }
    high
}

mod imp {
    use std::sync::OnceLock;

    use super::*;

    pub struct BalancedLabel {
        pub label: gtk::Label,
    }

    impl Default for BalancedLabel {
        fn default() -> Self {
            Self {
                label: gtk::Label::builder()
                    .wrap(true)
                    .justify(gtk::Justification::Center)
                    .build(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for BalancedLabel {
        const NAME: &'static str = "BalancedLabel";
        type Type = super::BalancedLabel;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for BalancedLabel {
        fn properties() -> &'static [glib::ParamSpec] {
            static PROPERTIES: OnceLock<Vec<glib::ParamSpec>> = OnceLock::new();
            PROPERTIES.get_or_init(|| vec![glib::ParamSpecString::builder("label").build()])
        }

        fn set_property(&self, _id: usize, value: &glib::Value, _pspec: &glib::ParamSpec) {
            let text = value.get::<Option<String>>().ok().flatten();
            self.label.set_label(text.as_deref().unwrap_or_default());
        }

        fn property(&self, _id: usize, _pspec: &glib::ParamSpec) -> glib::Value {
            self.label.label().to_value()
        }

        fn constructed(&self) {
            self.parent_constructed();
            self.label.set_parent(&*self.obj());
        }

        fn dispose(&self) {
            self.label.unparent();
        }
    }

    impl WidgetImpl for BalancedLabel {
        fn request_mode(&self) -> gtk::SizeRequestMode {
            gtk::SizeRequestMode::HeightForWidth
        }

        fn measure(&self, orientation: gtk::Orientation, for_size: i32) -> (i32, i32, i32, i32) {
            self.label.measure(orientation, for_size)
        }

        fn size_allocate(&self, width: i32, height: i32, baseline: i32) {
            let (minimum, _, _, _) = self.label.measure(gtk::Orientation::Horizontal, -1);
            let balanced = balanced_width(minimum, width, |width| {
                self.label.measure(gtk::Orientation::Vertical, width).1
            });
            let offset = gtk::graphene::Point::new((width - balanced) as f32 / 2.0, 0.0);
            self.label.allocate(
                balanced,
                height,
                baseline,
                Some(gtk::gsk::Transform::new().translate(&offset)),
            );
        }
    }
}

glib::wrapper! {
    pub struct BalancedLabel(ObjectSubclass<imp::BalancedLabel>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl BalancedLabel {
    pub fn new(text: &str) -> Self {
        glib::Object::builder().property("label", text).build()
    }

    /// The label drawing the text.
    pub fn text_label(&self) -> gtk::Label {
        self.imp().label.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PARAGRAPH: &str = "Copy the command below and enter them in the Terminal to install all necessary components.";

    /// Greedy word wrap at one unit per character: the line count's height.
    fn height(text: &str, width: i32) -> i32 {
        let mut lines = 1;
        let mut used = 0;
        for word in text.split(' ') {
            let length = word.len() as i32;
            if used > 0 && used + 1 + length > width {
                lines += 1;
                used = length;
            } else {
                used += if used > 0 { 1 } else { 0 } + length;
            }
        }
        lines
    }

    #[test]
    fn the_narrowest_width_keeping_the_line_count_is_found() {
        let width = balanced_width(11, 80, |width| height(PARAGRAPH, width));
        assert_eq!(height(PARAGRAPH, width), 2);
        assert_eq!(height(PARAGRAPH, width - 1), 3);
        assert!(width <= PARAGRAPH.len() as i32 / 2 + 11, "{width}");
    }

    #[test]
    fn one_line_keeps_its_own_width_and_degenerate_bounds_hold() {
        assert_eq!(
            balanced_width(5, 40, |width| height("Set up Dictation", width)),
            16
        );
        assert_eq!(balanced_width(50, 40, |_| 1), 40);
        assert_eq!(balanced_width(0, 0, |_| 1), 0);
    }

    fn line_widths(label: &gtk::Label) -> Vec<i32> {
        let layout = label.layout();
        (0..layout.line_count())
            .filter_map(|line| layout.line_readonly(line))
            .map(|line| line.pixel_extents().1.width())
            .collect()
    }

    fn allocate(widget: &impl IsA<gtk::Widget>, width: i32) {
        let height = widget.measure(gtk::Orientation::Vertical, width).1;
        widget.size_allocate(&gtk::Allocation::new(0, 0, width, height), -1);
    }

    #[test]
    fn wrapped_lines_come_out_balanced_and_one_line_stays_one() {
        crate::ui::on_gtk_thread(|| {
            let balanced = BalancedLabel::new(PARAGRAPH);
            let plain = gtk::Label::builder().label(PARAGRAPH).wrap(true).build();
            // Room for all but the last word or so: plain wrapping orphans it.
            let width = plain.measure(gtk::Orientation::Horizontal, -1).1 * 9 / 10;
            allocate(&plain, width);
            allocate(&balanced, width);

            let greedy = line_widths(&plain);
            let lines = line_widths(&balanced.text_label());
            assert_eq!(greedy.len(), 2, "{greedy:?}");
            assert!(greedy[1] * 3 < greedy[0], "{greedy:?}");
            assert_eq!(lines.len(), 2, "{lines:?}");
            let (short, long) = (lines.iter().min().unwrap(), lines.iter().max().unwrap());
            assert!(short * 10 >= long * 8, "{lines:?}");
            let bounds = balanced.text_label().compute_bounds(&balanced).unwrap();
            assert!(bounds.width() < width as f32 - 8.0, "{bounds:?}");
            assert!(
                (bounds.x() * 2.0 + bounds.width() - width as f32).abs() <= 1.0,
                "not centred: {bounds:?}"
            );

            let title = BalancedLabel::new("Set up Dictation");
            allocate(&title, width);
            assert_eq!(line_widths(&title.text_label()).len(), 1);
        });
    }
}
