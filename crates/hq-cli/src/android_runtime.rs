//! The phone app runs `hq` as a child process, and a child process has no JVM.
//!
//! Two crates under the bridge ask for the JVM through `ndk-context`: `netdev` (to read the
//! interfaces' extras) and iroh's system DNS reader. `ndk_context::android_context()` panics
//! when nothing initialised it, and the release profiles abort on panic, so the first call ended
//! the process (the spike measured a printed panic only because its build unwound).
//!
//! `init` sets a context with null pointers. `netdev` checks them and skips the JNI-only extras.
//! The DNS reader does not survive a null JVM, so nothing may reach it: `hq_bridge::dns` gives
//! every endpoint its own resolver. Whoever adds a new iroh endpoint passes that resolver too.

/// Called once, first thing in `main`. A host that has a real context (a JNI library loaded
/// into the app) must not call it, since the context can be set only once.
#[cfg(target_os = "android")]
pub fn init() {
    // Safety: the pointers are never dereferenced; both readers check them for null first.
    unsafe { ndk_context::initialize_android_context(std::ptr::null_mut(), std::ptr::null_mut()) }
}

#[cfg(not(target_os = "android"))]
pub fn init() {}
