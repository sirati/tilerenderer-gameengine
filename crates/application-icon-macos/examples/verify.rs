//! Noninteractive comparison with AppKit's standard application-icon setter.
#![deny(unsafe_op_in_unsafe_fn)]

#[cfg(target_os = "macos")]
fn rgba(bitmap: &objc2_app_kit::NSBitmapImageRep) -> image::RgbaImage {
    let width = u32::try_from(bitmap.pixelsWide()).unwrap();
    let height = u32::try_from(bitmap.pixelsHigh()).unwrap();
    image::RgbaImage::from_fn(width, height, |x, y| {
        let color = bitmap.colorAtX_y(x as isize, y as isize).unwrap();
        image::Rgba(
            [
                color.redComponent(),
                color.greenComponent(),
                color.blueComponent(),
                color.alphaComponent(),
            ]
            .map(|component| (component * 255.).round() as u8),
        )
    })
}

#[cfg(target_os = "macos")]
fn install_control(app: &objc2_app_kit::NSApplication, image: &objc2_app_kit::NSImage) {
    // SAFETY: This setter's only extra contract is that None may be invalid.
    // The caller supplies a valid NSImage in Some on the process main thread.
    unsafe {
        app.setApplicationIconImage(Some(image));
    }
}

#[cfg(target_os = "macos")]
fn main() {
    use objc2::{AnyThread, MainThreadMarker};
    use objc2_app_kit::{NSApplication, NSBitmapImageRep, NSImage};
    use objc2_foundation::NSData;

    let path = std::env::args_os().nth(1).expect("supply an icon PNG path");
    let png = std::fs::read(path).unwrap();
    let original = NSData::with_bytes(&png);
    let reference = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
        .unwrap()
        .to_rgba8();
    assert_eq!(
        rgba(&NSBitmapImageRep::imageRepWithData(&original).unwrap()),
        reference
    );
    let mtm = MainThreadMarker::new().unwrap();
    let app = NSApplication::sharedApplication(mtm);
    let source = NSImage::initWithData(NSImage::alloc(), &original).unwrap();
    assert!(source.isValid());
    // Use a direct native control to cover AppKit's backing scale, display ICC
    // conversion and internal raster precision. Reimplementing those operations
    // or comparing TIFF containers would not verify the installed artwork.
    install_control(&app, &source);
    let control = app.applicationIconImage().unwrap();
    assert_eq!(control.size(), source.size());
    let control_bitmap =
        NSBitmapImageRep::imageRepWithData(&control.TIFFRepresentation().unwrap()).unwrap();
    let expected = rgba(&control_bitmap);
    let expected_profile = control_bitmap.colorSpace().ICCProfileData().unwrap();

    // A no-op wrapper must fail. Replace the control with a distinct marker
    // first, then require the wrapper to restore all the control's pixels.
    let marker = image::RgbaImage::from_pixel(
        reference.width(),
        reference.height(),
        image::Rgba([0, 255, 255, 255]),
    );
    let mut marker_png = std::io::Cursor::new(Vec::new());
    marker
        .write_to(&mut marker_png, image::ImageFormat::Png)
        .unwrap();
    let marker =
        NSImage::initWithData(NSImage::alloc(), &NSData::with_bytes(marker_png.get_ref())).unwrap();
    assert!(marker.isValid());
    install_control(&app, &marker);
    let marker = app.applicationIconImage().unwrap();
    let marker_bitmap =
        NSBitmapImageRep::imageRepWithData(&marker.TIFFRepresentation().unwrap()).unwrap();
    assert_ne!(rgba(&marker_bitmap), expected);

    application_icon_macos::set_png(&png).unwrap();
    let actual = app.applicationIconImage().unwrap();
    assert!(actual.isValid());
    assert_eq!(actual.size(), source.size());
    let actual_bitmap =
        NSBitmapImageRep::imageRepWithData(&actual.TIFFRepresentation().unwrap()).unwrap();
    assert_eq!(rgba(&actual_bitmap), expected);
    assert!(
        actual_bitmap
            .colorSpace()
            .ICCProfileData()
            .unwrap()
            .isEqualToData(&expected_profile)
    );
    assert!(matches!(
        application_icon_macos::set_png(b"invalid image"),
        Err(application_icon_macos::Error::InvalidImage)
    ));
    let worker_png = png.clone();
    assert!(matches!(
        std::thread::spawn(move || application_icon_macos::set_png(&worker_png))
            .join()
            .unwrap(),
        Err(application_icon_macos::Error::MainThread)
    ));
    // Failed decode and wrong-thread calls leave the installed icon intact.
    let retained = app.applicationIconImage().unwrap();
    assert_eq!(retained.size(), source.size());
    let retained_bitmap =
        NSBitmapImageRep::imageRepWithData(&retained.TIFFRepresentation().unwrap()).unwrap();
    assert_eq!(rgba(&retained_bitmap), expected);
    assert!(
        retained_bitmap
            .colorSpace()
            .ICCProfileData()
            .unwrap()
            .isEqualToData(&expected_profile)
    );
    println!(
        "AppKit icon source RGBA, marker replacement, exact native-control RGBA/ICC/size, invalid-data retention, and exact MainThread checks passed: source {}x{}, native {}x{} ({} color/alpha bytes)",
        reference.width(),
        reference.height(),
        expected.width(),
        expected.height(),
        expected.as_raw().len()
    );
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("This check requires macOS/AppKit.");
    std::process::exit(1);
}
