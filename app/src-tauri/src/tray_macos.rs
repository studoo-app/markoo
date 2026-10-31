//! The colored dot of the menu bar icon.
//!
//! macOS only tints a monochrome image to match each menu bar, light or dark.
//! So the anchor is that image, and the dot is a view of its own on top of it.
//! Tauri does not give access to the button of the icon: it is looked for among
//! the windows of the app. If it cannot be found, the anchor stays without a dot.

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::{AnyThread, MainThreadMarker};
use objc2_app_kit::{
    NSApplication, NSAutoresizingMaskOptions, NSImage, NSImageView, NSStatusBarButton, NSView,
    NSWindowOrderingMode,
};
use objc2_foundation::{NSData, NSSize};

/// Height of the images in the menu bar, as `tray-icon` gives its own.
const ICON_POINTS: f64 = 18.0;

thread_local! {
    /// Only used on the main thread, like the rest of AppKit.
    static DOT: RefCell<Option<Retained<NSImageView>>> = const { RefCell::new(None) };
}

fn find_button(view: &NSView) -> Option<Retained<NSStatusBarButton>> {
    for child in view.subviews().iter() {
        if let Ok(button) = child.clone().downcast::<NSStatusBarButton>() {
            return Some(button);
        }
        if let Some(button) = find_button(&child) {
            return Some(button);
        }
    }
    None
}

fn status_button(mtm: MainThreadMarker) -> Option<Retained<NSStatusBarButton>> {
    NSApplication::sharedApplication(mtm)
        .windows()
        .iter()
        .find_map(|window| {
            let content = window.contentView()?;
            match content.clone().downcast::<NSStatusBarButton>() {
                Ok(button) => Some(button),
                Err(_) => find_button(&content),
            }
        })
}

/// Shows a dot, from the bytes of its image. To be called on the main thread.
pub fn show_dot(png: &[u8]) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let data = NSData::with_bytes(png);
    let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) else {
        return;
    };
    image.setSize(NSSize::new(ICON_POINTS, ICON_POINTS));
    DOT.with_borrow_mut(|dot| {
        if let Some(view) = dot {
            view.setImage(Some(&image));
            return;
        }
        let Some(button) = status_button(mtm) else {
            return;
        };
        let view = NSImageView::imageViewWithImage(&image, mtm);
        view.setFrame(button.bounds());
        view.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable
                | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        // Below the view of `tray-icon` that receives the clicks, or the menu
        // would no longer open.
        button.addSubview_positioned_relativeTo(&view, NSWindowOrderingMode::Below, None);
        *dot = Some(view);
    });
}
