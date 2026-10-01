//! Native source-app icons and Alfred's reference-date timestamps.

#[cfg(target_os = "macos")]
pub fn app_icon(path: &str) -> Option<std::sync::Arc<gpui::Image>> {
    use cocoa::{
        base::{id, nil},
        foundation::{NSAutoreleasePool, NSString},
    };
    use objc::{class, msg_send, sel, sel_impl};
    unsafe {
        let pool = NSAutoreleasePool::new(nil);
        let workspace: id = msg_send![class!(NSWorkspace), sharedWorkspace];
        let exists = std::path::Path::new(path).is_dir();
        let name = NSString::alloc(nil).init_str(if exists { path } else { "app" });
        let icon: id = if exists {
            msg_send![workspace, iconForFile: name]
        } else {
            msg_send![workspace, iconForFileType: name]
        };
        let _: () = msg_send![name, release];
        let result = if icon.is_null() {
            None
        } else {
            let data: id = msg_send![icon, TIFFRepresentation];
            if data.is_null() {
                None
            } else {
                let length: usize = msg_send![data, length];
                let bytes: *const u8 = msg_send![data, bytes];
                if bytes.is_null() || length == 0 {
                    None
                } else {
                    Some(std::sync::Arc::new(gpui::Image::from_bytes(
                        gpui::ImageFormat::Tiff,
                        std::slice::from_raw_parts(bytes, length).to_vec(),
                    )))
                }
            }
        };
        let _: () = msg_send![pool, drain];
        result
    }
}

#[cfg(not(target_os = "macos"))]
pub fn app_icon(_: &str) -> Option<std::sync::Arc<gpui::Image>> {
    None
}

#[cfg(target_os = "macos")]
pub fn copied_at(timestamp: f64) -> String {
    use cocoa::{
        base::{id, nil},
        foundation::{NSAutoreleasePool, NSString},
    };
    use objc::{class, msg_send, sel, sel_impl};
    if !timestamp.is_finite() || timestamp <= 0. {
        return "Copy time unavailable".into();
    }
    unsafe {
        let pool = NSAutoreleasePool::new(nil);
        // Alfred stores seconds since 2001-01-01, not Unix timestamps.
        let date: id = msg_send![class!(NSDate), dateWithTimeIntervalSinceReferenceDate: timestamp];
        let calendar: id = msg_send![class!(NSCalendar), currentCalendar];
        let today: bool = msg_send![calendar, isDateInToday: date];
        let yesterday: bool = msg_send![calendar, isDateInYesterday: date];
        let formatter: id = msg_send![class!(NSDateFormatter), new];
        let format = if today {
            "'Today' HH:mm"
        } else if yesterday {
            "'Yesterday' HH:mm"
        } else {
            "d MMM yyyy HH:mm"
        };
        let format = NSString::alloc(nil).init_str(format);
        let _: () = msg_send![formatter, setDateFormat: format];
        let _: () = msg_send![format, release];
        let formatted: id = msg_send![formatter, stringFromDate: date];
        let utf8: *const std::ffi::c_char = msg_send![formatted, UTF8String];
        let result = if utf8.is_null() {
            "Copy time unavailable".into()
        } else {
            format!(
                "Copied {}",
                std::ffi::CStr::from_ptr(utf8).to_string_lossy()
            )
        };
        let _: () = msg_send![formatter, release];
        let _: () = msg_send![pool, drain];
        result
    }
}

#[cfg(not(target_os = "macos"))]
pub fn copied_at(_: f64) -> String {
    "Copy time unavailable".into()
}
