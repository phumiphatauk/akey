//! The menu bar app: an NSStatusItem whose button toggles a popover (see `ui`).
//!
//! Resource model: while the popover is closed the process has no timers and nothing to draw.
//! The popover and all of its views are built when it opens and released when it closes.
//! While it is open, one timer fires at the next code expiry; the countdown rings are Core
//! Animation animations played by the render server, not by this process. Copying arms one
//! one-shot timer that clears the clipboard 30 s later if it still holds our code.

use std::cell::{Cell, OnceCell, RefCell};
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSControl, NSEventMask, NSImage, NSMenuItem, NSPasteboard,
    NSPasteboardTypeString, NSPopoverDelegate, NSStatusBar, NSStatusItem, NSVariableStatusItemLength,
};
use objc2_foundation::{
    NSNotification, NSObject, NSObjectProtocol, NSRunLoop, NSRunLoopCommonModes, NSString, NSTimer, ns_string,
};

use crate::keychain;
use crate::ui::{self, Mode, Popover};

const CLIPBOARD_CLEAR_SECONDS: f64 = 30.0;
/// How long "✓ Copied!" stays visible before the popover closes.
const COPIED_FEEDBACK_SECONDS: f64 = 0.5;
/// Marks the pasteboard entry as a password so clipboard managers skip it (nspasteboard.org).
const CONCEALED_TYPE: &str = "org.nspasteboard.ConcealedType";

#[derive(Default)]
struct Ivars {
    status_item: OnceCell<Retained<NSStatusItem>>,
    /// The open popover, if any.
    ui: RefCell<Option<Popover>>,
    /// A just-closed popover, released on the next run loop pass rather than inside its own
    /// delegate callback.
    closed: RefCell<Option<Popover>>,
    /// Clicking the icon while the popover is open closes it (transient behavior) and then
    /// delivers the click; this suppresses the immediate reopen.
    last_closed: Cell<Option<Instant>>,
    close_timer: RefCell<Option<Retained<NSTimer>>>,
    clear_timer: RefCell<Option<Retained<NSTimer>>>,
    /// Pasteboard change count right after our copy; anything else means the user copied over it.
    copied_change_count: Cell<Option<isize>>,
}

define_class!(
    // SAFETY: NSObject has no subclassing requirements and `Controller` does not implement Drop.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "AKeyController"]
    #[ivars = Ivars]
    struct Controller;

    impl Controller {
        #[unsafe(method(togglePopover:))]
        fn toggle_popover(&self, _sender: Option<&AnyObject>) {
            self.toggle(Mode::List);
        }

        #[unsafe(method(rowClicked:))]
        fn row_clicked(&self, sender: &NSControl) {
            let index = sender.tag() as usize;
            let name = {
                let ui = self.ivars().ui.borrow();
                match ui.as_ref() {
                    Some(ui) if ui.mode == Mode::List => ui.name(index).map(str::to_string),
                    _ => None,
                }
            };
            let Some(name) = name else {
                return;
            };
            if !self.copy_code(&name) {
                return;
            }
            if let Some(ui) = self.ivars().ui.borrow().as_ref() {
                ui.show_copied(index);
            }
            let timer = one_shot_timer(self.as_ref(), COPIED_FEEDBACK_SECONDS, sel!(closeAfterCopy:));
            if let Some(old) = self.ivars().close_timer.replace(Some(timer)) {
                old.invalidate();
            }
        }

        #[unsafe(method(rowDragged:))]
        fn row_dragged(&self, sender: &NSControl) {
            let index = sender.tag() as usize;
            self.with_ui(|ui| ui.drag_moved(index));
        }

        #[unsafe(method(rowDropped:))]
        fn row_dropped(&self, sender: &NSControl) {
            let index = sender.tag() as usize;
            self.with_ui(|ui| ui.drop_row(index));
        }

        #[unsafe(method(addAccount:))]
        fn add_account(&self, _sender: Option<&AnyObject>) {
            self.with_ui(|ui| ui.set_mode(Mode::Add));
        }

        #[unsafe(method(toggleEdit:))]
        fn toggle_edit(&self, _sender: Option<&AnyObject>) {
            self.with_ui(|ui| {
                let next = if ui.mode == Mode::List { Mode::Edit } else { Mode::List };
                ui.set_mode(next);
            });
        }

        #[unsafe(method(editAccount:))]
        fn edit_account(&self, sender: &NSControl) {
            let index = sender.tag() as usize;
            self.with_ui(|ui| ui.set_mode(Mode::EditAccount(index)));
        }

        #[unsafe(method(removeAccount:))]
        fn remove_account(&self, sender: &NSControl) {
            let index = sender.tag() as usize;
            self.with_ui(|ui| ui.set_mode(Mode::ConfirmRemove(index)));
        }

        #[unsafe(method(cancelRemove:))]
        fn cancel_remove(&self, _sender: Option<&AnyObject>) {
            self.with_ui(|ui| ui.set_mode(Mode::Edit));
        }

        #[unsafe(method(confirmRemove:))]
        fn confirm_remove(&self, _sender: Option<&AnyObject>) {
            self.with_ui(Popover::remove_confirmed);
        }

        #[unsafe(method(formSave:))]
        fn form_save(&self, _sender: Option<&AnyObject>) {
            self.with_ui(Popover::save_form);
        }

        #[unsafe(method(formCancel:))]
        fn form_cancel(&self, _sender: Option<&AnyObject>) {
            self.with_ui(Popover::cancel_form);
        }

        #[unsafe(method(quit:))]
        fn quit(&self, _sender: Option<&AnyObject>) {
            NSApplication::sharedApplication(self.mtm()).terminate(None);
        }

        #[unsafe(method(refreshCodes:))]
        fn refresh_codes(&self, _timer: &NSTimer) {
            self.with_ui(Popover::refresh);
        }

        #[unsafe(method(closeAfterCopy:))]
        fn close_after_copy(&self, _timer: &NSTimer) {
            self.ivars().close_timer.replace(None);
            self.close_popover();
        }

        #[unsafe(method(releaseClosed:))]
        fn release_closed(&self, _timer: &NSTimer) {
            drop(self.ivars().closed.take());
            // Hand the pages freed by the popover back to the system instead of keeping them
            // cached in the malloc zones.
            unsafe { malloc_zone_pressure_relief(std::ptr::null_mut(), 0) };
        }

        #[unsafe(method(clearClipboard:))]
        fn clear_clipboard(&self, _timer: &NSTimer) {
            self.ivars().clear_timer.replace(None);
            let pasteboard = NSPasteboard::generalPasteboard();
            if self.ivars().copied_change_count.take() == Some(pasteboard.changeCount()) {
                pasteboard.clearContents();
            }
        }

        /// Debug aid: `AKEY_OPEN_ON_LAUNCH=list|edit|confirm|add`, optionally `AKEY_CLOSE_AFTER=<s>`.
        #[unsafe(method(openOnLaunch:))]
        fn open_on_launch(&self, _timer: &NSTimer) {
            let mode = match std::env::var("AKEY_OPEN_ON_LAUNCH").as_deref() {
                Ok("edit") => Mode::Edit,
                Ok("confirm") => Mode::ConfirmRemove(1),
                Ok("add") => Mode::Add,
                _ => Mode::List,
            };
            self.toggle(mode);
            if let Some(seconds) = std::env::var("AKEY_CLOSE_AFTER").ok().and_then(|s| s.parse().ok()) {
                one_shot_timer(self.as_ref(), seconds, sel!(closeAfterCopy:));
            }
            if std::env::var_os("AKEY_SNAPSHOT").is_some() {
                one_shot_timer(self.as_ref(), 1.0, sel!(takeSnapshot:));
            }
        }

        /// Debug aid: `AKEY_SNAPSHOT=/path/out.png` saves the open popover, then quits.
        #[unsafe(method(takeSnapshot:))]
        fn take_snapshot(&self, _timer: &NSTimer) {
            if let (Some(path), Some(view)) = (
                std::env::var("AKEY_SNAPSHOT").ok(),
                self.ivars().ui.borrow().as_ref().and_then(Popover::frame_view),
            ) {
                ui::snapshot(&view, &path);
            }
            NSApplication::sharedApplication(self.mtm()).terminate(None);
        }
    }

    unsafe impl NSObjectProtocol for Controller {}

    unsafe impl NSPopoverDelegate for Controller {
        #[unsafe(method(popoverDidClose:))]
        fn popover_did_close(&self, _notification: &NSNotification) {
            if let Some(mut ui) = self.ivars().ui.take() {
                ui.stop();
                self.ivars().closed.replace(Some(ui));
                one_shot_timer(self.as_ref(), 0.0, sel!(releaseClosed:));
            }
            if let Some(timer) = self.ivars().close_timer.take() {
                timer.invalidate();
            }
            self.ivars().last_closed.set(Some(Instant::now()));
            // Give focus back to the app the user was in, so a copied code can be pasted at once.
            NSApplication::sharedApplication(self.mtm()).hide(None);
        }
    }
);

impl Controller {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(Ivars::default());
        unsafe { msg_send![super(this), init] }
    }

    fn toggle(&self, mode: Mode) {
        let open = self.ivars().ui.borrow().as_ref().map(|ui| ui.popover.clone());
        if let Some(popover) = open {
            popover.close();
            return;
        }
        let just_closed = self
            .ivars()
            .last_closed
            .get()
            .is_some_and(|t| t.elapsed() < Duration::from_millis(300));
        if just_closed {
            return;
        }

        let mtm = self.mtm();
        let Some(button) = self.ivars().status_item.get().and_then(|item| item.button(mtm)) else {
            return;
        };
        ui::install_edit_menu(mtm);
        let popover = Popover::new(mtm, self, mode);
        NSApplication::sharedApplication(mtm).activate();
        popover.show(&button);
        self.ivars().ui.replace(Some(popover));
    }

    /// Runs `f` on the open popover. Never call AppKit methods that close the popover from
    /// inside `f`: the delegate callback would re-borrow `ui`.
    fn with_ui(&self, f: impl FnOnce(&mut Popover)) {
        if let Some(ui) = self.ivars().ui.borrow_mut().as_mut() {
            f(ui);
        }
    }

    fn close_popover(&self) {
        let open = self.ivars().ui.borrow().as_ref().map(|ui| ui.popover.clone());
        if let Some(popover) = open {
            popover.close();
        }
    }

    /// Re-reads the account and computes its code at click time, so a code that just rolled
    /// over is never copied. Returns false if the account cannot be read.
    fn copy_code(&self, name: &str) -> bool {
        let Ok(account) = keychain::get(name) else {
            return false;
        };
        let code = format!(
            "{:0width$}",
            account.code_at(crate::unix_time()),
            width = account.digits as usize
        );
        drop(account);

        let pasteboard = NSPasteboard::generalPasteboard();
        pasteboard.clearContents();
        pasteboard.setString_forType(&NSString::from_str(&code), unsafe { NSPasteboardTypeString });
        pasteboard.setString_forType(ns_string!(""), &NSString::from_str(CONCEALED_TYPE));
        self.ivars()
            .copied_change_count
            .set(Some(pasteboard.changeCount()));

        let timer = one_shot_timer(self.as_ref(), CLIPBOARD_CLEAR_SECONDS, sel!(clearClipboard:));
        timer.setTolerance(1.0);
        if let Some(old) = self.ivars().clear_timer.replace(Some(timer)) {
            old.invalidate();
        }
        true
    }
}

unsafe extern "C" {
    /// libSystem (`<malloc/malloc.h>`): returns free pages of `zone` (all zones if null) to the OS.
    fn malloc_zone_pressure_relief(zone: *mut std::ffi::c_void, goal: usize) -> usize;
}

/// A non-repeating timer registered in the common modes so it also fires during event tracking.
pub fn one_shot_timer(target: &AnyObject, seconds: f64, selector: Sel) -> Retained<NSTimer> {
    let timer = unsafe {
        NSTimer::timerWithTimeInterval_target_selector_userInfo_repeats(
            seconds, target, selector, None, false,
        )
    };
    unsafe { NSRunLoop::mainRunLoop().addTimer_forMode(&timer, NSRunLoopCommonModes) };
    timer
}

pub fn menu_item(mtm: MainThreadMarker, title: &str, action: Option<Sel>, key: &str) -> Retained<NSMenuItem> {
    unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(title),
            action,
            &NSString::from_str(key),
        )
    }
}

pub fn run() {
    let mtm = MainThreadMarker::new().expect("must run on the main thread");
    let app = NSApplication::sharedApplication(mtm);
    // No Dock icon or app menu, even when the binary runs outside the .app bundle.
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    let controller = Controller::new(mtm);
    let status_item = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
    if let Some(button) = status_item.button(mtm) {
        match NSImage::imageWithSystemSymbolName_accessibilityDescription(
            ns_string!("key.fill"),
            Some(ns_string!("AKey")),
        ) {
            Some(image) => {
                image.setTemplate(true);
                button.setImage(Some(&image));
            }
            None => button.setTitle(ns_string!("AKey")),
        }
        unsafe {
            button.setTarget(Some(controller.as_ref()));
            button.setAction(Some(sel!(togglePopover:)));
        }
        button.sendActionOn(NSEventMask::LeftMouseDown | NSEventMask::RightMouseDown);
    }
    let _ = controller.ivars().status_item.set(status_item);

    if std::env::var_os("AKEY_OPEN_ON_LAUNCH").is_some() {
        one_shot_timer(controller.as_ref(), 0.5, sel!(openOnLaunch:));
    }

    app.run();
    drop(controller);
}
