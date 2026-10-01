// The session event tap runs before Alfred's Carbon shortcut. Only consume
// Cmd+Shift+V while the popup is the active key window, never in preferences.
#![allow(unsafe_op_in_unsafe_fn)]

use std::{
    ffi::c_void,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

static TAP: AtomicUsize = AtomicUsize::new(0);
static CONSUMED_DOWN: AtomicBool = AtomicBool::new(false);

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn CGEventTapCreate(
        location: u32,
        placement: u32,
        options: u32,
        mask: u64,
        callback: extern "C" fn(*mut c_void, u32, *mut c_void, *mut c_void) -> *mut c_void,
        user_info: *mut c_void,
    ) -> *mut c_void;
    fn CGEventTapEnable(tap: *mut c_void, enable: bool);
    fn CGEventGetFlags(event: *mut c_void) -> u64;
    fn CGEventGetIntegerValueField(event: *mut c_void, field: u32) -> i64;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFMachPortCreateRunLoopSource(
        allocator: *const c_void,
        port: *mut c_void,
        order: isize,
    ) -> *mut c_void;
    fn CFRunLoopGetMain() -> *mut c_void;
    fn CFRunLoopAddSource(run_loop: *mut c_void, source: *mut c_void, mode: *const c_void);
    fn CFRelease(value: *mut c_void);
    static kCFRunLoopCommonModes: *const c_void;
}

fn matches_shortcut(key: i64, flags: u64) -> bool {
    const MODIFIERS: u64 = (1 << 17) | (1 << 18) | (1 << 19) | (1 << 20);
    key == 9 && flags & MODIFIERS == (1 << 17) | (1 << 20)
}

extern "C" fn callback(
    _: *mut c_void,
    kind: u32,
    event: *mut c_void,
    _: *mut c_void,
) -> *mut c_void {
    unsafe {
        if kind == 0xFFFF_FFFE || kind == 0xFFFF_FFFF {
            let tap = TAP.load(Ordering::SeqCst) as *mut c_void;
            if !tap.is_null() {
                CGEventTapEnable(tap, true);
            }
            CONSUMED_DOWN.store(false, Ordering::SeqCst);
            return event;
        }
        let key = CGEventGetIntegerValueField(event, 9);
        if kind == 11 && key == 9 && CONSUMED_DOWN.swap(false, Ordering::SeqCst) {
            return std::ptr::null_mut();
        }
        if kind == 10
            && matches_shortcut(key, CGEventGetFlags(event))
            && crate::hotkey::popup_is_active()
        {
            // Ignore auto-repeat until the physical V key is released.
            if !CONSUMED_DOWN.swap(true, Ordering::SeqCst) {
                crate::hotkey::request_clipboard_history();
            }
            return std::ptr::null_mut();
        }
    }
    event
}

/// Called on the main thread after the popup has been registered.
pub unsafe fn install() {
    if TAP.load(Ordering::SeqCst) != 0 {
        return;
    }
    let tap = CGEventTapCreate(
        1,
        0,
        0,
        (1 << 10) | (1 << 11),
        callback,
        std::ptr::null_mut(),
    );
    if tap.is_null() {
        crate::logging::event(
            "history.hotkey",
            "event_tap_unavailable; Accessibility access required",
        );
        return;
    }
    let source = CFMachPortCreateRunLoopSource(std::ptr::null(), tap, 0);
    if source.is_null() {
        CFRelease(tap);
        return;
    }
    TAP.store(tap as usize, Ordering::SeqCst);
    CFRunLoopAddSource(CFRunLoopGetMain(), source, kCFRunLoopCommonModes);
    CFRelease(source);
    CGEventTapEnable(tap, true);
    crate::logging::event("history.hotkey", "event_tap_installed");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_only_command_shift_v() {
        let combo = (1 << 17) | (1 << 20);
        assert!(matches_shortcut(9, combo));
        assert!(matches_shortcut(9, combo | (1 << 16)));
        assert!(!matches_shortcut(9, 1 << 20));
        assert!(!matches_shortcut(9, combo | (1 << 19)));
        assert!(!matches_shortcut(9, combo | (1 << 18)));
        assert!(!matches_shortcut(13, combo));
    }
}
