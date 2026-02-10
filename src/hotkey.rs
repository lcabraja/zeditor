// Allow unsafe operations in unsafe fns - this is an FFI-heavy module
#![allow(unsafe_op_in_unsafe_fn)]

use cocoa::base::{id, nil};
use cocoa::foundation::NSString;
use objc::runtime::Object;
use objc::{class, msg_send, sel, sel_impl};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

// Carbon Event constants
const K_VK_ESCAPE: u16 = 0x35; // Virtual key code for Escape
const K_EVENT_CLASS_KEYBOARD: u32 = 0x6B657962; // 'keyb'
const K_EVENT_HOT_KEY_PRESSED: u32 = 5;
const K_EVENT_PARAM_DIRECT_OBJECT: u32 = 0x2D2D2D2D; // '----'
const TYPE_EVENT_HOT_KEY_ID: u32 = 0x686B6964; // 'hkid'
const NS_KEY_DOWN_MASK: u64 = 1 << 10; // NSEventMaskKeyDown

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
    fn UnregisterEventHotKey(in_ref: EventHotKeyRef) -> OSStatus;
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

// Accessibility API
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: id) -> bool;
}

// Global state
static GLOBAL_STATUS_ITEM: AtomicUsize = AtomicUsize::new(0);
static GLOBAL_WINDOW: AtomicUsize = AtomicUsize::new(0);
static GLOBAL_VISIBLE: AtomicUsize = AtomicUsize::new(0);
static GLOBAL_PREVIOUS_APP: AtomicUsize = AtomicUsize::new(0);
static GLOBAL_HOTKEY_REF: AtomicUsize = AtomicUsize::new(0);
static GLOBAL_MENU: AtomicUsize = AtomicUsize::new(0);
static HANDLER_INSTALLED: AtomicBool = AtomicBool::new(false);
static OPEN_PREFS_REQUESTED: AtomicBool = AtomicBool::new(false);
static SHOW_REQUESTED: AtomicBool = AtomicBool::new(false);

static GLOBAL_ERROR: Mutex<Option<String>> = Mutex::new(None);
static PENDING_CLIPBOARD: Mutex<Option<String>> = Mutex::new(None);

/// Check if the preferences window was requested from the menu.
/// Atomically swaps the flag and returns the old value.
pub fn is_prefs_requested() -> bool {
    OPEN_PREFS_REQUESTED.swap(false, Ordering::SeqCst)
}

/// Get the current error message, if any.
pub fn get_error() -> Option<String> {
    GLOBAL_ERROR.lock().ok().and_then(|g| g.clone())
}

fn set_error(err: Option<String>) {
    if let Ok(mut g) = GLOBAL_ERROR.lock() {
        *g = err;
    }
    unsafe { update_menu_error() };
}

/// Take the pre-fetched clipboard text (if any). Returns None if no text was pre-fetched.
/// This is used by the editor to avoid the slow GPUI clipboard read.
pub fn take_pending_clipboard() -> Option<String> {
    PENDING_CLIPBOARD.lock().ok().and_then(|mut g| g.take())
}

/// Check if a show-window was requested (hotkey pressed while hidden).
/// Atomically swaps the flag and returns the old value.
pub fn is_show_requested() -> bool {
    SHOW_REQUESTED.swap(false, Ordering::SeqCst)
}

/// Set initial text and request the window to show.
/// Used for CLI argument text.
pub fn set_initial_text(text: String) {
    if let Ok(mut pending) = PENDING_CLIPBOARD.lock() {
        *pending = Some(text);
    }
    SHOW_REQUESTED.store(true, Ordering::SeqCst);
}

/// Actually show the window. Called from the GPUI side after the editor text has been set.
///
/// # Safety
/// Must be called from the main thread.
pub unsafe fn show_window_now() {
    let ns_window = GLOBAL_WINDOW.load(Ordering::SeqCst) as *mut Object;
    let visible_ptr = GLOBAL_VISIBLE.load(Ordering::SeqCst) as *mut Arc<AtomicBool>;
    if ns_window.is_null() || visible_ptr.is_null() {
        return;
    }

    let ns_app: id = msg_send![class!(NSApplication), sharedApplication];
    let _: () = msg_send![ns_app, activateIgnoringOtherApps: true];

    let _: () = msg_send![ns_window, center];
    let _: () = msg_send![ns_window, makeKeyAndOrderFront: nil];
    let _: () = msg_send![ns_window, orderFrontRegardless];

    (*visible_ptr).store(true, Ordering::SeqCst);
}

fn version_string() -> String {
    let version = env!("CARGO_PKG_VERSION");
    if version == "0.1.0" {
        format!(
            "Zeditor dev ({}, {})",
            env!("GIT_COMMIT"),
            env!("BUILD_DATE")
        )
    } else {
        format!("Zeditor v{}", version)
    }
}

/// Registers a global hotkey using Carbon Events.
/// Also disables window animation and creates a status bar item with menu.
///
/// # Safety
/// `ns_window` must be a valid NSWindow/NSPanel pointer that outlives the monitors.
pub unsafe fn register_hotkey(ns_window: *mut Object, key_code: u32, modifiers: u32) {
    // Check if we have accessibility permissions, prompt if not
    let trusted = AXIsProcessTrusted();
    if !trusted {
        let key: id = NSString::alloc(nil).init_str("AXTrustedCheckOptionPrompt");
        let yes_num: id = msg_send![class!(NSNumber), numberWithBool: true];
        let options: id =
            msg_send![class!(NSDictionary), dictionaryWithObject: yes_num forKey: key];
        let _ = AXIsProcessTrustedWithOptions(options);
    }

    let visible = Arc::new(AtomicBool::new(false));

    // Disable window animation for instant show/hide
    let _: () = msg_send![ns_window, setAnimationBehavior: NS_WINDOW_ANIMATION_BEHAVIOR_NONE];

    // Create status bar item with menu
    create_status_item(ns_window, visible.clone());

    // Register Carbon global hotkey
    register_carbon_hotkey(ns_window, visible.clone(), key_code, modifiers);

    // Register local ESC key monitor to hide window
    register_escape_monitor(ns_window, visible.clone());

    // Register for app deactivation to auto-hide window
    register_deactivation_observer(ns_window, visible);
}

/// Re-registers the global hotkey with new key code and modifiers.
/// Call this after the user changes the hotkey in preferences.
///
/// # Safety
/// Must be called from the main thread after `register_hotkey` has been called.
pub unsafe fn re_register_hotkey(key_code: u32, modifiers: u32) {
    // Unregister old hotkey
    let old_ref = GLOBAL_HOTKEY_REF.swap(0, Ordering::SeqCst) as EventHotKeyRef;
    if !old_ref.is_null() {
        UnregisterEventHotKey(old_ref);
    }

    // Register new hotkey
    let hotkey_id = EventHotKeyID {
        signature: 0x5A454449, // 'ZEDI'
        id: 1,
    };
    let event_target = GetEventDispatcherTarget();
    let mut hotkey_ref: EventHotKeyRef = std::ptr::null_mut();
    let status = RegisterEventHotKey(
        key_code,
        modifiers,
        hotkey_id,
        event_target,
        0,
        &mut hotkey_ref,
    );

    if status != 0 {
        set_error(Some(format!(
            "Hotkey registration failed (status: {})",
            status
        )));
    } else {
        GLOBAL_HOTKEY_REF.store(hotkey_ref as usize, Ordering::SeqCst);
        set_error(None);
    }
}

unsafe fn register_carbon_hotkey(
    ns_window: *mut Object,
    visible: Arc<AtomicBool>,
    key_code: u32,
    modifiers: u32,
) {
    // Store in globals for the callback
    GLOBAL_WINDOW.store(ns_window as usize, Ordering::SeqCst);
    GLOBAL_VISIBLE.store(Box::into_raw(Box::new(visible)) as usize, Ordering::SeqCst);

    let hotkey_id = EventHotKeyID {
        signature: 0x5A454449, // 'ZEDI'
        id: 1,
    };

    let event_target = GetEventDispatcherTarget();

    // Register the hotkey
    let mut hotkey_ref: EventHotKeyRef = std::ptr::null_mut();
    let status = RegisterEventHotKey(
        key_code,
        modifiers,
        hotkey_id,
        event_target,
        0,
        &mut hotkey_ref,
    );

    if status != 0 {
        set_error(Some(format!(
            "Hotkey registration failed (status: {})",
            status
        )));
    } else {
        GLOBAL_HOTKEY_REF.store(hotkey_ref as usize, Ordering::SeqCst);
    }

    // Install the event handler (only once)
    if !HANDLER_INSTALLED.swap(true, Ordering::SeqCst) {
        let event_type = EventTypeSpec {
            event_class: K_EVENT_CLASS_KEYBOARD,
            event_kind: K_EVENT_HOT_KEY_PRESSED,
        };

        let mut handler_ref: EventHandlerRef = std::ptr::null_mut();
        let status = InstallEventHandler(
            event_target,
            hotkey_handler,
            1,
            &event_type,
            std::ptr::null_mut(),
            &mut handler_ref,
        );

        if status != 0 {
            eprintln!("InstallEventHandler failed with status: {}", status);
        }
    }
}

unsafe fn register_escape_monitor(ns_window: *mut Object, visible: Arc<AtomicBool>) {
    let ns_window = ns_window as usize;

    let handler = block::ConcreteBlock::new(move |event: id| -> id {
        unsafe {
            let key_code: u16 = msg_send![event, keyCode];
            if key_code == K_VK_ESCAPE && visible.load(Ordering::SeqCst) {
                let ns_window = ns_window as *mut Object;
                let visible_ptr = GLOBAL_VISIBLE.load(Ordering::SeqCst) as *mut Arc<AtomicBool>;
                if !visible_ptr.is_null() {
                    hide_window(ns_window, &*visible_ptr);
                }
                return nil;
            }
            event
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

extern "C" fn hotkey_handler(
    _handler: EventHandlerRef,
    event: EventRef,
    _user_data: *mut c_void,
) -> OSStatus {
    unsafe {
        let mut hotkey_id = EventHotKeyID {
            signature: 0,
            id: 0,
        };
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
            let ns_window = GLOBAL_WINDOW.load(Ordering::SeqCst) as *mut Object;
            let visible_ptr = GLOBAL_VISIBLE.load(Ordering::SeqCst) as *mut Arc<AtomicBool>;
            if !visible_ptr.is_null() && !ns_window.is_null() {
                toggle_window(ns_window, &*visible_ptr);
            }
        }
    }
    0
}

unsafe fn register_deactivation_observer(ns_window: *mut Object, visible: Arc<AtomicBool>) {
    let ns_window = ns_window as usize;

    let handler = block::ConcreteBlock::new(move |_notification: id| {
        if visible.load(Ordering::SeqCst) {
            unsafe {
                let ns_window = ns_window as *mut Object;
                let _: () = msg_send![ns_window, orderOut: nil];
            }
            visible.store(false, Ordering::SeqCst);
        }
    });
    let handler = handler.copy();

    let notification_center: id = msg_send![class!(NSNotificationCenter), defaultCenter];
    let notification_name =
        NSString::alloc(nil).init_str(NS_APPLICATION_DID_RESIGN_ACTIVE_NOTIFICATION);

    let _: id = msg_send![
        notification_center,
        addObserverForName: notification_name
        object: nil
        queue: nil
        usingBlock: &*handler
    ];

    std::mem::forget(handler);
}

unsafe fn create_status_item(ns_window: *mut Object, visible: Arc<AtomicBool>) {
    let status_bar: id = msg_send![class!(NSStatusBar), systemStatusBar];
    let status_item: id =
        msg_send![status_bar, statusItemWithLength: NS_VARIABLE_STATUS_ITEM_LENGTH];

    let button: id = msg_send![status_item, button];
    let title = NSString::alloc(nil).init_str("Z");
    let _: () = msg_send![button, setTitle: title];

    // Retain the status item to prevent deallocation
    let _: id = msg_send![status_item, retain];

    let ns_window = ns_window as usize;
    GLOBAL_STATUS_ITEM.store(status_item as usize, Ordering::SeqCst);
    GLOBAL_WINDOW.store(ns_window, Ordering::SeqCst);
    GLOBAL_VISIBLE.store(Box::into_raw(Box::new(visible)) as usize, Ordering::SeqCst);

    // Set up the NSMenu
    setup_status_menu(status_item);

    // Ensure visible
    let _: () = msg_send![status_item, setVisible: true];
}

unsafe fn setup_status_menu(status_item: id) {
    use objc::declare::ClassDecl;
    use objc::runtime::{Class, Sel};

    // Create the menu
    let menu: id = msg_send![class!(NSMenu), alloc];
    let menu: id = msg_send![menu, initWithTitle: NSString::alloc(nil).init_str("")];

    // 1. Version item (disabled, gray)
    let version_str = version_string();
    let version_title = NSString::alloc(nil).init_str(&version_str);
    let version_item: id = msg_send![class!(NSMenuItem), alloc];
    let version_item: id = msg_send![
        version_item,
        initWithTitle: version_title
        action: std::ptr::null::<Sel>()
        keyEquivalent: NSString::alloc(nil).init_str("")
    ];
    let _: () = msg_send![version_item, setEnabled: false];
    let _: () = msg_send![version_item, setTag: 50i64];
    let _: () = msg_send![menu, addItem: version_item];

    // Separator
    let sep: id = msg_send![class!(NSMenuItem), separatorItem];
    let _: () = msg_send![menu, addItem: sep];

    // 2. Error item (hidden by default)
    let error_title = NSString::alloc(nil).init_str("");
    let error_item: id = msg_send![class!(NSMenuItem), alloc];
    let error_item: id = msg_send![
        error_item,
        initWithTitle: error_title
        action: std::ptr::null::<Sel>()
        keyEquivalent: NSString::alloc(nil).init_str("")
    ];
    let _: () = msg_send![error_item, setEnabled: false];
    let _: () = msg_send![error_item, setTag: 100i64];
    let _: () = msg_send![error_item, setHidden: true];
    let _: () = msg_send![menu, addItem: error_item];

    // Error separator (hidden by default)
    let error_sep: id = msg_send![class!(NSMenuItem), separatorItem];
    let _: () = msg_send![error_sep, setTag: 101i64];
    let _: () = msg_send![error_sep, setHidden: true];
    let _: () = msg_send![menu, addItem: error_sep];

    // 3. Toggle Editor
    let class_name = "ZeditorMenuTarget";
    let target_class = if let Some(cls) = Class::get(class_name) {
        cls
    } else {
        let superclass = Class::get("NSObject").unwrap();
        let mut decl = ClassDecl::new(class_name, superclass).unwrap();

        extern "C" fn menu_toggle(_self: &Object, _cmd: Sel, _sender: id) {
            unsafe {
                let ns_window = GLOBAL_WINDOW.load(Ordering::SeqCst) as *mut Object;
                let visible_ptr = GLOBAL_VISIBLE.load(Ordering::SeqCst) as *mut Arc<AtomicBool>;
                if !visible_ptr.is_null() {
                    toggle_window(ns_window, &*visible_ptr);
                }
            }
        }

        extern "C" fn menu_preferences(_self: &Object, _cmd: Sel, _sender: id) {
            OPEN_PREFS_REQUESTED.store(true, Ordering::SeqCst);
            unsafe {
                let ns_app: id = msg_send![class!(NSApplication), sharedApplication];
                let _: () = msg_send![ns_app, activateIgnoringOtherApps: true];
            }
        }

        extern "C" fn menu_quit(_self: &Object, _cmd: Sel, _sender: id) {
            unsafe {
                let ns_app: id = msg_send![class!(NSApplication), sharedApplication];
                let _: () = msg_send![ns_app, terminate: nil];
            }
        }

        decl.add_method(
            sel!(menuToggle:),
            menu_toggle as extern "C" fn(&Object, Sel, id),
        );
        decl.add_method(
            sel!(menuPreferences:),
            menu_preferences as extern "C" fn(&Object, Sel, id),
        );
        decl.add_method(
            sel!(menuQuit:),
            menu_quit as extern "C" fn(&Object, Sel, id),
        );

        decl.register()
    };

    let target: id = msg_send![target_class, new];

    let toggle_title = NSString::alloc(nil).init_str("Toggle Editor");
    let toggle_item: id = msg_send![class!(NSMenuItem), alloc];
    let toggle_item: id = msg_send![
        toggle_item,
        initWithTitle: toggle_title
        action: sel!(menuToggle:)
        keyEquivalent: NSString::alloc(nil).init_str("")
    ];
    let _: () = msg_send![toggle_item, setTarget: target];
    let _: () = msg_send![toggle_item, setTag: 200i64];
    let _: () = msg_send![menu, addItem: toggle_item];

    // Separator
    let sep2: id = msg_send![class!(NSMenuItem), separatorItem];
    let _: () = msg_send![menu, addItem: sep2];

    // 4. Preferences...
    let prefs_title = NSString::alloc(nil).init_str("Preferences...");
    let prefs_item: id = msg_send![class!(NSMenuItem), alloc];
    let prefs_item: id = msg_send![
        prefs_item,
        initWithTitle: prefs_title
        action: sel!(menuPreferences:)
        keyEquivalent: NSString::alloc(nil).init_str(",")
    ];
    let _: () = msg_send![prefs_item, setTarget: target];
    let _: () = msg_send![prefs_item, setTag: 300i64];
    let _: () = msg_send![menu, addItem: prefs_item];

    // Separator
    let sep3: id = msg_send![class!(NSMenuItem), separatorItem];
    let _: () = msg_send![menu, addItem: sep3];

    // 5. Quit Zeditor
    let quit_title = NSString::alloc(nil).init_str("Quit Zeditor");
    let quit_item: id = msg_send![class!(NSMenuItem), alloc];
    let quit_item: id = msg_send![
        quit_item,
        initWithTitle: quit_title
        action: sel!(menuQuit:)
        keyEquivalent: NSString::alloc(nil).init_str("q")
    ];
    let _: () = msg_send![quit_item, setTarget: target];
    let _: () = msg_send![quit_item, setTag: 400i64];
    let _: () = msg_send![menu, addItem: quit_item];

    // Store menu pointer for later updates (before attaching)
    GLOBAL_MENU.store(menu as usize, Ordering::SeqCst);

    // Attach menu to status item
    let _: () = msg_send![status_item, setMenu: menu];
}

unsafe fn update_menu_error() {
    let menu = GLOBAL_MENU.load(Ordering::SeqCst) as id;
    if menu.is_null() {
        return;
    }

    let error_item: id = msg_send![menu, itemWithTag: 100i64];
    let error_sep: id = msg_send![menu, itemWithTag: 101i64];

    if error_item.is_null() || error_sep.is_null() {
        return;
    }

    if let Some(err) = get_error() {
        let title = NSString::alloc(nil).init_str(&format!("⚠ {}", err));
        let _: () = msg_send![error_item, setTitle: title];
        let _: () = msg_send![error_item, setHidden: false];
        let _: () = msg_send![error_sep, setHidden: false];
    } else {
        let _: () = msg_send![error_item, setHidden: true];
        let _: () = msg_send![error_sep, setHidden: true];
    }
}

/// Hides the window and restores focus to the previous app.
///
/// # Safety
/// `ns_window` must be a valid NSWindow pointer.
pub unsafe fn hide_window(ns_window: *mut Object, visible: &AtomicBool) {
    if !visible.load(Ordering::SeqCst) {
        return;
    }

    let _: () = msg_send![ns_window, orderOut: nil];
    visible.store(false, Ordering::SeqCst);

    let prev_app = GLOBAL_PREVIOUS_APP.swap(0, Ordering::SeqCst) as id;
    if !prev_app.is_null() {
        let _: bool = msg_send![prev_app, activateWithOptions: 2u64];
        let _: () = msg_send![prev_app, release];
    }
}

pub unsafe fn toggle_window(ns_window: *mut Object, visible: &AtomicBool) {
    if visible.load(Ordering::SeqCst) {
        hide_window(ns_window, visible);
    } else {
        // Remember the previous frontmost app for focus restoration on hide
        let workspace: id = msg_send![class!(NSWorkspace), sharedWorkspace];
        let frontmost_app: id = msg_send![workspace, frontmostApplication];
        if !frontmost_app.is_null() {
            let _: id = msg_send![frontmost_app, retain];
            let old = GLOBAL_PREVIOUS_APP.swap(frontmost_app as usize, Ordering::SeqCst) as id;
            if !old.is_null() {
                let _: () = msg_send![old, release];
            }
        }

        // Signal the GPUI polling task to show the window
        SHOW_REQUESTED.store(true, Ordering::SeqCst);
    }
}

/// Submits text by copying to clipboard, hiding the window, restoring focus,
/// and simulating Cmd+V to paste into the previous app.
///
/// # Safety
/// Must be called from the main thread with a valid ns_window pointer.
pub unsafe fn submit_and_paste(text: &str) {
    let text = text.to_string();
    let result = std::panic::catch_unwind(move || unsafe { submit_and_paste_inner(&text) });
    if let Err(e) = result {
        eprintln!("[submit_and_paste] Panic: {:?}", e);
    }
}

// Store app to release after paste
static PENDING_RELEASE_APP: AtomicUsize = AtomicUsize::new(0);

unsafe fn submit_and_paste_inner(text: &str) {
    let pasteboard: id = msg_send![class!(NSPasteboard), generalPasteboard];
    let _: () = msg_send![pasteboard, clearContents];
    let ns_string: id = NSString::alloc(nil).init_str(text);
    let string_type: id = NSString::alloc(nil).init_str("public.utf8-plain-text");
    let _: bool = msg_send![pasteboard, setString: ns_string forType: string_type];

    let ns_window = GLOBAL_WINDOW.load(Ordering::SeqCst) as *mut Object;
    let visible_ptr = GLOBAL_VISIBLE.load(Ordering::SeqCst) as *mut Arc<AtomicBool>;
    let prev_app = GLOBAL_PREVIOUS_APP.swap(0, Ordering::SeqCst) as id;

    if !ns_window.is_null() && !visible_ptr.is_null() {
        let _: () = msg_send![ns_window, orderOut: nil];
        (*visible_ptr).store(false, Ordering::SeqCst);
    }

    if !prev_app.is_null() {
        let _: bool = msg_send![prev_app, activateWithOptions: 2u64];
        PENDING_RELEASE_APP.store(prev_app as usize, Ordering::SeqCst);
    }

    schedule_paste_with_delay();
}

unsafe fn schedule_paste_with_delay() {
    use objc::declare::ClassDecl;
    use objc::runtime::{Class, Sel};

    let class_name = "ZeditorPasteHelper";
    let helper_class = if let Some(cls) = Class::get(class_name) {
        cls
    } else {
        let Some(superclass) = Class::get("NSObject") else {
            eprintln!("Failed to get NSObject class");
            return;
        };
        let Some(mut decl) = ClassDecl::new(class_name, superclass) else {
            eprintln!("Failed to create class declaration");
            return;
        };

        extern "C" fn do_paste(_self: &Object, _cmd: Sel) {
            let result = std::panic::catch_unwind(|| unsafe {
                simulate_paste();

                let prev_app = PENDING_RELEASE_APP.swap(0, Ordering::SeqCst) as id;
                if !prev_app.is_null() {
                    let _: () = msg_send![prev_app, release];
                }
            });
            if let Err(e) = result {
                eprintln!("[do_paste] Panic: {:?}", e);
            }
        }

        decl.add_method(
            sel!(doPaste),
            do_paste as extern "C" fn(&Object, Sel),
        );

        decl.register()
    };

    let helper: id = msg_send![helper_class, new];
    let _: () = msg_send![
        helper,
        performSelector: sel!(doPaste)
        withObject: nil
        afterDelay: 0.05f64
    ];
}

unsafe fn simulate_paste() {
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventSourceCreate(state_id: i32) -> *mut c_void;
        fn CGEventCreateKeyboardEvent(
            source: *mut c_void,
            virtual_key: u16,
            key_down: bool,
        ) -> *mut c_void;
        fn CGEventSetFlags(event: *mut c_void, flags: u64);
        fn CGEventPost(tap: u32, event: *mut c_void);
        fn CFRelease(cf: *mut c_void);
    }

    const K_VK_ANSI_V: u16 = 0x09;
    const K_CG_EVENT_FLAG_MASK_COMMAND: u64 = 1 << 20;
    const K_CG_HID_EVENT_TAP: u32 = 0;
    const K_CG_EVENT_SOURCE_STATE_HID_SYSTEM_STATE: i32 = 1;

    let source = CGEventSourceCreate(K_CG_EVENT_SOURCE_STATE_HID_SYSTEM_STATE);
    if source.is_null() {
        return;
    }

    let key_down = CGEventCreateKeyboardEvent(source, K_VK_ANSI_V, true);
    if !key_down.is_null() {
        CGEventSetFlags(key_down, K_CG_EVENT_FLAG_MASK_COMMAND);
        CGEventPost(K_CG_HID_EVENT_TAP, key_down);
        CFRelease(key_down);
    }

    let key_up = CGEventCreateKeyboardEvent(source, K_VK_ANSI_V, false);
    if !key_up.is_null() {
        CGEventSetFlags(key_up, K_CG_EVENT_FLAG_MASK_COMMAND);
        CGEventPost(K_CG_HID_EVENT_TAP, key_up);
        CFRelease(key_up);
    }

    CFRelease(source);
}
