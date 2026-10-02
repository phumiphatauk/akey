//! Popover contents and modes. Everything here lives only while the popover is open.

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{MainThreadMarker, MainThreadOnly, Message, sel};
use objc2_app_kit::{
    NSAnimatablePropertyContainer, NSAnimationContext, NSBezelStyle, NSButtonType, NSColor, NSControlSize,
    NSControlStateValueOff, NSControlStateValueOn, NSFont, NSFontWeightBold, NSFontWeightRegular, NSPopover,
    NSPopoverBehavior, NSPopoverDelegate, NSScrollView, NSStatusBarButton, NSTextField, NSView,
    NSViewController,
};
use objc2_foundation::{NSArray, NSPoint, NSRectEdge, NSSize, NSString, NSTimer};

use super::form::{Form, FormText};
use super::ring::{self, Ring};
use super::row::RowView;
use super::{FlippedView, WIDTH, font, label, rect, symbol_button, text_button};
use crate::app::one_shot_timer;
use crate::{accounts, keychain, totp};

const TOOLBAR_HEIGHT: f64 = 38.0;
const ROW_HEIGHT: f64 = 52.0;
const ROW_INSET: f64 = 6.0;
const MAX_VISIBLE_ROWS: usize = 8;
const BOTTOM_PAD: f64 = 6.0;
/// How long the other rows take to slide out of the way during a drag.
const SHUFFLE_SECONDS: f64 = 0.15;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// Click a row to copy its code.
    List,
    /// Rows show edit / remove buttons.
    Edit,
    /// Row `n` asks for confirmation before removing.
    ConfirmRemove(usize),
    Add,
    EditAccount(usize),
}

/// A row whose code is live and must be refreshed at its period boundary.
struct LiveRow {
    index: usize,
    code: Retained<NSTextField>,
    ring: Option<Ring>,
    period: f64,
    boundary: f64,
}

pub struct Popover {
    mtm: MainThreadMarker,
    pub popover: Retained<NSPopover>,
    root: Retained<NSView>,
    target: Retained<AnyObject>,
    names: Vec<String>,
    pub mode: Mode,
    live: Vec<LiveRow>,
    copied_labels: Vec<Option<Retained<NSTextField>>>,
    /// Account rows in list order (index = tag).
    rows: Vec<Retained<RowView>>,
    /// Where the row being dragged would land if dropped now.
    drop_target: Option<usize>,
    form: Option<Form>,
    refresh_timer: Option<Retained<NSTimer>>,
}

impl Popover {
    /// `target` receives every button action and timer; it is also the popover's delegate.
    pub fn new<T>(mtm: MainThreadMarker, target: &T, mode: Mode) -> Popover
    where
        T: AsRef<AnyObject> + objc2::Message + NSPopoverDelegate,
    {
        let popover = NSPopover::new(mtm);
        popover.setAnimates(true);
        popover.setDelegate(Some(ProtocolObject::from_ref(target)));
        let root = FlippedView::new(mtm, rect(0.0, 0.0, WIDTH, 100.0));
        let controller = NSViewController::new(mtm);
        controller.setView(&root);
        popover.setContentViewController(Some(&controller));

        let mut this = Popover {
            mtm,
            popover,
            root: Retained::into_super(root),
            target: target.as_ref().retain(),
            names: Vec::new(),
            mode,
            live: Vec::new(),
            copied_labels: Vec::new(),
            rows: Vec::new(),
            drop_target: None,
            form: None,
            refresh_timer: None,
        };
        this.reload_names();
        this.render();
        this
    }

    pub fn show(&self, button: &NSStatusBarButton) {
        self.popover
            .showRelativeToRect_ofView_preferredEdge(button.bounds(), button, NSRectEdge::MinY);
        if let Some(form) = &self.form {
            form.focus();
        }
    }

    /// The popover window's frame view (contents plus chrome), for `ui::snapshot`.
    pub fn frame_view(&self) -> Option<Retained<NSView>> {
        // SAFETY: only used to read the view hierarchy for drawing.
        unsafe { self.root.window()?.contentView()?.superview() }
    }

    pub fn name(&self, index: usize) -> Option<&str> {
        self.names.get(index).map(String::as_str)
    }

    pub fn set_mode(&mut self, mode: Mode) {
        self.mode = mode;
        self.render();
        if let Some(form) = &self.form {
            form.focus();
        }
    }

    pub fn show_copied(&self, index: usize) {
        if let Some(Some(label)) = self.copied_labels.get(index) {
            label.setHidden(false);
        }
    }

    pub fn save_form(&mut self) {
        let Some(form) = &self.form else {
            return;
        };
        let input = form.input();
        let (result, next) = match self.mode {
            Mode::Add => (accounts::add(&input), Mode::List),
            Mode::EditAccount(i) => match self.names.get(i) {
                Some(old) => (accounts::edit(old, &input), Mode::Edit),
                None => return,
            },
            _ => return,
        };
        match result {
            Ok(()) => {
                self.reload_names();
                self.set_mode(next);
            }
            Err(message) => form.show_error(&message),
        }
    }

    pub fn cancel_form(&mut self) {
        let next = if matches!(self.mode, Mode::EditAccount(_)) {
            Mode::Edit
        } else {
            Mode::List
        };
        self.set_mode(next);
    }

    pub fn remove_confirmed(&mut self) {
        if let Mode::ConfirmRemove(i) = self.mode
            && let Some(name) = self.names.get(i)
        {
            let _ = keychain::remove(name);
        }
        self.reload_names();
        self.set_mode(if self.names.is_empty() {
            Mode::List
        } else {
            Mode::Edit
        });
    }

    /// The row at `index` moved during a drag: it follows the pointer, and the other rows
    /// slide to show where it would land.
    pub fn drag_moved(&mut self, index: usize) {
        let Some(row) = self.rows.get(index) else {
            return;
        };
        let count = self.rows.len();
        let max_y = (count - 1) as f64 * ROW_HEIGHT;
        let y = (index as f64 * ROW_HEIGHT + row.drag_offset()).clamp(0.0, max_y);
        row.set_lifted(true);
        row.setFrameOrigin(NSPoint::new(ROW_INSET, y));

        let target = target_index(y, ROW_HEIGHT, count);
        if self.drop_target.replace(target) == Some(target) {
            return;
        }
        NSAnimationContext::beginGrouping();
        NSAnimationContext::currentContext().setDuration(SHUFFLE_SECONDS);
        for (other, view) in self.rows.iter().enumerate().filter(|(other, _)| *other != index) {
            let slot = slot_during_drag(other, index, target);
            view.animator()
                .setFrameOrigin(NSPoint::new(ROW_INSET, slot as f64 * ROW_HEIGHT));
        }
        NSAnimationContext::endGrouping();
    }

    /// The drag ended: saves the new order (if it changed) and lays the list out again.
    pub fn drop_row(&mut self, index: usize) {
        if let Some(target) = self.drop_target.take()
            && target != index
            && index < self.names.len()
        {
            keychain::move_item(&mut self.names, index, target);
            let _ = keychain::set_order(&self.names);
        }
        self.render();
    }

    /// Timer callback: updates rows whose period ended and restarts their rings.
    pub fn refresh(&mut self) {
        self.refresh_timer = None;
        let now = crate::unix_time_f64();
        for row in self.live.iter_mut().filter(|row| row.boundary <= now) {
            if let Some(code) = code_for(&self.names[row.index], now) {
                row.code.setStringValue(&NSString::from_str(&code.text));
            }
            row.boundary = next_boundary(now, row.period);
            if let Some(ring) = &row.ring {
                ring.start(row.boundary - now, row.period);
            }
        }
        self.schedule_refresh();
    }

    pub fn stop(&mut self) {
        if let Some(timer) = self.refresh_timer.take() {
            timer.invalidate();
        }
    }

    fn reload_names(&mut self) {
        self.names = keychain::list().unwrap_or_default();
    }

    /// Rebuilds the popover's contents for the current mode.
    fn render(&mut self) {
        self.stop();
        self.live.clear();
        self.copied_labels.clear();
        self.rows.clear();
        self.drop_target = None;
        self.form = None;
        self.root.setSubviews(&NSArray::new());

        let height = match self.mode {
            Mode::Add => self.render_form(FormText {
                title: "Add Account",
                hint: "Paste the setup key or otpauth:// link from the website's 2FA setup page.",
                name: "",
                name_placeholder: "Name (optional for otpauth:// links)",
                secret_placeholder: "Setup key or otpauth://…",
            }),
            Mode::EditAccount(i) => {
                let name = self.names.get(i).cloned().unwrap_or_default();
                self.render_form(FormText {
                    title: &format!("Edit “{name}”"),
                    hint: "Rename it, or paste a new setup key to replace its secret.",
                    name: &name,
                    name_placeholder: "Name",
                    secret_placeholder: "Leave blank to keep the current secret",
                })
            }
            Mode::List | Mode::Edit | Mode::ConfirmRemove(_) => self.render_list(),
        };

        let behavior = if self.form.is_some() {
            // Stay open while the user switches to the browser to copy the setup key.
            NSPopoverBehavior::ApplicationDefined
        } else {
            NSPopoverBehavior::Transient
        };
        self.popover.setBehavior(behavior);
        self.root.setFrameSize(NSSize::new(WIDTH, height));
        self.popover.setContentSize(NSSize::new(WIDTH, height));
        self.schedule_refresh();
    }

    fn render_form(&mut self, text: FormText) -> f64 {
        let form = Form::new(self.mtm, &self.target, &text);
        self.root.addSubview(&form.view);
        self.form = Some(form);
        Form::HEIGHT
    }

    fn render_list(&mut self) -> f64 {
        let mtm = self.mtm;
        self.render_toolbar();

        if self.names.is_empty() {
            let regular = unsafe { NSFontWeightRegular };
            let title = label(
                mtm,
                "No accounts yet",
                &font(13.0, regular),
                &NSColor::labelColor(),
                rect(0.0, TOOLBAR_HEIGHT + 8.0, WIDTH, 18.0),
            );
            let hint = label(
                mtm,
                "Click + to add one",
                &font(11.0, regular),
                &NSColor::secondaryLabelColor(),
                rect(0.0, TOOLBAR_HEIGHT + 28.0, WIDTH, 16.0),
            );
            for l in [&title, &hint] {
                l.setAlignment(objc2_app_kit::NSTextAlignment::Center);
                self.root.addSubview(l);
            }
            return TOOLBAR_HEIGHT + 56.0;
        }

        let content_height = self.names.len() as f64 * ROW_HEIGHT;
        let container: Retained<NSView> = if self.names.len() > MAX_VISIBLE_ROWS {
            let visible = MAX_VISIBLE_ROWS as f64 * ROW_HEIGHT;
            let scroll = NSScrollView::initWithFrame(
                NSScrollView::alloc(mtm),
                rect(0.0, TOOLBAR_HEIGHT, WIDTH, visible),
            );
            scroll.setDrawsBackground(false);
            scroll.setHasVerticalScroller(true);
            scroll.setAutohidesScrollers(true);
            let document = FlippedView::new(mtm, rect(0.0, 0.0, WIDTH, content_height));
            scroll.setDocumentView(Some(&document));
            self.root.addSubview(&scroll);
            Retained::into_super(document)
        } else {
            let rows = FlippedView::new(mtm, rect(0.0, TOOLBAR_HEIGHT, WIDTH, content_height));
            self.root.addSubview(&rows);
            Retained::into_super(rows)
        };

        let now = crate::unix_time_f64();
        for index in 0..self.names.len() {
            let row = self.render_row(index, now);
            container.addSubview(&row);
            self.rows.push(row);
        }

        let visible = self.names.len().min(MAX_VISIBLE_ROWS) as f64 * ROW_HEIGHT;
        TOOLBAR_HEIGHT + visible + BOTTOM_PAD
    }

    fn render_toolbar(&self) {
        let mtm = self.mtm;
        let (width, height, gap) = (36.0, 24.0, 10.0);
        let x0 = (WIDTH - 3.0 * width - 2.0 * gap) / 2.0;
        let buttons = [
            ("plus", "Add account", sel!(addAccount:)),
            ("pencil", "Edit accounts", sel!(toggleEdit:)),
            ("power", "Quit AKey", sel!(quit:)),
        ];
        for (i, (symbol, description, action)) in buttons.into_iter().enumerate() {
            let frame = rect(x0 + i as f64 * (width + gap), 8.0, width, height);
            let button = symbol_button(mtm, symbol, description, &self.target, action, frame);
            button.setBezelStyle(NSBezelStyle::Toolbar);
            // No focus ring on the first button when the popover opens.
            button.setRefusesFirstResponder(true);
            if action == sel!(toggleEdit:) {
                button.setButtonType(NSButtonType::PushOnPushOff);
                let editing = matches!(self.mode, Mode::Edit | Mode::ConfirmRemove(_));
                button.setState(if editing {
                    NSControlStateValueOn
                } else {
                    NSControlStateValueOff
                });
            }
            self.root.addSubview(&button);
        }
    }

    fn render_row(&mut self, index: usize, now: f64) -> Retained<RowView> {
        let mtm = self.mtm;
        let name = self.names[index].clone();
        let width = WIDTH - 2.0 * ROW_INSET;
        let listing = self.mode == Mode::List;
        let row = RowView::new(
            mtm,
            rect(ROW_INSET, index as f64 * ROW_HEIGHT, width, ROW_HEIGHT),
            listing,
        );
        row.setTag(index as isize);
        let regular = unsafe { NSFontWeightRegular };
        self.copied_labels.push(None);

        if self.mode == Mode::ConfirmRemove(index) {
            row.addSubview(&label(
                mtm,
                &format!("Remove “{name}”?"),
                &font(13.0, unsafe { objc2_app_kit::NSFontWeightSemibold }),
                &NSColor::labelColor(),
                rect(10.0, 6.0, width - 20.0, 18.0),
            ));
            let cancel = text_button(
                mtm,
                "Cancel",
                &self.target,
                sel!(cancelRemove:),
                rect(width - 152.0, 26.0, 70.0, 20.0),
            );
            let remove = text_button(
                mtm,
                "Remove",
                &self.target,
                sel!(confirmRemove:),
                rect(width - 78.0, 26.0, 70.0, 20.0),
            );
            remove.setHasDestructiveAction(true);
            for button in [&cancel, &remove] {
                button.setControlSize(NSControlSize::Small);
                button.setFont(Some(&NSFont::systemFontOfSize(11.0)));
                row.addSubview(button);
            }
            return row;
        }

        let code = code_for(&name, now);
        let code_color = if code.is_some() {
            NSColor::labelColor()
        } else {
            NSColor::tertiaryLabelColor()
        };
        let code_label = label(
            mtm,
            code.as_ref().map_or("—— ——", |c| &c.text),
            &NSFont::monospacedDigitSystemFontOfSize_weight(22.0, unsafe { NSFontWeightBold }),
            &code_color,
            rect(10.0, 4.0, width - 60.0, 28.0),
        );
        row.addSubview(&code_label);

        let name_label = label(
            mtm,
            &name,
            &font(11.0, regular),
            &NSColor::secondaryLabelColor(),
            rect(10.0, 31.0, width - 60.0, 15.0),
        );
        name_label.sizeToFit();
        let name_width = name_label.frame().size.width.min(width - 130.0);
        name_label.setFrame(rect(10.0, 31.0, name_width, 15.0));
        row.addSubview(&name_label);

        if listing {
            if code.is_some() {
                unsafe {
                    row.setTarget(Some(&self.target));
                    row.setAction(Some(sel!(rowClicked:)));
                }
                let copied = label(
                    mtm,
                    "✓ Copied!",
                    &font(11.0, regular),
                    &NSColor::systemGreenColor(),
                    rect(10.0 + name_width + 8.0, 31.0, 70.0, 15.0),
                );
                copied.setHidden(true);
                row.addSubview(&copied);
                self.copied_labels[index] = Some(copied);
            }
        } else {
            let edit = symbol_button(
                mtm,
                "pencil",
                "Edit",
                &self.target,
                sel!(editAccount:),
                rect(width - 62.0, 14.0, 24.0, 24.0),
            );
            let remove = symbol_button(
                mtm,
                "trash",
                "Remove",
                &self.target,
                sel!(removeAccount:),
                rect(width - 32.0, 14.0, 24.0, 24.0),
            );
            remove.setContentTintColor(Some(&NSColor::systemRedColor()));
            for button in [&edit, &remove] {
                button.setBordered(false);
                button.setTag(index as isize);
                row.addSubview(button);
            }
        }

        if let Some(code) = code {
            let boundary = next_boundary(now, code.period);
            let ring = listing.then(|| {
                let ring = Ring::new(mtm, width - 10.0 - ring::SIZE, (ROW_HEIGHT - ring::SIZE) / 2.0);
                row.addSubview(&ring.view);
                ring.start(boundary - now, code.period);
                ring
            });
            self.live.push(LiveRow {
                index,
                code: code_label,
                ring,
                period: code.period,
                boundary,
            });
        }
        row
    }

    /// One timer for the earliest expiry among visible codes; none in form mode.
    fn schedule_refresh(&mut self) {
        let Some(boundary) = self.live.iter().map(|row| row.boundary).reduce(f64::min) else {
            return;
        };
        let delay = (boundary - crate::unix_time_f64()).max(0.0) + 0.05;
        let timer = one_shot_timer(&self.target, delay, sel!(refreshCodes:));
        timer.setTolerance(0.1);
        if let Some(old) = self.refresh_timer.replace(timer) {
            old.invalidate();
        }
    }
}

impl Drop for Popover {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Code {
    text: String,
    period: f64,
}

/// Reads the account from the Keychain and formats its current code. The secret is dropped
/// (and wiped) before returning.
fn code_for(name: &str, now: f64) -> Option<Code> {
    let account = keychain::get(name).ok()?;
    Some(Code {
        text: totp::format_code_pairs(account.code_at(now as u64), account.digits),
        period: account.period as f64,
    })
}

fn next_boundary(now: f64, period: f64) -> f64 {
    (now / period).floor() * period + period
}

/// The slot a dragged row at `y` (top edge) would drop into.
fn target_index(y: f64, row_height: f64, count: usize) -> usize {
    ((y / row_height).round().max(0.0) as usize).min(count.saturating_sub(1))
}

/// Where row `other` sits while row `from` hovers over slot `to`.
fn slot_during_drag(other: usize, from: usize, to: usize) -> usize {
    if from < to && (from + 1..=to).contains(&other) {
        other - 1
    } else if to < from && (to..from).contains(&other) {
        other + 1
    } else {
        other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drop_target_rounds_to_nearest_slot() {
        assert_eq!(target_index(0.0, 52.0, 4), 0);
        assert_eq!(target_index(25.0, 52.0, 4), 0);
        assert_eq!(target_index(27.0, 52.0, 4), 1);
        assert_eq!(target_index(500.0, 52.0, 4), 3);
        assert_eq!(target_index(-10.0, 52.0, 4), 0);
    }

    #[test]
    fn rows_make_room_for_the_dragged_row() {
        // Dragging row 0 down to slot 2: rows 1 and 2 move up, row 3 stays.
        let slots: Vec<usize> = [1, 2, 3].iter().map(|&r| slot_during_drag(r, 0, 2)).collect();
        assert_eq!(slots, [0, 1, 3]);
        // Dragging row 3 up to slot 1: rows 1 and 2 move down, row 0 stays.
        let slots: Vec<usize> = [0, 1, 2].iter().map(|&r| slot_during_drag(r, 3, 1)).collect();
        assert_eq!(slots, [0, 2, 3]);
        // The final order matches what `move_item` produces.
        let mut names = vec!["a", "b", "c", "d"];
        keychain::move_item(&mut names, 3, 1);
        assert_eq!(names, ["a", "d", "b", "c"]);
    }
}
