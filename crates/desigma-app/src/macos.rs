//! macOS never passes `desigma://` links or "open this document" clicks as arguments: they arrive
//! as Apple events (`GURL`/`GURL` and `aevt`/`odoc`), and winit does not surface them. Two small
//! Objective-C objects pick them up and queue them in `instance::push`; the UI drains that queue
//! every frame.
//!
//! * [`install_open_hook`] + [`install_delegate`] answer AppKit directly. A double click in
//!   Finder, "Open With", a drag onto the Dock icon or `open -a desigma FILE` all end up there.
//!   This is the path that matters: AppKit handles the `odoc` event itself while the app finishes
//!   launching, so an `NSAppleEventManager` handler installed at that moment never sees it (Cocoa
//!   replaces the handlers on launch), and AppKit then falls back to `NSDocumentController`, which
//!   has no document class for `.desigma` and shows the "cannot open files in the desigma document
//!   format" alert. Answering the delegate is what stops that alert and opens the file. winit 0.30
//!   registers an `NSApplicationDelegate` of its own and refuses to be replaced, so the answer is
//!   grafted onto that delegate's class instead, from an `-run` hook that fires after winit's
//!   delegate exists and before AppKit dispatches the launch event.
//! * [`register`] keeps the `NSAppleEventManager` handlers for the events AppKit does not
//!   delegate: `desigma://` links from a browser, and the opens forwarded to a second launch by
//!   [`crate::instance::forward`]. It is safe to call early (events that arrive before the window
//!   exists wait in the queue) and again once the app has launched.
//!
//! Both paths funnel through [`push_url`], and `instance::push` drops duplicates, so an event that
//! reaches us twice opens one file.

use crate::instance::{self, Open};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, ClassBuilder, Imp, NSObject, Sel};
use objc2::{declare_class, msg_send, msg_send_id, mutability, sel, ClassType, DeclaredClass};
use objc2_app_kit::NSApplication;
use objc2_foundation::{MainThreadMarker, NSAppleEventDescriptor, NSAppleEventManager, NSArray, NSURL};
use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::atomic::{AtomicPtr, Ordering};

/// `kInternetEventClass` and `kAEGetURL` are both 'GURL'.
const GURL: u32 = u32::from_be_bytes(*b"GURL");
/// `kCoreEventClass` 'aevt' and `kAEOpenDocuments` 'odoc'.
const AEVT: u32 = u32::from_be_bytes(*b"aevt");
const ODOC: u32 = u32::from_be_bytes(*b"odoc");
/// `keyDirectObject` '----' and `typeFileURL` 'furl'.
const KEY_DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");
const TYPE_FILE_URL: u32 = u32::from_be_bytes(*b"furl");

declare_class!(
    struct AppleEventHandler;

    unsafe impl ClassType for AppleEventHandler {
        type Super = NSObject;
        type Mutability = mutability::InteriorMutable;
        const NAME: &'static str = "DesigmaAppleEventHandler";
    }

    impl DeclaredClass for AppleEventHandler {}

    unsafe impl AppleEventHandler {
        #[method(handleURL:withReplyEvent:)]
        fn handle_url(&self, event: &NSAppleEventDescriptor, _reply: &NSAppleEventDescriptor) {
            if let Some(url) = direct_object(event).and_then(|d| unsafe { d.stringValue() }) {
                push_url(&url.to_string());
            }
        }

        #[method(handleOpenDocuments:withReplyEvent:)]
        fn handle_open_documents(&self, event: &NSAppleEventDescriptor, _reply: &NSAppleEventDescriptor) {
            let Some(list) = direct_object(event) else { return };
            // the items of a list are 1-based
            for i in 1..=unsafe { list.numberOfItems() } {
                let item = unsafe { list.descriptorAtIndex(i) };
                if let Some(path) = item.and_then(|d| file_path(&d)) {
                    instance::push(Open::File(path));
                }
            }
        }
    }
);

fn direct_object(event: &NSAppleEventDescriptor) -> Option<Retained<NSAppleEventDescriptor>> {
    unsafe { msg_send_id![event, paramDescriptorForKeyword: KEY_DIRECT_OBJECT] }
}

/// The path of a file item (an alias or file reference in the event), through its file URL.
fn file_path(item: &NSAppleEventDescriptor) -> Option<std::path::PathBuf> {
    let url: Option<Retained<NSAppleEventDescriptor>> = unsafe { msg_send_id![item, coerceToDescriptorType: TYPE_FILE_URL] };
    let url = url?;
    let data = unsafe { url.data() };
    let text = std::str::from_utf8(data.bytes()).ok()?.trim_end_matches('\0');
    url::Url::parse(text).ok()?.to_file_path().ok()
}

thread_local! {
    // The manager does not retain its handler, so keep it for the life of the main thread.
    static HANDLER: RefCell<Option<Retained<AppleEventHandler>>> = const { RefCell::new(None) };
}

/// Registers the handlers for links and for the opens a second launch forwards to us. Main thread
/// only (both callers are).
///
/// Documents opened by the user do not come through here: AppKit dispatches those itself, which is
/// why [`install_delegate`] exists.
pub fn register() {
    HANDLER.with(|slot| {
        let mut slot = slot.borrow_mut();
        let handler = slot.get_or_insert_with(|| {
            let this = AppleEventHandler::alloc().set_ivars(());
            unsafe { msg_send_id![super(this), init] }
        });
        unsafe {
            let manager = NSAppleEventManager::sharedAppleEventManager();
            let _: () = msg_send![&*manager, setEventHandler: &**handler, andSelector: sel!(handleURL:withReplyEvent:), forEventClass: GURL, andEventID: GURL];
            let _: () =
                msg_send![&*manager, setEventHandler: &**handler, andSelector: sel!(handleOpenDocuments:withReplyEvent:), forEventClass: AEVT, andEventID: ODOC];
        }
    });
}

/// Queues whatever a URL means: a file for `file:`, the link itself otherwise.
///
/// Both entry points (the AppKit delegate and the Apple event handler) go through here, so a
/// `desigma://` link and a document behave the same no matter which one received them.
fn push_url(text: &str) {
    let Ok(parsed) = url::Url::parse(text) else { return };
    if parsed.scheme() == "file" {
        if let Ok(path) = parsed.to_file_path() {
            instance::push(Open::File(path));
        }
    } else {
        instance::push(Open::Link(text.to_string()));
    }
}

/// AppKit's entry point for a document open: a Finder double click, "Open With", a drop on the
/// Dock icon and `open -a desigma FILE` all end up here.
///
/// # Safety
///
/// Called by AppKit with the arguments of `-application:openURLs:`.
unsafe extern "C" fn application_open_urls(_this: *mut AnyObject, _cmd: Sel, _application: *mut NSApplication, urls: *mut NSArray<NSURL>) {
    let Some(urls) = (unsafe { urls.as_ref() }) else { return };
    for url in urls.iter() {
        // unsupported URLs parse to nothing in `push_url`, so they are dropped there
        if let Some(text) = unsafe { url.absoluteString() } {
            push_url(&text.to_string());
        }
    }
}

/// The subclass that answers document opens.
const OPEN_URLS_CLASS: &str = "DesigmaOpenURLsDelegate";

/// Teaches the delegate winit installs to answer document opens. Main thread only.
///
/// The delegate cannot be replaced: winit 0.30 asserts that `NSApp.delegate` is its own class
/// (`WinitApp::get`: "tried to get a delegate that was not the one Winit has registered") every
/// time it pumps the event loop, so a delegate of our own dies on the first frame. Instead this
/// keeps winit's delegate and adds `application:openURLs:` to a *subclass* of the class winit
/// registered, then moves the very same instance onto that subclass with
/// [`AnyObject::set_class`]. The instance stays a kind of winit's class (the assertion holds), and
/// the subclass adds no instance variables, so the instance size is unchanged, which is what makes
/// the class switch sound.
///
/// Call it from the `-run` hook (see [`install_open_hook`]) and once more from the app; both calls
/// are cheap and the second one returns early once the hook is in place.
pub fn install_delegate() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    let app = NSApplication::sharedApplication(mtm);
    let Some(delegate): Option<Retained<NSObject>> = (unsafe { msg_send_id![&*app, delegate] }) else {
        return;
    };
    let object = unsafe { &*(&*delegate as *const NSObject as *const AnyObject) };
    let current = object.class();
    if current.name() == OPEN_URLS_CLASS {
        // already hooked
        return;
    }
    let hooked = match AnyClass::get(OPEN_URLS_CLASS) {
        Some(hooked) => hooked,
        None => {
            let Some(mut builder) = ClassBuilder::new(OPEN_URLS_CLASS, current) else { return };
            unsafe {
                builder.add_method(
                    sel!(application:openURLs:),
                    application_open_urls as unsafe extern "C" fn(*mut AnyObject, Sel, *mut NSApplication, *mut NSArray<NSURL>),
                );
            }
            builder.register()
        }
    };
    // SAFETY: `hooked` is a subclass of the delegate's class and declares no ivars of its own, so
    // the instance size is unchanged and every winit callback still resolves through its class.
    unsafe {
        AnyObject::set_class(object, hooked);
    }
}

/// The original `-[NSApplication run]`, kept so the swap below can chain to it.
static ORIGINAL_RUN: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());

/// Installs the open hook, then lets AppKit run normally.
///
/// # Safety
///
/// Called by AppKit as `-[NSApplication run]`.
unsafe extern "C" fn run_with_open_hook(_this: *mut AnyObject, _cmd: Sel) {
    install_delegate();
    // SAFETY: `install_open_hook` stored the implementation it replaced before swapping.
    let original: Imp = unsafe { std::mem::transmute(ORIGINAL_RUN.load(Ordering::Relaxed)) };
    unsafe { original() }
}

/// Hooks `-[NSApplication run]` so [`install_delegate`] runs at the last moment that is still early
/// enough. Main thread only; call once before `eframe`.
///
/// winit installs its delegate while the event loop is built, which happens inside
/// [`eframe::run_native`], and the document AppKit was launched for arrives as an Apple event
/// during `-finishLaunching`, i.e. before the first frame. So there is no place of our own between
/// "winit's delegate exists" and "AppKit asks it to open the file". `-run` is exactly that seam:
/// it is called after the event loop is built and before `-finishLaunching`.
pub fn install_open_hook() {
    let Some(mtm) = MainThreadMarker::new() else { return };
    // the shared application has to exist before its methods can be swapped
    let _ = NSApplication::sharedApplication(mtm);
    let Some(method) = NSApplication::class().instance_method(sel!(run)) else { return };
    let original = method.implementation();
    // swapping the slot first keeps a second call from chaining to itself
    let previous = ORIGINAL_RUN.swap(original as *const () as *mut c_void, Ordering::Relaxed);
    if previous.is_null() {
        unsafe { method.set_implementation(std::mem::transmute::<unsafe extern "C" fn(*mut AnyObject, Sel), Imp>(run_with_open_hook)) };
    }
}
