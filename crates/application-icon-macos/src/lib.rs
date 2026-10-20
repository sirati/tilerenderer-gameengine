//! The AppKit implementation of the common application icon API.

#![cfg(target_os = "macos")]
#![deny(unsafe_op_in_unsafe_fn)]

use objc2::rc::{Retained, autoreleasepool};
use objc2::{AnyThread, MainThreadMarker};
use objc2_app_kit::{NSApplication, NSImage};
use objc2_foundation::NSData;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the application icon must be set on the main thread")]
    MainThread,
    #[error("AppKit could not decode the application icon")]
    InvalidImage,
}

fn decode(png: &[u8]) -> Result<Retained<NSImage>, Error> {
    let data = NSData::with_bytes(png);
    let image = NSImage::initWithData(NSImage::alloc(), &data).ok_or(Error::InvalidImage)?;
    if !image.isValid() {
        return Err(Error::InvalidImage);
    }
    Ok(image)
}

/// Set this process's Dock/application icon without creating or activating a
/// window. The PNG is copied into AppKit-owned data before this returns.
pub fn set_png(png: &[u8]) -> Result<(), Error> {
    let mtm = MainThreadMarker::new().ok_or(Error::MainThread)?;
    autoreleasepool(|_| {
        let image = decode(png)?;
        let app = NSApplication::sharedApplication(mtm);
        // SAFETY: objc2 marks this setter unsafe because None may be invalid.
        // This always passes a valid, decoded NSImage in Some on the main
        // thread. AppKit retains the image after this call returns.
        unsafe {
            app.setApplicationIconImage(Some(&image));
        }
        Ok(())
    })
}
