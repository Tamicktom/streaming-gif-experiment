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
use crate::frames::{FrameSource, NextFrameFuture, RgbFrame, SyntheticFrameSource};

/// 4×4×4 RGB cube (64 colours). Replaces per-frame NeuQuant (256 colours).
const CUBE64_PALETTE: [u8; 192] = cube64_palette();

const fn expand_2bit(value: u8) -> u8 {
    value * 85
}

const fn cube64_palette() -> [u8; 192] {
    let mut palette = [0u8; 192];
    let mut i = 0;
    let mut r = 0;
    while r < 4 {
        let mut g = 0;
        while g < 4 {
            let mut b = 0;
            while b < 4 {
                palette[i] = expand_2bit(r);
                palette[i + 1] = expand_2bit(g);
                palette[i + 2] = expand_2bit(b);
                i += 3;
                b += 1;
            }
            g += 1;
        }
        r += 1;
    }
    palette
}

fn cube64_index(r: u8, g: u8, b: u8) -> u8 {
    ((r >> 6) << 4) | ((g >> 6) << 2) | (b >> 6)
}

/// When (if ever) to emit the GIF trailer byte `0x3B`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrailerPolicy {
    /// Open stream — never write the trailer (phase 2+).
    Never,
    /// Finite GIF — write trailer after `n` frames (phase 1).
    AfterN(u32),
    /// Stream until source EOF (or drop), then trailer (phase 5 / video play-once).
    OnDrop,
}

/// Owns the open GIF byte stream behind a small interface.
pub struct GifStream {
    interval: Duration,
    policy: TrailerPolicy,
    /// Graphic control delay in units of 10 ms (100 = 1.00 s).
    frame_delay_cs: u16,
    source: Box<dyn FrameSource>,
}

impl GifStream {
    /// Synthetic walking-block control stream (original experiment).
    pub fn new(interval: Duration, policy: TrailerPolicy) -> Self {
        Self::from_source(interval, policy, Box::new(SyntheticFrameSource::new()))
    }

    /// Stream frames from an arbitrary [`FrameSource`].
    pub fn from_source(
        interval: Duration,
        policy: TrailerPolicy,
        source: Box<dyn FrameSource>,
    ) -> Self {
        Self {
            interval,
            policy,
            frame_delay_cs: duration_to_gif_delay(interval),
            source,
        }
    }

    /// Encode frames into a single buffer (tests / offline).
    ///
    /// - `AfterN(n)`: encodes `n` frames and a trailer (ignores `max_frames`).
    /// - `Never`: encodes `max_frames` frames and **no** trailer.
    /// - `OnDrop`: encodes until source EOF (or `max_frames` if source is infinite)
    ///   then writes a trailer via Drop.
    #[allow(dead_code)] // used by unit tests; kept for offline encode experiments
    pub async fn encode_to_vec(mut self, max_frames: u32) -> io::Result<Vec<u8>> {
        let mut output = Vec::new();
        let width = self.source.width();
        let height = self.source.height();
        let frame_count_limit = match self.policy {
            TrailerPolicy::AfterN(n) => n,
            TrailerPolicy::Never | TrailerPolicy::OnDrop => max_frames,
        };

        match self.policy {
            TrailerPolicy::Never => {
                let encoder = Encoder::new(&mut output, width, height, &CUBE64_PALETTE)
                    .map_err(encoding_to_io)?;
                let mut encoder = ManuallyDrop::new(encoder);
                encoder
                    .set_repeat(Repeat::Infinite)
                    .map_err(encoding_to_io)?;
                for _ in 0..frame_count_limit {
                    let rgb = next_frame_looping(&mut *self.source).await?;
                    write_rgb_frame(&mut *encoder, width, height, &rgb, self.frame_delay_cs)?;
                }
                // Prevent `Encoder::Drop` from writing the trailer.
                std::mem::forget(ManuallyDrop::into_inner(encoder));
            }
            TrailerPolicy::AfterN(_) | TrailerPolicy::OnDrop => {
                let mut encoder = Encoder::new(&mut output, width, height, &CUBE64_PALETTE)
                    .map_err(encoding_to_io)?;
                encoder
                    .set_repeat(Repeat::Infinite)
                    .map_err(encoding_to_io)?;
                for _ in 0..frame_count_limit {
                    match self.source.next_frame().await? {
                        Some(rgb) => {
                            write_rgb_frame(
                                &mut encoder,
                                width,
                                height,
                                &rgb,
                                self.frame_delay_cs,
                            )?;
                        }
                        None => break,
                    }
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

#[allow(dead_code)] // helper for `encode_to_vec` (Never policy)
async fn next_frame_looping(source: &mut dyn FrameSource) -> io::Result<RgbFrame> {
    loop {
        match source.next_frame().await? {
            Some(frame) => return Ok(frame),
            None => {
                source.reset()?;
            }
        }
    }
}

fn write_rgb_frame<W: Write>(
    encoder: &mut Encoder<W>,
    width: u16,
    height: u16,
    rgb: &RgbFrame,
    delay_cs: u16,
) -> io::Result<()> {
    let indexed: Vec<u8> = rgb
        .pixels
        .chunks_exact(3)
        .map(|pix| cube64_index(pix[0], pix[1], pix[2]))
        .collect();
    let mut frame = Frame::from_indexed_pixels(width, height, indexed, None);
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
        Self { buffer: Vec::new() }
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

enum Pending {
    NextFrame(NextFrameFuture),
    AfterReset(NextFrameFuture),
}

/// Async stream yielding GIF bytes one flush-chunk at a time.
pub struct GifByteStream {
    // `frame_op` must be declared before `source` so it is dropped first:
    // ffmpeg futures may hold a raw pointer into the source.
    frame_op: Option<Pending>,
    source: Box<dyn FrameSource>,
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
            frame_op: None,
            source: config.source,
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

    fn finish_with_trailer(
        &mut self,
        encoder: ManuallyDrop<Encoder<ChunkWriter>>,
        frame_chunk: Bytes,
    ) -> Poll<Option<Result<Bytes, io::Error>>> {
        let trailer = Self::release_encoder(encoder, true);
        self.phase = Phase::Done;
        if !trailer.is_empty() {
            self.pending = Some(trailer);
        }
        Poll::Ready(Some(Ok(frame_chunk)))
    }
}

impl Drop for GifByteStream {
    fn drop(&mut self) {
        // Drop any in-flight frame future before the source is torn down.
        self.frame_op = None;
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
                    let width = this.source.width();
                    let height = this.source.height();
                    let writer = ChunkWriter::new();
                    let mut encoder = match Encoder::new(writer, width, height, &CUBE64_PALETTE) {
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
                Phase::Frame { waiting: false, .. } => {
                    if this.frame_op.is_none() {
                        this.frame_op = Some(Pending::NextFrame(this.source.next_frame()));
                    }

                    let result = {
                        let pending = this.frame_op.as_mut().expect("frame_op set");
                        match pending {
                            Pending::NextFrame(fut) | Pending::AfterReset(fut) => {
                                match fut.as_mut().poll(cx) {
                                    Poll::Pending => return Poll::Pending,
                                    Poll::Ready(result) => result,
                                }
                            }
                        }
                    };
                    let was_after_reset = matches!(this.frame_op, Some(Pending::AfterReset(_)));
                    this.frame_op = None;

                    let rgb = match result {
                        Ok(Some(rgb)) => rgb,
                        Ok(None) => {
                            if matches!(this.policy, TrailerPolicy::Never) {
                                if let Err(err) = this.source.reset() {
                                    let Phase::Frame { encoder, .. } =
                                        std::mem::replace(&mut this.phase, Phase::Done)
                                    else {
                                        unreachable!();
                                    };
                                    let _ = Self::release_encoder(encoder, false);
                                    return Poll::Ready(Some(Err(err)));
                                }
                                this.frame_op = Some(Pending::AfterReset(this.source.next_frame()));
                                continue;
                            }
                            // EOF: finish with trailer for AfterN / OnDrop.
                            let Phase::Frame { encoder, .. } =
                                std::mem::replace(&mut this.phase, Phase::Done)
                            else {
                                unreachable!();
                            };
                            let trailer = Self::release_encoder(encoder, true);
                            this.phase = Phase::Done;
                            if trailer.is_empty() {
                                return Poll::Ready(None);
                            }
                            return Poll::Ready(Some(Ok(trailer)));
                        }
                        Err(err) => {
                            let Phase::Frame { encoder, .. } =
                                std::mem::replace(&mut this.phase, Phase::Done)
                            else {
                                unreachable!();
                            };
                            let _ = Self::release_encoder(encoder, false);
                            return Poll::Ready(Some(Err(err)));
                        }
                    };

                    if was_after_reset && rgb.pixels.is_empty() {
                        // Defensive: empty frame after reset should not happen.
                    }

                    let Phase::Frame {
                        encoder,
                        frame_index,
                        ..
                    } = std::mem::replace(&mut this.phase, Phase::Done)
                    else {
                        unreachable!();
                    };

                    let mut encoder = encoder;
                    let width = this.source.width();
                    let height = this.source.height();
                    if let Err(err) =
                        write_rgb_frame(&mut *encoder, width, height, &rgb, this.frame_delay_cs)
                    {
                        let _ = Self::release_encoder(encoder, false);
                        return Poll::Ready(Some(Err(err)));
                    }

                    let chunk = encoder.get_mut().drain();
                    let next_index = frame_index + 1;

                    if matches!(this.policy, TrailerPolicy::AfterN(n) if next_index >= n) {
                        return this.finish_with_trailer(encoder, chunk);
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
    use crate::frames::{FixedFrameSource, RgbFrame};

    fn count_decoded_frames(bytes: &[u8]) -> usize {
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::Indexed);
        let mut decoder = options
            .read_info(std::io::Cursor::new(bytes))
            .expect("decode header");
        let mut count = 0;
        while decoder.read_next_frame().expect("decode frame").is_some() {
            count += 1;
        }
        count
    }

    fn solid(width: u16, height: u16, rgb: [u8; 3]) -> RgbFrame {
        let n = (width as usize) * (height as usize);
        let mut pixels = Vec::with_capacity(n * 3);
        for _ in 0..n {
            pixels.extend_from_slice(&rgb);
        }
        RgbFrame { pixels }
    }

    mod gif_stream {
        use super::*;

        #[tokio::test]
        async fn after_n_starts_with_gif89a_and_ends_with_trailer() {
            let stream = GifStream::new(Duration::ZERO, TrailerPolicy::AfterN(10));
            let bytes = stream.encode_to_vec(10).await.expect("encode");
            assert!(bytes.starts_with(b"GIF89a"), "missing GIF89a header");
            assert_eq!(*bytes.last().unwrap(), 0x3B, "missing trailer");
        }

        #[tokio::test]
        async fn never_starts_with_gif89a_and_has_no_trailer() {
            let stream = GifStream::new(Duration::ZERO, TrailerPolicy::Never);
            let bytes = stream.encode_to_vec(3).await.expect("encode");
            assert!(bytes.starts_with(b"GIF89a"), "missing GIF89a header");
            assert_ne!(
                *bytes.last().unwrap(),
                0x3B,
                "unexpected trailer under Never"
            );
        }

        #[tokio::test]
        async fn after_n_grows_with_each_additional_frame() {
            let one = GifStream::new(Duration::ZERO, TrailerPolicy::AfterN(1))
                .encode_to_vec(1)
                .await
                .expect("encode 1");
            let two = GifStream::new(Duration::ZERO, TrailerPolicy::AfterN(2))
                .encode_to_vec(2)
                .await
                .expect("encode 2");
            assert!(two.len() > one.len(), "expected more bytes for more frames");
        }

        #[tokio::test]
        async fn after_n_encode_to_vec_decodes_to_exactly_ten_frames() {
            let bytes = GifStream::new(Duration::ZERO, TrailerPolicy::AfterN(10))
                .encode_to_vec(10)
                .await
                .expect("encode");
            assert_eq!(*bytes.last().unwrap(), 0x3B, "missing trailer");
            assert_eq!(count_decoded_frames(&bytes), 10);
        }

        #[tokio::test]
        async fn after_n_byte_stream_decodes_to_exactly_ten_frames() {
            use futures_util::StreamExt;

            let stream =
                GifStream::new(Duration::ZERO, TrailerPolicy::AfterN(10)).into_byte_stream();
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
            assert_ne!(*bytes.last().unwrap(), 0x3B, "Never must not emit trailer");

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

        #[tokio::test]
        async fn fixed_source_after_n_decodes_to_exactly_two_frames() {
            let source = FixedFrameSource::new(
                4,
                4,
                vec![solid(4, 4, [255, 0, 0]), solid(4, 4, [0, 0, 255])],
            );
            let bytes =
                GifStream::from_source(Duration::ZERO, TrailerPolicy::AfterN(2), Box::new(source))
                    .encode_to_vec(2)
                    .await
                    .expect("encode");
            assert!(bytes.starts_with(b"GIF89a"));
            assert_eq!(*bytes.last().unwrap(), 0x3B);
            assert_eq!(count_decoded_frames(&bytes), 2);
        }

        #[tokio::test]
        async fn never_on_two_frame_fixed_source_loops_without_trailer() {
            use futures_util::StreamExt;

            let source = FixedFrameSource::new(
                2,
                2,
                vec![solid(2, 2, [10, 20, 30]), solid(2, 2, [40, 50, 60])],
            );
            // Header + more frames than the file length → must have reset.
            const CHUNK_COUNT: usize = 6;
            let mut stream =
                GifStream::from_source(Duration::ZERO, TrailerPolicy::Never, Box::new(source))
                    .into_byte_stream();

            let mut chunks = Vec::with_capacity(CHUNK_COUNT);
            for _ in 0..CHUNK_COUNT {
                let chunk = stream
                    .next()
                    .await
                    .expect("Never must keep yielding")
                    .expect("chunk ok");
                chunks.push(chunk);
            }
            let bytes: Vec<u8> = chunks.iter().flat_map(|c| c.to_vec()).collect();
            assert!(bytes.starts_with(b"GIF89a"));
            assert_ne!(*bytes.last().unwrap(), 0x3B);
            // 1 header + 5 frames > 2 source frames ⇒ reset happened.
            assert_eq!(chunks.len(), CHUNK_COUNT);
        }

        #[tokio::test]
        async fn on_drop_ends_with_trailer_when_fixed_source_reaches_eof() {
            use futures_util::StreamExt;

            let source = FixedFrameSource::new(2, 2, vec![solid(2, 2, [1, 2, 3])]);
            let stream =
                GifStream::from_source(Duration::ZERO, TrailerPolicy::OnDrop, Box::new(source))
                    .into_byte_stream();
            let chunks: Vec<Bytes> = stream
                .collect::<Vec<_>>()
                .await
                .into_iter()
                .collect::<Result<Vec<_>, _>>()
                .expect("chunks");
            let bytes: Vec<u8> = chunks.into_iter().flat_map(|c| c.to_vec()).collect();
            assert!(bytes.starts_with(b"GIF89a"));
            assert_eq!(*bytes.last().unwrap(), 0x3B);
            assert_eq!(count_decoded_frames(&bytes), 1);
        }

        #[tokio::test]
        async fn uses_a_sixty_four_colour_global_palette() {
            let bytes = GifStream::new(Duration::ZERO, TrailerPolicy::AfterN(1))
                .encode_to_vec(1)
                .await
                .expect("encode");
            let mut options = gif::DecodeOptions::new();
            options.set_color_output(gif::ColorOutput::Indexed);
            let decoder = options
                .read_info(std::io::Cursor::new(&bytes))
                .expect("decode header");
            let palette = decoder
                .global_palette()
                .expect("GIF must have a global colour table");
            assert_eq!(
                palette.len() / 3,
                64,
                "global palette should be a 64-colour cube, not NeuQuant 256"
            );
        }

        #[tokio::test]
        async fn maps_full_red_to_a_red_palette_entry() {
            let source = FixedFrameSource::new(1, 1, vec![solid(1, 1, [255, 0, 0])]);
            let bytes =
                GifStream::from_source(Duration::ZERO, TrailerPolicy::AfterN(1), Box::new(source))
                    .encode_to_vec(1)
                    .await
                    .expect("encode");
            let mut options = gif::DecodeOptions::new();
            options.set_color_output(gif::ColorOutput::Indexed);
            let mut decoder = options
                .read_info(std::io::Cursor::new(&bytes))
                .expect("decode header");
            let index = decoder
                .read_next_frame()
                .expect("decode frame")
                .expect("one frame")
                .buffer[0] as usize;
            let palette = decoder.palette().expect("frame palette");
            assert_eq!(
                &palette[index * 3..index * 3 + 3],
                &[255, 0, 0],
                "255,0,0 must land on an exact red entry of the 4×4×4 cube"
            );
        }
    }
}
