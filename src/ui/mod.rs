//! The popover UI: built when the status item is clicked, released when the popover closes.

mod form;
mod popover;
mod ring;
mod row;

pub use popover::{Mode, Popover};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSBitmapImageFileType, NSButton, NSColor, NSFont, NSFontWeight, NSImage, NSLineBreakMode,
    NSMenu, NSTextField, NSView,
};
use objc2_foundation::{NSDictionary, NSPoint, NSRect, NSSize, NSString, ns_string};

use crate::app::menu_item;

pub const WIDTH: f64 = 260.0;

define_class!(
    /// A plain container with a top-left origin, which makes top-down layout straightforward.
    // SAFETY: NSView has no subclassing requirements and `FlippedView` does not implement Drop.
    #[unsafe(super(NSView, objc2_app_kit::NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "AKeyFlippedView"]
    pub struct FlippedView;

    impl FlippedView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }
    }
);

impl FlippedView {
    pub fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        unsafe { msg_send![Self::alloc(mtm), initWithFrame: frame] }
    }
}

pub fn rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
}

pub fn font(size: f64, weight: NSFontWeight) -> Retained<NSFont> {
    NSFont::systemFontOfSize_weight(size, weight)
}

pub fn label(
    mtm: MainThreadMarker,
    text: &str,
    font: &NSFont,
    color: &NSColor,
    frame: NSRect,
) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    label.setFont(Some(font));
    label.setTextColor(Some(color));
    label.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    label.setFrame(frame);
    label
}

pub fn wrapping_label(
    mtm: MainThreadMarker,
    text: &str,
    font: &NSFont,
    color: &NSColor,
    frame: NSRect,
) -> Retained<NSTextField> {
    let label = NSTextField::wrappingLabelWithString(&NSString::from_str(text), mtm);
    label.setFont(Some(font));
    label.setTextColor(Some(color));
    label.setFrame(frame);
    label
}

/// A button showing an SF Symbol.
pub fn symbol_button(
    mtm: MainThreadMarker,
    symbol: &str,
    description: &str,
    target: &AnyObject,
    action: Sel,
    frame: NSRect,
) -> Retained<NSButton> {
    let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str(symbol),
        Some(&NSString::from_str(description)),
    )
    .unwrap_or_default();
    let button = unsafe { NSButton::buttonWithImage_target_action(&image, Some(target), Some(action), mtm) };
    button.setFrame(frame);
    button.setToolTip(Some(&NSString::from_str(description)));
    button
}

pub fn text_button(
    mtm: MainThreadMarker,
    title: &str,
    target: &AnyObject,
    action: Sel,
    frame: NSRect,
) -> Retained<NSButton> {
    let button = unsafe {
        NSButton::buttonWithTitle_target_action(&NSString::from_str(title), Some(target), Some(action), mtm)
    };
    button.setFrame(frame);
    button
}

/// Without a main menu, ⌘V/⌘C/⌘X/⌘A/⌘Z do nothing in text fields. An Accessory app never shows
/// its main menu, but AppKit still routes key equivalents through it. Installed once, lazily.
pub fn install_edit_menu(mtm: MainThreadMarker) {
    let app = NSApplication::sharedApplication(mtm);
    if app.mainMenu().is_some() {
        return;
    }
    let edit = NSMenu::initWithTitle(NSMenu::alloc(mtm), ns_string!("Edit"));
    for (title, action, key) in [
        ("Undo", sel!(undo:), "z"),
        ("Redo", sel!(redo:), "Z"),
        ("Cut", sel!(cut:), "x"),
        ("Copy", sel!(copy:), "c"),
        ("Paste", sel!(paste:), "v"),
        ("Select All", sel!(selectAll:), "a"),
    ] {
        // No target: the action goes to the first responder (the focused text field).
        edit.addItem(&menu_item(mtm, title, Some(action), key));
    }
    let edit_item = menu_item(mtm, "Edit", None, "");
    edit_item.setSubmenu(Some(&edit));
    let main = NSMenu::new(mtm);
    main.addItem(&edit_item);
    app.setMainMenu(Some(&main));
}

/// Development aid: renders `view` and its subviews to a PNG, which needs no Screen Recording
/// permission. Layer-hosted content (the rings) and the popover material are not captured.
pub fn snapshot(view: &NSView, path: &str) -> bool {
    let bounds = view.bounds();
    let Some(rep) = view.bitmapImageRepForCachingDisplayInRect(bounds) else {
        return false;
    };
    view.cacheDisplayInRect_toBitmapImageRep(bounds, &rep);
    let data =
        unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new()) };
    data.is_some_and(|data| data.writeToFile_atomically(&NSString::from_str(path), true))
}
