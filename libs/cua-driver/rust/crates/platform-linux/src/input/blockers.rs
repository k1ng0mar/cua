//! Modal-dialog blockers for input actions.
//!
//! A modal dialog owned by ANOTHER application holds the X11 core focus, so a
//! background keystroke aimed at the caller's pid is delivered to the dialog
//! (or dropped), and the action's verdict still reads "unverifiable" with no
//! hint about the dialog. An agent then retries, and the retry can land on a
//! button. Reproduced live on X11: `get_window_state` on the target window
//! reported no `dialogs` key at all, and a background `type_text` said only
//! "Typed N character(s) (via AT-SPI)" while an `xmessage` dialog owned the
//! focus.
//!
//! A dialog is only a blocker when it is MAPPED, MODAL, owned by a different
//! pid than the target, and the core keyboard focus resolves to it. A dialog
//! the target itself owns is that app's own business and is left alone.

use serde_json::{json, Value};

/// A modal dialog currently holding the keyboard focus away from the target.
#[derive(Clone, Debug)]
pub struct FocusBlocker {
    pub window_id: u64,
    pub title: String,
    pub pid: Option<u32>,
    /// `true` when the core focus is the dialog itself, `false` when it sits
    /// on another window belonging to the dialog's application.
    pub focus_is_dialog: bool,
}

impl FocusBlocker {
    pub fn describe(&self) -> String {
        let how = if self.focus_is_dialog {
            "holds the keyboard focus"
        } else {
            "owns the application that holds the keyboard focus"
        };
        let pid = self
            .pid
            .map(|p| p.to_string())
            .unwrap_or_else(|| "unknown".to_owned());
        format!(
            "modal dialog \"{}\" (window_id {}, pid {pid}) {how} and is not the target window; \
             input aimed at the target will not reach it",
            self.title, self.window_id
        )
    }

    pub fn to_json(&self) -> Value {
        json!({
            "window_id": self.window_id,
            "title": self.title,
            "pid": self.pid,
            "modal": true,
            "focus_is_dialog": self.focus_is_dialog,
            "describe": self.describe(),
        })
    }
}

/// The focused window's XID, walking up from a client that advertises none.
fn focused_window() -> Option<u64> {
    use std::os::raw::c_uint;
    let display = unsafe { x11::xlib::XOpenDisplay(std::ptr::null()) };
    if display.is_null() {
        return None;
    }
    let mut focus: x11::xlib::Window = 0;
    let mut revert: std::os::raw::c_int = 0;
    unsafe { x11::xlib::XGetInputFocus(display, &mut focus, &mut revert) };
    let root = unsafe { x11::xlib::XDefaultRootWindow(display) };
    let mut current = focus;
    // An override-redirect or utility child can hold the focus without a
    // _NET_WM_PID; climb to the nearest ancestor that has one.
    for _ in 0..8 {
        if current == 0
            || current == root
            || current == x11::xlib::PointerRoot as x11::xlib::Window
        {
            break;
        }
        if crate::x11::window_pid(current as u64).is_some() {
            unsafe { x11::xlib::XCloseDisplay(display) };
            return Some(current as u64);
        }
        let mut qroot: x11::xlib::Window = 0;
        let mut parent: x11::xlib::Window = 0;
        let mut children: *mut x11::xlib::Window = std::ptr::null_mut();
        let mut n: c_uint = 0;
        let rc = unsafe {
            x11::xlib::XQueryTree(
                display,
                current,
                &mut qroot,
                &mut parent,
                &mut children,
                &mut n,
            )
        };
        if !children.is_null() {
            unsafe { x11::xlib::XFree(children as *mut _) };
        }
        if rc == 0 || parent == 0 || parent == current {
            break;
        }
        current = parent;
    }
    unsafe { x11::xlib::XCloseDisplay(display) };
    if focus != 0 && focus != root && focus != x11::xlib::PointerRoot as x11::xlib::Window {
        Some(focus as u64)
    } else {
        None
    }
}

/// The modal dialog standing between the agent and `target_pid`, if any.
///
/// `None` on a Wayland session, where focus authority belongs to the
/// compositor and this X11 query would be meaningless.
pub fn blocking_modal_dialog(target_pid: u32) -> Option<FocusBlocker> {
    if crate::wayland::is_wayland() {
        return None;
    }
    let window = focused_window()?;
    let pid = crate::x11::window_pid(window);
    if pid == Some(target_pid) {
        return None;
    }
    if !crate::x11::window_is_modal(window) {
        return None;
    }
    let info = crate::x11::window_info(window)?;
    if info.width == 0 || info.height == 0 {
        return None;
    }
    Some(FocusBlocker {
        window_id: window,
        title: info.title,
        pid,
        focus_is_dialog: true,
    })
}
