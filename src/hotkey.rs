use cocoa::appkit::NSEventModifierFlags;
use cocoa::base::{id, nil};
use cocoa::foundation::NSString;
use objc::runtime::Object;
use objc::{class, msg_send, sel, sel_impl};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

const KEY_CODE_E: u16 = 14;
const NS_KEY_DOWN_MASK: u64 = 1 << 10;

// NSWindowAnimationBehavior values
const NS_WINDOW_ANIMATION_BEHAVIOR_NONE: i64 = 2;

// Notification name for app deactivation
const NS_APPLICATION_DID_RESIGN_ACTIVE_NOTIFICATION: &str = "NSApplicationDidResignActiveNotification";

// NSStatusBar thickness (for menu bar)
const NS_VARIABLE_STATUS_ITEM_LENGTH: f64 = -1.0;

/// Registers both global and local NSEvent monitors for Cmd+Shift+E.
/// Also disables window animation and creates a status bar item.
///
/// # Safety
/// `ns_window` must be a valid NSWindow/NSPanel pointer that outlives the monitors.
pub unsafe fn register_hotkey(ns_window: *mut Object) {
    let visible = Arc::new(AtomicBool::new(false));

    // Disable window animation for instant show/hide
    // SAFETY: ns_window is a valid NSWindow pointer per the function's safety contract
    let _: () = unsafe { msg_send![ns_window, setAnimationBehavior: NS_WINDOW_ANIMATION_BEHAVIOR_NONE] };

    // Create status bar item (menu bar icon)
    // SAFETY: ns_window is valid, and create_status_item's requirements are met
    unsafe { create_status_item(ns_window, visible.clone()) };

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
        // SAFETY: NSEvent class exists and the handler block is valid
        let _: id = unsafe {
            msg_send![
                class!(NSEvent),
                addGlobalMonitorForEventsMatchingMask: NS_KEY_DOWN_MASK
                handler: &*handler
            ]
        };
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
        // SAFETY: NSEvent class exists and the handler block is valid
        let _: id = unsafe {
            msg_send![
                class!(NSEvent),
                addLocalMonitorForEventsMatchingMask: NS_KEY_DOWN_MASK
                handler: &*handler
            ]
        };
        std::mem::forget(handler);
    }

    // Register for app deactivation to auto-hide window
    // SAFETY: ns_window is valid, visible Arc is cloned
    unsafe { register_deactivation_observer(ns_window, visible) };
}

/// Registers an observer for NSApplicationDidResignActiveNotification.
/// When the app loses focus, the window is automatically hidden.
///
/// # Safety
/// `ns_window` must be a valid NSWindow pointer that outlives the observer.
unsafe fn register_deactivation_observer(ns_window: *mut Object, visible: Arc<AtomicBool>) {
    let ns_window = ns_window as usize; // make it Send

    let handler = block::ConcreteBlock::new(move |_notification: id| {
        // When app loses focus, hide the window
        if visible.load(Ordering::SeqCst) {
            unsafe {
                let ns_window = ns_window as *mut Object;
                let _: () = msg_send![ns_window, orderOut: nil];
            }
            visible.store(false, Ordering::SeqCst);
        }
    });
    let handler = handler.copy();

    // Get the default notification center
    // SAFETY: NSNotificationCenter class exists on macOS
    let notification_center: id =
        unsafe { msg_send![class!(NSNotificationCenter), defaultCenter] };

    // Create the notification name string
    // SAFETY: NSString::alloc and init_str are safe
    let notification_name =
        unsafe { NSString::alloc(nil).init_str(NS_APPLICATION_DID_RESIGN_ACTIVE_NOTIFICATION) };

    // Register the observer
    // SAFETY: notification_center is valid, handler block is valid
    let _: id = unsafe {
        msg_send![
            notification_center,
            addObserverForName: notification_name
            object: nil
            queue: nil
            usingBlock: &*handler
        ]
    };

    std::mem::forget(handler);
}

unsafe fn create_status_item(ns_window: *mut Object, visible: Arc<AtomicBool>) {
    // Get the system status bar
    // SAFETY: NSStatusBar class exists on macOS
    let status_bar: id = unsafe { msg_send![class!(NSStatusBar), systemStatusBar] };

    // Create a status item with variable length
    // SAFETY: status_bar is a valid NSStatusBar instance
    let status_item: id =
        unsafe { msg_send![status_bar, statusItemWithLength: NS_VARIABLE_STATUS_ITEM_LENGTH] };

    // Get the button from the status item
    // SAFETY: status_item is a valid NSStatusItem instance
    let button: id = unsafe { msg_send![status_item, button] };

    // Set the title to a simple "Z" character (or could use an SF Symbol)
    // SAFETY: NSString::alloc and init_str are safe with valid nil argument
    let title = unsafe { NSString::alloc(nil).init_str("Z") };
    // SAFETY: button is a valid NSStatusBarButton instance
    let _: () = unsafe { msg_send![button, setTitle: title] };

    // Store the status item to prevent it from being released
    // We'll use statics to keep references alive
    let ns_window = ns_window as usize;
    GLOBAL_STATUS_ITEM.store(status_item as usize, Ordering::SeqCst);
    GLOBAL_WINDOW.store(ns_window, Ordering::SeqCst);
    GLOBAL_VISIBLE.store(Box::into_raw(Box::new(visible)) as usize, Ordering::SeqCst);

    // Set up click handling via NSButton's action
    // We need to create an Objective-C object to receive the action
    // SAFETY: button is a valid NSStatusBarButton instance
    unsafe { setup_status_button_action(button) };
}

// Global state for the status item callback
static GLOBAL_STATUS_ITEM: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static GLOBAL_WINDOW: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static GLOBAL_VISIBLE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

unsafe fn setup_status_button_action(button: id) {
    use objc::declare::ClassDecl;
    use objc::runtime::{Class, Sel};

    // Check if we already registered the class
    let class_name = "ZeditorStatusTarget";
    let existing = Class::get(class_name);

    let target_class = if let Some(cls) = existing {
        cls
    } else {
        // Create a new Objective-C class to handle the click
        let superclass = Class::get("NSObject").unwrap();
        let mut decl = ClassDecl::new(class_name, superclass).unwrap();

        extern "C" fn handle_click(_self: &Object, _cmd: Sel, _sender: id) {
            unsafe {
                let ns_window = GLOBAL_WINDOW.load(Ordering::SeqCst) as *mut Object;
                let visible_ptr = GLOBAL_VISIBLE.load(Ordering::SeqCst) as *mut Arc<AtomicBool>;
                if !visible_ptr.is_null() {
                    toggle_window(ns_window, &*visible_ptr);
                }
            }
        }

        // SAFETY: Adding a method to a class being declared, with valid selector and fn pointer
        unsafe {
            decl.add_method(
                sel!(statusItemClicked:),
                handle_click as extern "C" fn(&Object, Sel, id),
            );
        }

        decl.register()
    };

    // Create an instance of our target class
    // SAFETY: target_class is a valid registered ObjC class
    let target: id = unsafe { msg_send![target_class, new] };

    // Set the button's target and action
    // SAFETY: button is a valid NSStatusBarButton, target is a valid ObjC object
    let _: () = unsafe { msg_send![button, setTarget: target] };
    let _: () = unsafe { msg_send![button, setAction: sel!(statusItemClicked:)] };

    // Note: target is a raw pointer (Copy type), so we don't need mem::forget.
    // The ObjC runtime retains it via setTarget:.
}

unsafe fn is_cmd_shift_e(event: id) -> bool {
    let key_code: u16 = msg_send![event, keyCode];
    let modifier_flags: u64 = msg_send![event, modifierFlags];
    let cmd = modifier_flags & NSEventModifierFlags::NSCommandKeyMask.bits() != 0;
    let shift = modifier_flags & NSEventModifierFlags::NSShiftKeyMask.bits() != 0;
    key_code == KEY_CODE_E && cmd && shift
}

pub unsafe fn toggle_window(ns_window: *mut Object, visible: &AtomicBool) {
    if visible.load(Ordering::SeqCst) {
        let _: () = msg_send![ns_window, orderOut: nil];
        visible.store(false, Ordering::SeqCst);
    } else {
        // Activate the application so it can receive keyboard focus
        // SAFETY: NSApplication class exists on macOS
        let ns_app: id = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![ns_app, activateIgnoringOtherApps: true];

        // Center on the screen with the mouse cursor
        let _: () = msg_send![ns_window, center];
        let _: () = msg_send![ns_window, makeKeyAndOrderFront: nil];
        visible.store(true, Ordering::SeqCst);
    }
}
