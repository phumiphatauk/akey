//! The Add / Edit form shown inside the popover.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, MainThreadOnly, sel};
use objc2_app_kit::{
    NSBezelStyle, NSColor, NSFontWeightRegular, NSFontWeightSemibold, NSSecureTextField, NSTextField, NSView,
};
use objc2_foundation::{NSString, ns_string};

use super::{FlippedView, WIDTH, font, label, rect, text_button, wrapping_label};
use crate::accounts::FormInput;

const PAD: f64 = 16.0;
const FIELD_HEIGHT: f64 = 24.0;
const BUTTON_WIDTH: f64 = 80.0;

pub struct Form {
    pub view: Retained<NSView>,
    name: Retained<NSTextField>,
    secret: Retained<NSSecureTextField>,
    error: Retained<NSTextField>,
}

pub struct FormText<'a> {
    pub title: &'a str,
    pub hint: &'a str,
    pub name: &'a str,
    pub name_placeholder: &'a str,
    pub secret_placeholder: &'a str,
}

impl Form {
    pub const HEIGHT: f64 = 204.0;

    pub fn new(mtm: MainThreadMarker, target: &AnyObject, text: &FormText) -> Form {
        let width = WIDTH - 2.0 * PAD;
        let view = FlippedView::new(mtm, rect(0.0, 0.0, WIDTH, Self::HEIGHT));
        let regular = unsafe { NSFontWeightRegular };

        view.addSubview(&label(
            mtm,
            text.title,
            &font(13.0, unsafe { NSFontWeightSemibold }),
            &NSColor::labelColor(),
            rect(PAD, 14.0, width, 18.0),
        ));
        view.addSubview(&wrapping_label(
            mtm,
            text.hint,
            &font(11.0, regular),
            &NSColor::secondaryLabelColor(),
            rect(PAD, 34.0, width, 28.0),
        ));

        let name = NSTextField::initWithFrame(NSTextField::alloc(mtm), rect(PAD, 68.0, width, FIELD_HEIGHT));
        name.setStringValue(&NSString::from_str(text.name));
        name.setPlaceholderString(Some(&NSString::from_str(text.name_placeholder)));
        let secret = NSSecureTextField::initWithFrame(
            NSSecureTextField::alloc(mtm),
            rect(PAD, 100.0, width, FIELD_HEIGHT),
        );
        secret.setPlaceholderString(Some(&NSString::from_str(text.secret_placeholder)));
        unsafe {
            name.setNextKeyView(Some(&secret));
            secret.setNextKeyView(Some(&name));
        }
        view.addSubview(&name);
        view.addSubview(&secret);

        let error = wrapping_label(
            mtm,
            "",
            &font(11.0, regular),
            &NSColor::systemRedColor(),
            rect(PAD, 130.0, width, 30.0),
        );
        view.addSubview(&error);

        let buttons_y = Self::HEIGHT - PAD - 24.0;
        let save = text_button(
            mtm,
            "Save",
            target,
            sel!(formSave:),
            rect(WIDTH - PAD - BUTTON_WIDTH, buttons_y, BUTTON_WIDTH, 24.0),
        );
        save.setKeyEquivalent(ns_string!("\r"));
        let cancel = text_button(
            mtm,
            "Cancel",
            target,
            sel!(formCancel:),
            rect(
                WIDTH - PAD - 2.0 * BUTTON_WIDTH - 8.0,
                buttons_y,
                BUTTON_WIDTH,
                24.0,
            ),
        );
        cancel.setKeyEquivalent(ns_string!("\u{1b}"));
        for button in [&save, &cancel] {
            button.setBezelStyle(NSBezelStyle::Push);
            view.addSubview(button);
        }

        Form {
            view: Retained::into_super(view),
            name,
            secret,
            error,
        }
    }

    /// Reads the fields. The secret field keeps its contents so the user can fix a typo;
    /// it is cleared when the form goes away.
    pub fn input(&self) -> FormInput {
        FormInput {
            name: self.name.stringValue().to_string(),
            secret: self.secret.stringValue().to_string(),
        }
    }

    pub fn show_error(&self, message: &str) {
        self.error.setStringValue(&NSString::from_str(message));
    }

    pub fn focus(&self) {
        if let Some(window) = self.view.window() {
            window.makeFirstResponder(Some(&self.name));
        }
    }
}

impl Drop for Form {
    fn drop(&mut self) {
        self.secret.setStringValue(ns_string!(""));
    }
}
