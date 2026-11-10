//! A little native macOS (AppKit) code for quick capture while the app is out of sight.
//!
//! eframe skips drawing entirely while none of the app's windows are visible, which is
//! the case when the app is hidden (Cmd+H) or its window is minimized. Then the capture
//! window can never be created. So before opening it, the main window is made fully
//! transparent and click-through, and the app unhidden (or the window restored). eframe
//! sees a visible window and draws again, but only the capture window shows on screen.
//! Afterwards everything is put back the way it was.
use objc2::MainThreadMarker;
use objc2_app_kit::NSApplication;
use std::sync::Mutex;

/// What to undo once the capture window closes.
#[derive(Default)]
struct Revealed {
    /// The app was hidden (Cmd+H): hide it again.
    hidden: bool,
    /// Window numbers of windows that were minimized: minimize them again.
    minimized: Vec<isize>,
}

// A global, because the shortcut handler and the UI don't share any other state.
// `Mutex` makes it safe to touch from different places.
static REVEALED: Mutex<Option<Revealed>> = Mutex::new(None);

/// Called when the capture shortcut is pressed. Must run on the main thread
/// (macOS delivers global shortcuts there). Brings the app forward so the capture
/// window can be drawn and typed into.
pub fn reveal_for_capture() {
    // `MainThreadMarker::new()` returns None off the main thread, where AppKit isn't safe to call.
    let Some(mtm) = MainThreadMarker::new() else { return };
    let app = NSApplication::sharedApplication(mtm);
    let windows = app.windows();
    let hidden = app.isHidden();
    let minimized: Vec<isize> = windows.iter().filter(|w| w.isMiniaturized()).map(|w| w.windowNumber()).collect();
    let any_visible = windows.iter().any(|w| w.isVisible() && !w.isMiniaturized());
    if hidden || !any_visible {
        for w in windows.iter() {
            w.setAlphaValue(0.0);
            w.setIgnoresMouseEvents(true);
            if w.isMiniaturized() {
                w.deminiaturize(None);
            }
        }
        if hidden {
            app.unhideWithoutActivation();
        }
        *REVEALED.lock().unwrap() = Some(Revealed { hidden, minimized });
    }
    // Deprecated in macOS 14 in favor of `activate`, but works on every version this app supports.
    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
}

/// Called when the capture window closes: hides the app or minimizes the window again
/// if `reveal_for_capture` had to show it, then makes the windows visible again.
pub fn restore_after_capture() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let Some(revealed) = REVEALED.lock().unwrap().take() else { return };
    let app = NSApplication::sharedApplication(mtm);
    if revealed.hidden {
        // Hiding also hands focus back to the app you were using.
        app.hide(None);
    }
    for w in app.windows().iter() {
        if revealed.minimized.contains(&w.windowNumber()) {
            w.miniaturize(None);
        }
        w.setIgnoresMouseEvents(false);
        w.setAlphaValue(1.0);
    }
    if !revealed.hidden && !revealed.minimized.is_empty() {
        // Only minimized windows: step back so the previous app gets focus again.
        app.hide(None);
        app.unhideWithoutActivation();
    }
}
