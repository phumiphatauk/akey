//! One account row: a clickable, draggable, hover-highlighted container. Its contents are laid
//! out by `popover`. Being an NSControl gives it a weak target, an action and a tag for free.
//!
//! A press that moves less than [`DRAG_THRESHOLD`] is a click and sends the row's action. A
//! longer vertical move is a drag: the row sends `rowDragged:` on every move and `rowDropped:`
//! on release to the same target, which reads [`RowView::drag_offset`].

use std::cell::Cell;

use objc2::rc::Retained;
use objc2::{
    AnyThread, ClassType, DefinedClass, MainThreadMarker, MainThreadOnly, Message, define_class, msg_send,
    sel,
};
use objc2_app_kit::{
    NSButton, NSColor, NSControl, NSEvent, NSResponder, NSTrackingArea, NSTrackingAreaOptions, NSView,
};
use objc2_foundation::{NSObject, NSObjectProtocol, NSPoint, NSRect};

const DRAG_THRESHOLD: f64 = 4.0;

thread_local! {
    /// While any row is being dragged, the rows it passes over do not highlight.
    static DRAGGING: Cell<bool> = const { Cell::new(false) };
}

#[derive(Default)]
pub struct RowIvars {
    hoverable: Cell<bool>,
    draggable: Cell<bool>,
    /// Pointer y in the superview's coordinates at mouse down.
    press_y: Cell<Option<f64>>,
    dragging: Cell<bool>,
    drag_offset: Cell<f64>,
}

define_class!(
    // SAFETY: NSControl has no subclassing requirements and `RowView` does not implement Drop.
    #[unsafe(super(NSControl, NSView, NSResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "AKeyRowView"]
    #[ivars = RowIvars]
    pub struct RowView;

    impl RowView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        /// Clicks work even before the popover's window becomes key.
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        /// Buttons inside the row keep their own clicks; everything else is the row's.
        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self, point: NSPoint) -> Option<Retained<NSView>> {
            let hit: Option<Retained<NSView>> = unsafe { msg_send![super(self), hitTest: point] };
            match hit {
                Some(view) if view.isKindOfClass(NSButton::class()) => Some(view),
                Some(_) => {
                    let row: &NSView = self;
                    Some(row.retain())
                }
                None => None,
            }
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            self.ivars().press_y.set(self.pointer_y(event));
            self.ivars().dragging.set(false);
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            let ivars = self.ivars();
            let (Some(start), Some(now)) = (ivars.press_y.get(), self.pointer_y(event)) else {
                return;
            };
            if !ivars.draggable.get() {
                return;
            }
            let offset = now - start;
            if !ivars.dragging.get() && offset.abs() < DRAG_THRESHOLD {
                return;
            }
            if !ivars.dragging.replace(true) {
                DRAGGING.set(true);
                self.set_highlighted(true);
            }
            ivars.drag_offset.set(offset);
            // Keep this row alive even if the target rebuilds the list.
            let _alive = self.retain();
            self.send(sel!(rowDragged:));
            // Scrolls the enclosing scroll view when dragging past its edge.
            self.autoscroll(event);
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, _event: &NSEvent) {
            let _alive = self.retain();
            self.ivars().press_y.set(None);
            if self.ivars().dragging.replace(false) {
                DRAGGING.set(false);
                self.send(sel!(rowDropped:));
            } else if let Some(action) = self.action() {
                self.send(action);
            }
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, _event: &NSEvent) {
            if !DRAGGING.get() {
                self.set_highlighted(self.ivars().hoverable.get());
            }
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            if !self.ivars().dragging.get() {
                self.set_highlighted(false);
            }
        }
    }
);

impl RowView {
    /// List-mode rows are `interactive`: they highlight under the pointer and can be dragged.
    /// The click action is set by the caller.
    pub fn new(mtm: MainThreadMarker, frame: NSRect, interactive: bool) -> Retained<Self> {
        let hoverable = interactive;
        let this = Self::alloc(mtm).set_ivars(RowIvars {
            hoverable: Cell::new(interactive),
            draggable: Cell::new(interactive),
            ..Default::default()
        });
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        this.setWantsLayer(true);
        if let Some(layer) = this.layer() {
            layer.setCornerRadius(6.0);
        }
        if hoverable {
            let area = unsafe {
                NSTrackingArea::initWithRect_options_owner_userInfo(
                    NSTrackingArea::alloc(),
                    NSRect::ZERO,
                    NSTrackingAreaOptions::MouseEnteredAndExited
                        | NSTrackingAreaOptions::ActiveAlways
                        | NSTrackingAreaOptions::InVisibleRect,
                    Some(&this),
                    None,
                )
            };
            this.addTrackingArea(&area);
        }
        this
    }

    /// How far the row has been dragged vertically, in points (positive = down).
    pub fn drag_offset(&self) -> f64 {
        self.ivars().drag_offset.get()
    }

    /// Draws the row above its siblings while it is being dragged.
    pub fn set_lifted(&self, lifted: bool) {
        if let Some(layer) = self.layer() {
            layer.setZPosition(if lifted { 1.0 } else { 0.0 });
        }
    }

    fn pointer_y(&self, event: &NSEvent) -> Option<f64> {
        // SAFETY: only used for coordinate conversion.
        let parent = unsafe { self.superview() }?;
        Some(parent.convertPoint_fromView(event.locationInWindow(), None).y)
    }

    fn send(&self, action: objc2::runtime::Sel) {
        unsafe { self.sendAction_to(Some(action), self.target().as_deref()) };
    }

    fn set_highlighted(&self, on: bool) {
        if let Some(layer) = self.layer() {
            let color = on.then(|| NSColor::labelColor().colorWithAlphaComponent(0.08).CGColor());
            layer.setBackgroundColor(color.as_deref());
        }
    }
}
