use cocoa::appkit::NSEventModifierFlags;
use cocoa::base::{id, nil};
use objc::runtime::Object;
use objc::{class, msg_send, sel, sel_impl};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const KEY_CODE_E: u16 = 14;
const NS_KEY_DOWN_MASK: u64 = 1 << 10;

/// Registers both global and local NSEvent monitors for Cmd+Shift+E.
/// The callback toggles the NSWindow visibility.
///
/// # Safety
/// `ns_window` must be a valid NSWindow/NSPanel pointer that outlives the monitors.
pub unsafe fn register_hotkey(ns_window: *mut Object) {
    let visible = Arc::new(AtomicBool::new(false));

    // Global monitor — fires when another app is focused
    {
        let ns_window = ns_window as usize; // make it Send
        let visible = visible.clone();
        let handler = block::ConcreteBlock::new(move |event: id| {
            unsafe {
                if is_cmd_shift_e(event) {
                    toggle_window(ns_window as *mut Object, &visible);
                }
            }
        });
        let handler = handler.copy();
        let _: id = msg_send![
            class!(NSEvent),
            addGlobalMonitorForEventsMatchingMask: NS_KEY_DOWN_MASK
            handler: &*handler
        ];
        std::mem::forget(handler);
    }

    // Local monitor — fires when our own panel is focused
    {
        let ns_window = ns_window as usize;
        let visible = visible.clone();
        let handler = block::ConcreteBlock::new(move |event: id| -> id {
            unsafe {
                if is_cmd_shift_e(event) {
                    toggle_window(ns_window as *mut Object, &visible);
                    nil // swallow the event
                } else {
                    event
                }
            }
        });
        let handler = handler.copy();
        let _: id = msg_send![
            class!(NSEvent),
            addLocalMonitorForEventsMatchingMask: NS_KEY_DOWN_MASK
            handler: &*handler
        ];
        std::mem::forget(handler);
    }
}

unsafe fn is_cmd_shift_e(event: id) -> bool {
    let key_code: u16 = msg_send![event, keyCode];
    let modifier_flags: u64 = msg_send![event, modifierFlags];
    let cmd = modifier_flags & NSEventModifierFlags::NSCommandKeyMask.bits() as u64 != 0;
    let shift = modifier_flags & NSEventModifierFlags::NSShiftKeyMask.bits() as u64 != 0;
    key_code == KEY_CODE_E && cmd && shift
}

unsafe fn toggle_window(ns_window: *mut Object, visible: &AtomicBool) {
    if visible.load(Ordering::SeqCst) {
        let _: () = msg_send![ns_window, orderOut: nil];
        visible.store(false, Ordering::SeqCst);
    } else {
        // Center on the screen with the mouse cursor
        let _: () = msg_send![ns_window, center];
        let _: () = msg_send![ns_window, makeKeyAndOrderFront: nil];
        visible.store(true, Ordering::SeqCst);
    }
}
