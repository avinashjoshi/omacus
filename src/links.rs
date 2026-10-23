//! Share links coming into the app: from the command line (`omacus open <link>`, or
//! the OS starting Omacus for an `omacus://` link on Linux and Windows), from macOS
//! (which sends the link to the running app as an Apple Event), and from pasting one.
//! They queue up here and the window opens each as a sheet.

use std::sync::Mutex;

static PENDING: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Queue a sheet's text to open.
pub fn push(text: String) {
    PENDING.lock().unwrap().push(text);
}

/// The sheets waiting to open, oldest first.
pub fn take() -> Vec<String> {
    std::mem::take(&mut *PENDING.lock().unwrap())
}

/// The sheet in a command-line argument, if it's a share link.
pub fn from_arg(arg: &str) -> Option<String> {
    omacus_engine::share::decode(arg)
}

/// Make `omacus://` links open this copy of Omacus. Linux does it in the .desktop
/// file (see install.sh); macOS in Info.plist (see Cargo.toml); Windows here, per
/// user, since the .exe is portable and may move.
pub fn register() {
    #[cfg(windows)]
    windows::register();
    #[cfg(target_os = "macos")]
    macos::listen();
}

#[cfg(windows)]
mod windows {
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, REG_SZ, RegCloseKey, RegCreateKeyW, RegSetValueExW,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    fn set(key: &str, name: Option<&str>, value: &str) {
        let mut handle: HKEY = std::ptr::null_mut();
        let path = wide(key);
        let value = wide(value);
        let name = name.map(wide);
        // SAFETY: valid NUL-terminated UTF-16 strings and an out-pointer for the key;
        // the key is closed after use. Failures just leave links unregistered.
        unsafe {
            if RegCreateKeyW(HKEY_CURRENT_USER, path.as_ptr(), &mut handle) != 0 {
                return;
            }
            RegSetValueExW(
                handle,
                name.as_ref().map_or(std::ptr::null(), |n| n.as_ptr()),
                0,
                REG_SZ,
                value.as_ptr().cast(),
                (value.len() * 2) as u32,
            );
            RegCloseKey(handle);
        }
    }

    /// HKCU\Software\Classes\omacus, pointing at wherever this .exe is now.
    pub fn register() {
        let Ok(exe) = std::env::current_exe() else {
            return;
        };
        let exe = exe.display().to_string();
        set(r"Software\Classes\omacus", None, "URL:Omacus share link");
        set(r"Software\Classes\omacus", Some("URL Protocol"), "");
        set(
            r"Software\Classes\omacus\DefaultIcon",
            None,
            &format!("\"{exe}\",0"),
        );
        set(
            r"Software\Classes\omacus\shell\open\command",
            None,
            &format!("\"{exe}\" open \"%1\""),
        );
    }
}

#[cfg(target_os = "macos")]
mod macos {
    //! macOS doesn't pass a clicked link on the command line: it sends a "get URL"
    //! Apple Event, to the running app or the one it launches. Handle it with
    //! NSAppleEventManager before the window opens.

    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, NSObject};
    use objc2::{AllocAnyThread, define_class, msg_send, sel};
    use objc2_foundation::{NSAppleEventDescriptor, NSAppleEventManager};

    const INTERNET_EVENT_CLASS: u32 = u32::from_be_bytes(*b"GURL");
    const GET_URL: u32 = u32::from_be_bytes(*b"GURL");
    const DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");

    define_class!(
        #[unsafe(super(NSObject))]
        #[name = "OmacusLinkHandler"]
        struct Handler;

        impl Handler {
            #[unsafe(method(handleGetURL:withReplyEvent:))]
            fn handle(&self, event: &NSAppleEventDescriptor, _reply: &NSAppleEventDescriptor) {
                let url = event
                    .paramDescriptorForKeyword(DIRECT_OBJECT)
                    .and_then(|d| d.stringValue())
                    .map(|s| s.to_string());
                if let Some(text) = url.as_deref().and_then(super::from_arg) {
                    super::push(text);
                }
            }
        }
    );

    pub fn listen() {
        let handler: Retained<Handler> = unsafe { msg_send![Handler::alloc(), init] };
        let manager = NSAppleEventManager::sharedAppleEventManager();
        let target: &AnyObject = &handler;
        // SAFETY: the selector matches the method above; the handler is leaked so it
        // lives as long as the app.
        unsafe {
            manager.setEventHandler_andSelector_forEventClass_andEventID(
                target,
                sel!(handleGetURL:withReplyEvent:),
                INTERNET_EVENT_CLASS,
                GET_URL,
            );
        }
        std::mem::forget(handler);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_links_from_arguments() {
        let link = omacus_engine::share::link("# Hi\n2 + 2");
        assert_eq!(from_arg(&link).as_deref(), Some("# Hi\n2 + 2"));
        let scheme = link.replace("https://app.omacus.com", "omacus://open");
        assert_eq!(from_arg(&scheme).as_deref(), Some("# Hi\n2 + 2"));
        let old = link.replace("https://app.omacus.com", "https://omacus.com/app");
        assert_eq!(from_arg(&old).as_deref(), Some("# Hi\n2 + 2"));
        assert_eq!(from_arg("2 + 2"), None);
    }
}
