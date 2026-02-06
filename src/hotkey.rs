use cocoa::base::{id, nil};
use cocoa::foundation::NSString;
use objc::runtime::Object;
use objc::{class, msg_send, sel, sel_impl};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

// Carbon Event constants
const K_VK_ANSI_E: u32 = 0x0E; // Virtual key code for 'E'
const CMD_KEY: u32 = 1 << 8; // cmdKey modifier
const SHIFT_KEY: u32 = 1 << 9; // shiftKey modifier
const K_EVENT_CLASS_KEYBOARD: u32 = 0x6B657962; // 'keyb'
const K_EVENT_HOT_KEY_PRESSED: u32 = 5;
const K_EVENT_PARAM_DIRECT_OBJECT: u32 = 0x2D2D2D2D; // '----'
const TYPE_EVENT_HOT_KEY_ID: u32 = 0x686B6964; // 'hkid'

// NSWindowAnimationBehavior values
const NS_WINDOW_ANIMATION_BEHAVIOR_NONE: i64 = 2;

// Notification name for app deactivation
const NS_APPLICATION_DID_RESIGN_ACTIVE_NOTIFICATION: &str = "NSApplicationDidResignActiveNotification";

// NSStatusBar thickness (for menu bar)
const NS_VARIABLE_STATUS_ITEM_LENGTH: f64 = -1.0;

// Carbon Event types
#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct EventHotKeyID {
    signature: u32,
    id: u32,
}

#[repr(C)]
struct EventTypeSpec {
    event_class: u32,
    event_kind: u32,
}

type EventHandlerRef = *mut c_void;
type EventHotKeyRef = *mut c_void;
type EventTargetRef = *mut c_void;
type EventRef = *mut c_void;
type OSStatus = i32;

type EventHandlerProcPtr = extern "C" fn(
    handler: EventHandlerRef,
    event: EventRef,
    user_data: *mut c_void,
) -> OSStatus;

// Carbon Event Manager FFI
#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn GetEventDispatcherTarget() -> EventTargetRef;
    fn RegisterEventHotKey(
        in_hot_key_code: u32,
        in_hot_key_modifiers: u32,
        in_hot_key_id: EventHotKeyID,
        in_target: EventTargetRef,
        in_options: u32,
        out_ref: *mut EventHotKeyRef,
    ) -> OSStatus;
    fn InstallEventHandler(
        in_target: EventTargetRef,
        in_handler: EventHandlerProcPtr,
        in_num_types: u32,
        in_list: *const EventTypeSpec,
        in_user_data: *mut c_void,
        out_ref: *mut EventHandlerRef,
    ) -> OSStatus;
    fn GetEventParameter(
        in_event: EventRef,
        in_name: u32,
        in_desired_type: u32,
        out_actual_type: *mut u32,
        in_buffer_size: u32,
        out_actual_size: *mut u32,
        out_data: *mut c_void,
    ) -> OSStatus;
}

/// Registers a global hotkey using Carbon Events (Cmd+Shift+E).
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

    // Register Carbon global hotkey (Cmd+Shift+E)
    // SAFETY: ns_window is valid, visible Arc is cloned
    unsafe { register_carbon_hotkey(ns_window, visible.clone()) };

    // Register for app deactivation to auto-hide window
    // SAFETY: ns_window is valid, visible Arc is cloned
    unsafe { register_deactivation_observer(ns_window, visible) };
}

/// Registers a global hotkey using Carbon Event Manager.
/// This is more reliable than NSEvent monitors for background apps.
///
/// # Safety
/// `ns_window` must be a valid NSWindow pointer that outlives the hotkey.
unsafe fn register_carbon_hotkey(ns_window: *mut Object, visible: Arc<AtomicBool>) {
    // Store in globals for the callback
    GLOBAL_WINDOW.store(ns_window as usize, Ordering::SeqCst);
    GLOBAL_VISIBLE.store(Box::into_raw(Box::new(visible)) as usize, Ordering::SeqCst);

    // Define the hotkey ID
    let hotkey_id = EventHotKeyID {
        signature: 0x5A454449, // 'ZEDI' - unique signature for our app
        id: 1,
    };

    // Get the event dispatcher target
    // SAFETY: Carbon API call, returns valid target
    let event_target = unsafe { GetEventDispatcherTarget() };

    // Register the hotkey: Cmd+Shift+E
    let mut hotkey_ref: EventHotKeyRef = std::ptr::null_mut();
    // SAFETY: Carbon API call with valid parameters
    let status = unsafe {
        RegisterEventHotKey(
            K_VK_ANSI_E,
            CMD_KEY | SHIFT_KEY,
            hotkey_id,
            event_target,
            0,
            &mut hotkey_ref,
        )
    };

    if status != 0 {
        eprintln!("RegisterEventHotKey failed with status: {}", status);
        return;
    }

    // Install the event handler
    let event_type = EventTypeSpec {
        event_class: K_EVENT_CLASS_KEYBOARD,
        event_kind: K_EVENT_HOT_KEY_PRESSED,
    };

    let mut handler_ref: EventHandlerRef = std::ptr::null_mut();
    // SAFETY: Carbon API call with valid parameters
    let status = unsafe {
        InstallEventHandler(
            event_target,
            hotkey_handler,
            1,
            &event_type,
            std::ptr::null_mut(),
            &mut handler_ref,
        )
    };

    if status != 0 {
        eprintln!("InstallEventHandler failed with status: {}", status);
    }
}

/// Carbon event handler callback for hotkey presses.
extern "C" fn hotkey_handler(
    _handler: EventHandlerRef,
    event: EventRef,
    _user_data: *mut c_void,
) -> OSStatus {
    unsafe {
        // Get the hotkey ID from the event
        let mut hotkey_id = EventHotKeyID { signature: 0, id: 0 };
        let status = GetEventParameter(
            event,
            K_EVENT_PARAM_DIRECT_OBJECT,
            TYPE_EVENT_HOT_KEY_ID,
            std::ptr::null_mut(),
            std::mem::size_of::<EventHotKeyID>() as u32,
            std::ptr::null_mut(),
            &mut hotkey_id as *mut EventHotKeyID as *mut c_void,
        );

        if status == 0 && hotkey_id.id == 1 {
            // Our hotkey was pressed - toggle the window
            let ns_window = GLOBAL_WINDOW.load(Ordering::SeqCst) as *mut Object;
            let visible_ptr = GLOBAL_VISIBLE.load(Ordering::SeqCst) as *mut Arc<AtomicBool>;
            if !visible_ptr.is_null() && !ns_window.is_null() {
                toggle_window(ns_window, &*visible_ptr);
            }
        }
    }
    0 // noErr
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

// Global state for the status item callback and hotkey handler
static GLOBAL_STATUS_ITEM: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static GLOBAL_WINDOW: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
static GLOBAL_VISIBLE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
// Store the previously focused app to restore focus when hiding
static GLOBAL_PREVIOUS_APP: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

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

pub unsafe fn toggle_window(ns_window: *mut Object, visible: &AtomicBool) {
    if visible.load(Ordering::SeqCst) {
        // Hide the window
        let _: () = msg_send![ns_window, orderOut: nil];
        visible.store(false, Ordering::SeqCst);

        // Restore focus to the previous app
        let prev_app = GLOBAL_PREVIOUS_APP.swap(0, Ordering::SeqCst) as id;
        if !prev_app.is_null() {
            // NSApplicationActivateIgnoringOtherApps = 1 << 1 = 2
            let _: bool = msg_send![prev_app, activateWithOptions: 2u64];
            // Release the retained app reference
            let _: () = msg_send![prev_app, release];
        }
    } else {
        // Capture the currently focused app before we steal focus
        // SAFETY: NSWorkspace class exists on macOS
        let workspace: id = msg_send![class!(NSWorkspace), sharedWorkspace];
        let frontmost_app: id = msg_send![workspace, frontmostApplication];
        if !frontmost_app.is_null() {
            // Retain it so it doesn't get deallocated
            let _: id = msg_send![frontmost_app, retain];
            // Store the old value and release it if there was one
            let old = GLOBAL_PREVIOUS_APP.swap(frontmost_app as usize, Ordering::SeqCst) as id;
            if !old.is_null() {
                let _: () = msg_send![old, release];
            }
        }

        // Activate the application so it can receive keyboard focus
        // SAFETY: NSApplication class exists on macOS
        let ns_app: id = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![ns_app, activateIgnoringOtherApps: true];

        // Center on the screen with the mouse cursor
        let _: () = msg_send![ns_window, center];
        let _: () = msg_send![ns_window, makeKeyAndOrderFront: nil];

        // Force to front regardless of window level
        let _: () = msg_send![ns_window, orderFrontRegardless];

        visible.store(true, Ordering::SeqCst);
    }
}
