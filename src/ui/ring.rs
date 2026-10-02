//! The countdown ring. The whole countdown is one Core Animation animation, which the render
//! server plays on its own: this process is not woken per frame.

use std::f64::consts::{FRAC_PI_2, PI};

use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{NSColor, NSScreen, NSView};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_core_graphics::CGMutablePath;
use objc2_foundation::{NSNumber, ns_string};
use objc2_quartz_core::{
    CABasicAnimation, CALayer, CAMediaTiming, CAShapeLayer, CATransaction, kCALineCapRound,
};

use super::rect;

pub const SIZE: f64 = 22.0;
const LINE_WIDTH: f64 = 3.0;

pub struct Ring {
    pub view: Retained<NSView>,
    progress: Retained<CAShapeLayer>,
}

impl Ring {
    pub fn new(mtm: MainThreadMarker, x: f64, y: f64) -> Ring {
        let view = NSView::initWithFrame(NSView::alloc(mtm), rect(x, y, SIZE, SIZE));
        let host = CALayer::new();
        view.setLayer(Some(&host));
        view.setWantsLayer(true);

        let scale = NSScreen::mainScreen(mtm).map_or(2.0, |s| s.backingScaleFactor());
        let bounds = CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(SIZE, SIZE));
        let center = SIZE / 2.0;
        let path = CGMutablePath::new();
        // Starts at 12 o'clock and runs clockwise (the hosted layer has a bottom-left origin).
        unsafe {
            CGMutablePath::add_arc(
                Some(&path),
                std::ptr::null(),
                center,
                center,
                (SIZE - LINE_WIDTH) / 2.0,
                FRAC_PI_2,
                FRAC_PI_2 - 2.0 * PI,
                true,
            )
        };

        let shape = |color: &NSColor| {
            let layer = CAShapeLayer::new();
            layer.setFrame(bounds);
            layer.setContentsScale(scale);
            layer.setPath(Some(&path));
            layer.setFillColor(None);
            layer.setStrokeColor(Some(&color.CGColor()));
            layer.setLineWidth(LINE_WIDTH);
            layer.setLineCap(unsafe { kCALineCapRound });
            layer
        };
        let track = shape(&NSColor::quaternaryLabelColor());
        let progress = shape(&NSColor::controlAccentColor());
        host.addSublayer(&track);
        host.addSublayer(&progress);
        Ring { view, progress }
    }

    /// Animates from `remaining / period` of a full ring down to empty over `remaining` seconds.
    /// The arc's start moves clockwise from 12 o'clock, so the ring empties toward the right
    /// like a clock hand sweeping round.
    pub fn start(&self, remaining: f64, period: f64) {
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        self.progress.removeAllAnimations();
        self.progress.setStrokeStart(1.0);
        let animation = CABasicAnimation::animationWithKeyPath(Some(ns_string!("strokeStart")));
        let elapsed = 1.0 - (remaining / period).clamp(0.0, 1.0);
        unsafe {
            animation.setFromValue(Some(&NSNumber::new_f64(elapsed)));
            animation.setToValue(Some(&NSNumber::new_f64(1.0)));
        }
        animation.setDuration(remaining.max(0.0));
        self.progress
            .addAnimation_forKey(&animation, Some(ns_string!("countdown")));
        CATransaction::commit();
    }
}
