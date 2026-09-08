//! Synthetic 64×64 palette-index frames for the streaming GIF experiment.

pub const WIDTH: u16 = 64;
pub const HEIGHT: u16 = 64;
pub const FRAME_PIXELS: usize = (WIDTH as usize) * (HEIGHT as usize);

/// Global palette in `[r, g, b, ...]` form: index 0 = black, index 1 = white.
pub const GLOBAL_PALETTE: &[u8] = &[0, 0, 0, 255, 255, 255];

const BLOCK_SIZE: u16 = 8;

/// Generates full-frame buffers of palette indices (no LZW).
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
}

impl Default for FrameGenerator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    }
}
