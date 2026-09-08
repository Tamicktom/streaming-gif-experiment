//! Incremental GIF byte stream: header, frames, flush-as-chunks, trailer policy.

//* Libraries imports
use std::io::{self, Write};
use std::mem::ManuallyDrop;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;
use futures_util::Stream;
use gif::{Encoder, Frame, Repeat};

//* Local imports
use crate::frames::{FrameGenerator, GLOBAL_PALETTE, HEIGHT, WIDTH};

/// When (if ever) to emit the GIF trailer byte `0x3B`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrailerPolicy {
    /// Open stream — never write the trailer (phase 2+).
    Never,
    /// Finite GIF — write trailer after `n` frames (phase 1).
    AfterN(u32),
    /// Best-effort trailer when the stream is dropped (phase 5).
    OnDrop,
}

/// Owns the open GIF byte stream behind a small interface.
#[derive(Clone)]
pub struct GifStream {
    interval: Duration,
    policy: TrailerPolicy,
    /// Graphic control delay in units of 10 ms (100 = 1.00 s).
    frame_delay_cs: u16,
}

impl GifStream {
    pub fn new(interval: Duration, policy: TrailerPolicy) -> Self {
        Self {
            interval,
            policy,
            frame_delay_cs: duration_to_gif_delay(interval),
        }
    }

    /// Encode frames into a single buffer (tests / offline).
    ///
    /// - `AfterN(n)`: encodes `n` frames and a trailer (ignores `max_frames`).
    /// - `Never`: encodes `max_frames` frames and **no** trailer.
    /// - `OnDrop`: encodes `max_frames` frames then writes a trailer via Drop.
    pub fn encode_to_vec(&self, max_frames: u32) -> io::Result<Vec<u8>> {
        let mut output = Vec::new();
        let generator = FrameGenerator::new();
        let frame_count = match self.policy {
            TrailerPolicy::AfterN(n) => n,
            TrailerPolicy::Never | TrailerPolicy::OnDrop => max_frames,
        };

        match self.policy {
            TrailerPolicy::Never => {
                let encoder = Encoder::new(&mut output, WIDTH, HEIGHT, GLOBAL_PALETTE)
                    .map_err(encoding_to_io)?;
                let mut encoder = ManuallyDrop::new(encoder);
                encoder
                    .set_repeat(Repeat::Infinite)
                    .map_err(encoding_to_io)?;
                for n in 0..frame_count {
                    write_frame(&mut *encoder, &generator, n, self.frame_delay_cs)?;
                }
                // Prevent `Encoder::Drop` from writing the trailer.
                std::mem::forget(ManuallyDrop::into_inner(encoder));
            }
            TrailerPolicy::AfterN(_) | TrailerPolicy::OnDrop => {
                let mut encoder = Encoder::new(&mut output, WIDTH, HEIGHT, GLOBAL_PALETTE)
                    .map_err(encoding_to_io)?;
                encoder
                    .set_repeat(Repeat::Infinite)
                    .map_err(encoding_to_io)?;
                for n in 0..frame_count {
                    write_frame(&mut encoder, &generator, n, self.frame_delay_cs)?;
                }
                // Drop writes the trailer `0x3B`.
                drop(encoder);
            }
        }

        Ok(output)
    }

    /// Async stream of body chunks for Axum (`Body::from_stream`).
    pub fn into_byte_stream(self) -> GifByteStream {
        GifByteStream::new(self)
    }
}

fn write_frame<W: Write>(
    encoder: &mut Encoder<W>,
    generator: &FrameGenerator,
    n: u32,
    delay_cs: u16,
) -> io::Result<()> {
    let pixels = generator.generate(n);
    let mut frame = Frame::from_indexed_pixels(WIDTH, HEIGHT, pixels, None);
    frame.delay = delay_cs;
    encoder.write_frame(&frame).map_err(encoding_to_io)
}

fn encoding_to_io(err: gif::EncodingError) -> io::Error {
    io::Error::other(err.to_string())
}

fn duration_to_gif_delay(interval: Duration) -> u16 {
    if interval.is_zero() {
        return 0;
    }
    let cs = (interval.as_millis() / 10).min(u16::MAX as u128) as u16;
    cs.max(1)
}

/// Accumulates encoder writes and allows draining as HTTP chunks.
struct ChunkWriter {
    buffer: Vec<u8>,
}

impl ChunkWriter {
    fn new() -> Self {
        Self {
            buffer: Vec::new(),
        }
    }

    fn drain(&mut self) -> Bytes {
        Bytes::from(std::mem::take(&mut self.buffer))
    }
}

impl Write for ChunkWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.buffer.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Async stream yielding GIF bytes one flush-chunk at a time.
pub struct GifByteStream {
    generator: FrameGenerator,
    interval: Duration,
    policy: TrailerPolicy,
    frame_delay_cs: u16,
    phase: Phase,
    pending: Option<Bytes>,
    sleep: Option<Pin<Box<tokio::time::Sleep>>>,
}

enum Phase {
    Header,
    Frame {
        encoder: ManuallyDrop<Encoder<ChunkWriter>>,
        frame_index: u32,
        waiting: bool,
    },
    Done,
}

impl GifByteStream {
    fn new(config: GifStream) -> Self {
        Self {
            generator: FrameGenerator::new(),
            interval: config.interval,
            policy: config.policy,
            frame_delay_cs: config.frame_delay_cs,
            phase: Phase::Header,
            pending: None,
            sleep: None,
        }
    }

    fn release_encoder(encoder: ManuallyDrop<Encoder<ChunkWriter>>, write_trailer: bool) -> Bytes {
        if write_trailer {
            let encoder = ManuallyDrop::into_inner(encoder);
            match encoder.into_inner() {
                Ok(mut writer) => writer.drain(),
                Err(_) => Bytes::new(),
            }
        } else {
            let mut encoder = ManuallyDrop::into_inner(encoder);
            let chunk = encoder.get_mut().drain();
            std::mem::forget(encoder);
            chunk
        }
    }
}

impl Drop for GifByteStream {
    fn drop(&mut self) {
        if let Phase::Frame { encoder, .. } = std::mem::replace(&mut self.phase, Phase::Done) {
            match self.policy {
                TrailerPolicy::Never => {
                    let _ = Self::release_encoder(encoder, false);
                }
                TrailerPolicy::OnDrop | TrailerPolicy::AfterN(_) => {
                    let _ = Self::release_encoder(encoder, true);
                }
            }
        }
    }
}

impl Stream for GifByteStream {
    type Item = Result<Bytes, io::Error>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        if let Some(chunk) = this.pending.take() {
            return Poll::Ready(Some(Ok(chunk)));
        }

        loop {
            if let Some(sleep) = this.sleep.as_mut() {
                match sleep.as_mut().poll(cx) {
                    Poll::Ready(()) => this.sleep = None,
                    Poll::Pending => return Poll::Pending,
                }
            }

            match this.phase {
                Phase::Done => return Poll::Ready(None),
                Phase::Header => {
                    let writer = ChunkWriter::new();
                    let mut encoder = match Encoder::new(writer, WIDTH, HEIGHT, GLOBAL_PALETTE) {
                        Ok(encoder) => encoder,
                        Err(err) => {
                            this.phase = Phase::Done;
                            return Poll::Ready(Some(Err(encoding_to_io(err))));
                        }
                    };
                    if let Err(err) = encoder.set_repeat(Repeat::Infinite) {
                        this.phase = Phase::Done;
                        return Poll::Ready(Some(Err(encoding_to_io(err))));
                    }
                    let chunk = encoder.get_mut().drain();
                    this.phase = Phase::Frame {
                        encoder: ManuallyDrop::new(encoder),
                        frame_index: 0,
                        waiting: false,
                    };
                    return Poll::Ready(Some(Ok(chunk)));
                }
                Phase::Frame { waiting: true, .. } => {
                    if this.interval.is_zero() {
                        if let Phase::Frame { waiting, .. } = &mut this.phase {
                            *waiting = false;
                        }
                        continue;
                    }
                    this.sleep = Some(Box::pin(tokio::time::sleep(this.interval)));
                    if let Phase::Frame { waiting, .. } = &mut this.phase {
                        *waiting = false;
                    }
                    continue;
                }
                Phase::Frame {
                    waiting: false, ..
                } => {
                    let Phase::Frame {
                        encoder,
                        frame_index,
                        ..
                    } = std::mem::replace(&mut this.phase, Phase::Done)
                    else {
                        unreachable!();
                    };

                    let mut encoder = encoder;
                    if let Err(err) = write_frame(
                        &mut *encoder,
                        &this.generator,
                        frame_index,
                        this.frame_delay_cs,
                    ) {
                        let _ = Self::release_encoder(encoder, false);
                        return Poll::Ready(Some(Err(err)));
                    }

                    let chunk = encoder.get_mut().drain();
                    let next_index = frame_index + 1;

                    if matches!(this.policy, TrailerPolicy::AfterN(n) if next_index >= n) {
                        let trailer = Self::release_encoder(encoder, true);
                        this.phase = Phase::Done;
                        if !trailer.is_empty() {
                            this.pending = Some(trailer);
                        }
                        return Poll::Ready(Some(Ok(chunk)));
                    }

                    this.phase = Phase::Frame {
                        encoder,
                        frame_index: next_index,
                        waiting: true,
                    };
                    return Poll::Ready(Some(Ok(chunk)));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count_decoded_frames(bytes: &[u8]) -> usize {
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::Indexed);
        let mut decoder = options.read_info(std::io::Cursor::new(bytes)).expect("decode header");
        let mut count = 0;
        while decoder.read_next_frame().expect("decode frame").is_some() {
            count += 1;
        }
        count
    }

    mod gif_stream {
        use super::*;

        #[test]
        fn after_n_starts_with_gif89a_and_ends_with_trailer() {
            let stream = GifStream::new(Duration::ZERO, TrailerPolicy::AfterN(10));
            let bytes = stream.encode_to_vec(10).expect("encode");
            assert!(bytes.starts_with(b"GIF89a"), "missing GIF89a header");
            assert_eq!(*bytes.last().unwrap(), 0x3B, "missing trailer");
        }

        #[test]
        fn never_starts_with_gif89a_and_has_no_trailer() {
            let stream = GifStream::new(Duration::ZERO, TrailerPolicy::Never);
            let bytes = stream.encode_to_vec(3).expect("encode");
            assert!(bytes.starts_with(b"GIF89a"), "missing GIF89a header");
            assert_ne!(*bytes.last().unwrap(), 0x3B, "unexpected trailer under Never");
        }

        #[test]
        fn after_n_grows_with_each_additional_frame() {
            let one = GifStream::new(Duration::ZERO, TrailerPolicy::AfterN(1))
                .encode_to_vec(1)
                .expect("encode 1");
            let two = GifStream::new(Duration::ZERO, TrailerPolicy::AfterN(2))
                .encode_to_vec(2)
                .expect("encode 2");
            assert!(two.len() > one.len(), "expected more bytes for more frames");
        }

        #[test]
        fn after_n_encode_to_vec_decodes_to_exactly_ten_frames() {
            let bytes = GifStream::new(Duration::ZERO, TrailerPolicy::AfterN(10))
                .encode_to_vec(10)
                .expect("encode");
            assert_eq!(*bytes.last().unwrap(), 0x3B, "missing trailer");
            assert_eq!(count_decoded_frames(&bytes), 10);
        }

        #[tokio::test]
        async fn after_n_byte_stream_decodes_to_exactly_ten_frames() {
            use futures_util::StreamExt;

            let stream = GifStream::new(Duration::ZERO, TrailerPolicy::AfterN(10)).into_byte_stream();
            let chunks: Vec<Bytes> = stream
                .collect::<Vec<_>>()
                .await
                .into_iter()
                .collect::<Result<Vec<_>, _>>()
                .expect("stream chunks");
            let bytes: Vec<u8> = chunks.into_iter().flat_map(|c| c.to_vec()).collect();

            assert!(bytes.starts_with(b"GIF89a"), "missing GIF89a header");
            assert_eq!(*bytes.last().unwrap(), 0x3B, "missing trailer");
            assert_eq!(count_decoded_frames(&bytes), 10);
        }

        #[tokio::test]
        async fn never_byte_stream_emits_more_than_ten_chunks_without_trailer_or_end() {
            use futures_util::StreamExt;
            use std::task::{Context, Poll, Waker};

            // Header + 12 frames = 13 chunks (more than AfterN(10) would emit).
            const CHUNK_COUNT: usize = 13;

            let mut stream =
                GifStream::new(Duration::ZERO, TrailerPolicy::Never).into_byte_stream();
            let mut chunks = Vec::with_capacity(CHUNK_COUNT);
            for _ in 0..CHUNK_COUNT {
                let chunk = stream
                    .next()
                    .await
                    .expect("Never stream must keep yielding")
                    .expect("chunk ok");
                chunks.push(chunk);
            }

            let bytes: Vec<u8> = chunks.into_iter().flat_map(|c| c.to_vec()).collect();
            assert!(bytes.starts_with(b"GIF89a"), "missing GIF89a header");
            assert_ne!(
                *bytes.last().unwrap(),
                0x3B,
                "Never must not emit trailer"
            );

            // With Duration::ZERO the next poll is Ready(Some), not Pending —
            // but it must not be Ready(None) (stream end).
            let waker = Waker::noop();
            let mut cx = Context::from_waker(&waker);
            match Pin::new(&mut stream).poll_next(&mut cx) {
                Poll::Ready(None) => panic!("Never stream must not end"),
                Poll::Ready(Some(Ok(_))) | Poll::Pending => {}
                Poll::Ready(Some(Err(err))) => panic!("unexpected stream error: {err}"),
            }
        }
    }
}
