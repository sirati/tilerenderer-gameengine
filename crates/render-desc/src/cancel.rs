//! Cooperative cancellation between loads of a render description.

use crate::{ImageLoader, IndexedFrame};
use tile_render::RgbaImage;

/// The error returned when a cancelled compile next asks for a file.
pub const CANCELLED: &str = "no longer wanted";

/// An image loader that stops reading once `stop` says its work is no
/// longer wanted. An ongoing file load finishes; the next load fails.
/// Callers must also discard a completed compile if it was cancelled
/// during its last load or while the atlas was being packed.
pub struct StopWhen<L, F> {
    inner: L,
    stop: F,
}

impl<L, F: Fn() -> bool> StopWhen<L, F> {
    pub fn new(inner: L, stop: F) -> Self {
        Self { inner, stop }
    }

    fn check(&self) -> Result<(), String> {
        if (self.stop)() { Err(CANCELLED.to_owned()) } else { Ok(()) }
    }
}

impl<L: ImageLoader, F: Fn() -> bool> ImageLoader for StopWhen<L, F> {
    fn load(&self, source: &str, options: &toml::Table) -> Result<Vec<RgbaImage>, String> {
        self.check()?;
        self.inner.load(source, options)
    }

    fn sizes(&self, source: &str, options: &toml::Table) -> Result<Vec<(u32, u32)>, String> {
        self.check()?;
        self.inner.sizes(source, options)
    }

    fn font(&self, source: &str) -> Result<Vec<u8>, String> {
        self.check()?;
        self.inner.font(source)
    }

    fn identity(&self, source: &str, options: &toml::Table) -> String {
        self.inner.identity(source, options)
    }

    fn indexed(&self, source: &str, options: &toml::Table) -> Result<Option<Vec<IndexedFrame>>, String> {
        self.check()?;
        self.inner.indexed(source, options)
    }

    fn palette(&self, source: &str, options: &toml::Table) -> Result<Vec<[u8; 4]>, String> {
        self.check()?;
        self.inner.palette(source, options)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;
    use crate::Description;

    const DESCRIPTION: &str = r#"
[grid]
layout = "orthogonal"
step = [4, 4]
[images.first]
source = "first"
[images.second]
source = "second"
[sheets.first]
image = "first"
cell = [4, 4]
[sheets.second]
image = "second"
cell = [4, 4]
"#;

    /// Cancels while a file is read. The next read must never reach this
    /// loader, including when a compile reads only the image headers.
    struct Superseded<'a> {
        stopped: &'a Cell<bool>,
        reads: &'a Cell<u32>,
    }

    impl ImageLoader for Superseded<'_> {
        fn load(&self, _: &str, _: &toml::Table) -> Result<Vec<RgbaImage>, String> {
            self.reads.set(self.reads.get() + 1);
            self.stopped.set(true);
            Ok(vec![RgbaImage::new(4, 4)])
        }

        fn sizes(&self, _: &str, _: &toml::Table) -> Result<Vec<(u32, u32)>, String> {
            self.reads.set(self.reads.get() + 1);
            self.stopped.set(true);
            Ok(vec![(4, 4)])
        }
    }

    #[test]
    fn a_compile_superseded_during_a_load_stops_before_the_next_file() {
        for pixels in [true, false] {
            let (stopped, reads) = (Cell::new(false), Cell::new(0));
            let loader = StopWhen::new(Superseded { stopped: &stopped, reads: &reads }, || stopped.get());
            let error = match Description::parse(DESCRIPTION).unwrap().compile_growing(&loader, 64, pixels) {
                Err(e) => e,
                Ok(_) => panic!("a superseded compile finished"),
            };
            assert!(error.to_string().contains(CANCELLED), "{error}");
            assert_eq!(reads.get(), 1, "no other file read after cancellation, pixels={pixels}");
        }
    }
}
