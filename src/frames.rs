//! Frame sources: sequential RGB24 producers for the streaming GIF experiment.

//* Libraries imports
use std::future::Future;
use std::io;
use std::pin::Pin;

/// Synthetic scene dimensions (control experiment).
pub const WIDTH: u16 = 64;
pub const HEIGHT: u16 = 64;
pub const FRAME_PIXELS: usize = (WIDTH as usize) * (HEIGHT as usize);

const BLOCK_SIZE: u16 = 8;

/// One full frame of packed RGB24 pixels (`width * height * 3` bytes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbFrame {
    pub pixels: Vec<u8>,
}

/// Future returned by [`FrameSource::next_frame`].
pub type NextFrameFuture = Pin<Box<dyn Future<Output = io::Result<Option<RgbFrame>>> + Send>>;

/// Sequential RGB frame producer (not random-access).
pub trait FrameSource: Send {
    fn width(&self) -> u16;
    fn height(&self) -> u16;

    /// Returns the next frame, or `None` at end-of-source.
    fn next_frame(&mut self) -> NextFrameFuture;

    /// Restart from the beginning (used by `TrailerPolicy::Never` looping).
    fn reset(&mut self) -> io::Result<()>;
}

/// Generates full-frame buffers of palette indices (no LZW). Kept for tests of
/// the original synthetic scene; prefer [`SyntheticFrameSource`] for GIF encoding.
pub struct FrameGenerator;

impl FrameGenerator {
    pub fn new() -> Self {
        Self
    }

    /// Returns a 64×64 buffer of palette indices for frame `n`.
    ///
    /// Scene: black background, 8×8 white block moving horizontally
    /// one pixel per frame (wraps at the right edge).
    pub fn generate(&self, n: u32) -> Vec<u8> {
        let mut buffer = vec![0u8; FRAME_PIXELS];
        let x0 = (n as u16) % WIDTH;
        let y0: u16 = (HEIGHT - BLOCK_SIZE) / 2;

        for dy in 0..BLOCK_SIZE {
            for dx in 0..BLOCK_SIZE {
                let x = (x0 + dx) % WIDTH;
                let y = y0 + dy;
                let index = (y as usize) * (WIDTH as usize) + (x as usize);
                buffer[index] = 1;
            }
        }

        buffer
    }

    /// Same scene as [`Self::generate`], expanded to RGB24.
    pub fn generate_rgb(&self, n: u32) -> RgbFrame {
        let indexed = self.generate(n);
        let mut pixels = Vec::with_capacity(FRAME_PIXELS * 3);
        for &index in &indexed {
            if index == 0 {
                pixels.extend_from_slice(&[0, 0, 0]);
            } else {
                pixels.extend_from_slice(&[255, 255, 255]);
            }
        }
        RgbFrame { pixels }
    }
}

impl Default for FrameGenerator {
    fn default() -> Self {
        Self::new()
    }
}

/// Walking-block synthetic scene as a [`FrameSource`].
pub struct SyntheticFrameSource {
    generator: FrameGenerator,
    frame_index: u32,
}

impl SyntheticFrameSource {
    pub fn new() -> Self {
        Self {
            generator: FrameGenerator::new(),
            frame_index: 0,
        }
    }
}

impl Default for SyntheticFrameSource {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameSource for SyntheticFrameSource {
    fn width(&self) -> u16 {
        WIDTH
    }

    fn height(&self) -> u16 {
        HEIGHT
    }

    fn next_frame(&mut self) -> NextFrameFuture {
        let n = self.frame_index;
        self.frame_index = self.frame_index.wrapping_add(1);
        let frame = self.generator.generate_rgb(n);
        Box::pin(async move { Ok(Some(frame)) })
    }

    fn reset(&mut self) -> io::Result<()> {
        self.frame_index = 0;
        Ok(())
    }
}

/// Fixed list of RGB frames for tests (no ffmpeg, no large files).
#[cfg(test)]
pub struct FixedFrameSource {
    width: u16,
    height: u16,
    frames: Vec<RgbFrame>,
    index: usize,
}

#[cfg(test)]
impl FixedFrameSource {
    pub fn new(width: u16, height: u16, frames: Vec<RgbFrame>) -> Self {
        let expected = (width as usize) * (height as usize) * 3;
        for (i, frame) in frames.iter().enumerate() {
            assert_eq!(
                frame.pixels.len(),
                expected,
                "frame {i} has wrong RGB byte length"
            );
        }
        Self {
            width,
            height,
            frames,
            index: 0,
        }
    }
}

#[cfg(test)]
impl FrameSource for FixedFrameSource {
    fn width(&self) -> u16 {
        self.width
    }

    fn height(&self) -> u16 {
        self.height
    }

    fn next_frame(&mut self) -> NextFrameFuture {
        let frame = if self.index < self.frames.len() {
            let frame = self.frames[self.index].clone();
            self.index += 1;
            Some(frame)
        } else {
            None
        };
        Box::pin(async move { Ok(frame) })
    }

    fn reset(&mut self) -> io::Result<()> {
        self.index = 0;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::FutureExt;

    mod frame_generator {
        use super::*;

        #[test]
        fn returns_a_buffer_of_4096_palette_indices() {
            let generator = FrameGenerator::new();
            let frame = generator.generate(0);
            assert_eq!(frame.len(), 4096);
        }

        #[test]
        fn uses_only_palette_indices_zero_and_one() {
            let generator = FrameGenerator::new();
            for n in 0..64 {
                let frame = generator.generate(n);
                assert!(
                    frame.iter().all(|&i| i == 0 || i == 1),
                    "frame {n} contained an index outside {{0,1}}"
                );
            }
        }

        #[test]
        fn places_the_white_block_at_the_left_edge_for_frame_zero() {
            let generator = FrameGenerator::new();
            let frame = generator.generate(0);
            // Block origin (0, 28); top-left pixel of the block.
            assert_eq!(frame[28 * 64 + 0], 1);
            // First column past the 8-wide block is still black.
            assert_eq!(frame[28 * 64 + 8], 0);
            // Background far from the block.
            assert_eq!(frame[0], 0);
        }

        #[test]
        fn moves_the_white_block_one_pixel_right_for_frame_one() {
            let generator = FrameGenerator::new();
            let frame = generator.generate(1);
            // Block origin (1, 28).
            assert_eq!(frame[28 * 64 + 1], 1);
            assert_eq!(frame[28 * 64 + 0], 0);
            assert_eq!(frame[28 * 64 + 9], 0);
        }

        #[test]
        fn generate_rgb_expands_palette_indices_to_rgb24() {
            let generator = FrameGenerator::new();
            let rgb = generator.generate_rgb(0);
            assert_eq!(rgb.pixels.len(), FRAME_PIXELS * 3);
            // Top-left of white block at (0, 28) → white.
            let i = (28 * 64 + 0) * 3;
            assert_eq!(&rgb.pixels[i..i + 3], &[255, 255, 255]);
            // Background → black.
            assert_eq!(&rgb.pixels[0..3], &[0, 0, 0]);
        }
    }

    mod synthetic_frame_source {
        use super::*;

        #[tokio::test]
        async fn yields_frames_with_synthetic_dimensions() {
            let mut source = SyntheticFrameSource::new();
            assert_eq!(source.width(), WIDTH);
            assert_eq!(source.height(), HEIGHT);
            let frame = source.next_frame().await.expect("ok").expect("some");
            assert_eq!(frame.pixels.len(), FRAME_PIXELS * 3);
        }

        #[tokio::test]
        async fn reset_restarts_the_scene_from_frame_zero() {
            let mut source = SyntheticFrameSource::new();
            let first = source.next_frame().await.unwrap().unwrap();
            let _ = source.next_frame().await.unwrap().unwrap();
            source.reset().unwrap();
            let again = source.next_frame().await.unwrap().unwrap();
            assert_eq!(first, again);
        }
    }

    mod fixed_frame_source {
        use super::*;

        fn solid(width: u16, height: u16, rgb: [u8; 3]) -> RgbFrame {
            let n = (width as usize) * (height as usize);
            let mut pixels = Vec::with_capacity(n * 3);
            for _ in 0..n {
                pixels.extend_from_slice(&rgb);
            }
            RgbFrame { pixels }
        }

        #[tokio::test]
        async fn returns_none_after_the_last_frame() {
            let mut source = FixedFrameSource::new(
                2,
                2,
                vec![solid(2, 2, [255, 0, 0]), solid(2, 2, [0, 255, 0])],
            );
            assert!(source.next_frame().await.unwrap().is_some());
            assert!(source.next_frame().await.unwrap().is_some());
            assert!(source.next_frame().await.unwrap().is_none());
        }

        #[tokio::test]
        async fn reset_allows_replaying_the_same_frames() {
            let mut source = FixedFrameSource::new(2, 1, vec![solid(2, 1, [1, 2, 3])]);
            assert!(source.next_frame().await.unwrap().is_some());
            assert!(source.next_frame().await.unwrap().is_none());
            source.reset().unwrap();
            assert!(source.next_frame().await.unwrap().is_some());
        }

        #[test]
        fn next_frame_future_is_immediately_ready() {
            let mut source = FixedFrameSource::new(1, 1, vec![solid(1, 1, [0, 0, 0])]);
            assert!(source.next_frame().now_or_never().is_some());
        }
    }
}
