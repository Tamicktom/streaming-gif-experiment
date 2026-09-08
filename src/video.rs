//! Decode MP4 (or other ffmpeg-readable video) into sequential RGB24 frames.

//* Libraries imports
use std::env;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;

use tokio::io::{AsyncReadExt, BufReader};
use tokio::process::{Child, ChildStdout, Command};
use tokio::sync::Mutex;

//* Local imports
use crate::frames::{FrameSource, NextFrameFuture, RgbFrame};

/// Default downsample for lab video → GIF (confirmed against example.mp4).
pub const DEFAULT_SCALE_WIDTH: u32 = 480;
pub const DEFAULT_FPS: u32 = 1;

const INTEL_PCI_VENDOR: &str = "0x8086";

/// How ffmpeg should decode/scale frames.
#[derive(Debug, Clone, PartialEq, Eq)]
enum DecodePath {
    Software,
    Vaapi { device: PathBuf },
}

struct FfmpegInner {
    path: PathBuf,
    scale_width: u32,
    fps: u32,
    frame_len: usize,
    decode: DecodePath,
    child: Option<Child>,
    stdout: Option<BufReader<ChildStdout>>,
}

impl FfmpegInner {
    fn vf_filter(&self) -> String {
        match &self.decode {
            DecodePath::Software => {
                format!("fps={},scale={}:-2", self.fps, self.scale_width)
            }
            DecodePath::Vaapi { .. } => format!(
                "fps={},scale_vaapi=w={}:h=-2,hwdownload,format=nv12,format=rgb24",
                self.fps, self.scale_width
            ),
        }
    }

    fn ffmpeg_command(&self, extra_output_args: &[&str]) -> Command {
        let mut cmd = Command::new("ffmpeg");
        cmd.args(["-hide_banner", "-loglevel", "error"]);
        if let DecodePath::Vaapi { device } = &self.decode {
            cmd.args(["-hwaccel", "vaapi"])
                .arg("-hwaccel_device")
                .arg(device)
                .args(["-hwaccel_output_format", "vaapi"]);
        }
        cmd.args(["-an", "-i"]).arg(&self.path);
        cmd.args(["-vf", &self.vf_filter()]);
        cmd.args(extra_output_args);
        cmd.args(["-f", "rawvideo", "-pix_fmt", "rgb24", "pipe:1"]);
        cmd
    }

    async fn ensure_started(&mut self) -> io::Result<()> {
        if self.stdout.is_some() {
            return Ok(());
        }
        let mut child = self
            .ffmpeg_command(&[])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .map_err(|err| {
                io::Error::new(
                    err.kind(),
                    format!("failed to spawn ffmpeg (is it installed?): {err}"),
                )
            })?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("ffmpeg child missing stdout pipe"))?;
        self.stdout = Some(BufReader::new(stdout));
        self.child = Some(child);
        Ok(())
    }

    fn stop_child(&mut self) {
        self.stdout = None;
        if let Some(mut child) = self.child.take() {
            let _ = child.start_kill();
        }
    }

    async fn read_one_frame(&mut self) -> io::Result<Option<RgbFrame>> {
        self.ensure_started().await?;
        let stdout = self
            .stdout
            .as_mut()
            .expect("stdout present after ensure_started");
        let mut buf = vec![0u8; self.frame_len];
        match stdout.read_exact(&mut buf).await {
            Ok(_) => Ok(Some(RgbFrame { pixels: buf })),
            Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => {
                self.stop_child();
                Ok(None)
            }
            Err(err) => {
                self.stop_child();
                Err(err)
            }
        }
    }
}

impl Drop for FfmpegInner {
    fn drop(&mut self) {
        self.stop_child();
    }
}

/// ffmpeg-backed sequential RGB frame source.
pub struct FfmpegFrameSource {
    width: u16,
    height: u16,
    inner: Arc<Mutex<FfmpegInner>>,
}

impl FfmpegFrameSource {
    /// Probe scaled dimensions and prepare to spawn ffmpeg on first read.
    pub async fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        Self::open_with_options(path, DEFAULT_SCALE_WIDTH, DEFAULT_FPS).await
    }

    pub async fn open_with_options(
        path: impl AsRef<Path>,
        scale_width: u32,
        fps: u32,
    ) -> io::Result<Self> {
        let requested = decode_path_from_env();
        match Self::open_with_decode_path(&path, scale_width, fps, requested.clone()).await {
            Ok(source) => Ok(source),
            Err(err)
                if matches!(requested, DecodePath::Vaapi { .. })
                    && err.kind() != io::ErrorKind::NotFound =>
            {
                eprintln!("ffmpeg vaapi probe failed ({err}), falling back to software decode");
                Self::open_with_decode_path(path, scale_width, fps, DecodePath::Software).await
            }
            Err(err) => Err(err),
        }
    }

    async fn open_with_decode_path(
        path: impl AsRef<Path>,
        scale_width: u32,
        fps: u32,
        decode: DecodePath,
    ) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if !path.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("video file not found: {}", path.display()),
            ));
        }

        let (width, height) = probe_scaled_dimensions(&path, scale_width, fps, &decode).await?;
        let frame_len = (width as usize) * (height as usize) * 3;
        eprintln!("ffmpeg decode path: {}", format_decode_path(&decode));
        Ok(Self {
            width,
            height,
            inner: Arc::new(Mutex::new(FfmpegInner {
                path,
                scale_width,
                fps,
                frame_len,
                decode,
                child: None,
                stdout: None,
            })),
        })
    }
}

impl FrameSource for FfmpegFrameSource {
    fn width(&self) -> u16 {
        self.width
    }

    fn height(&self) -> u16 {
        self.height
    }

    fn next_frame(&mut self) -> NextFrameFuture {
        let inner = Arc::clone(&self.inner);
        Box::pin(async move {
            let mut guard = inner.lock().await;
            guard.read_one_frame().await
        })
    }

    fn reset(&mut self) -> io::Result<()> {
        // Try to stop without blocking the async runtime on a sync mutex.
        // `try_lock` is enough: GifByteStream never resets while a next_frame
        // future is still pending.
        match self.inner.try_lock() {
            Ok(mut guard) => {
                guard.stop_child();
                Ok(())
            }
            Err(_) => Err(io::Error::other(
                "cannot reset FfmpegFrameSource while a frame read is in progress",
            )),
        }
    }
}

fn decode_path_from_env() -> DecodePath {
    match env::var("GIF_HWACCEL") {
        Ok(value) if value.eq_ignore_ascii_case("off") || value.eq_ignore_ascii_case("none") => {
            DecodePath::Software
        }
        Ok(value) if value.eq_ignore_ascii_case("vaapi") => DecodePath::Vaapi {
            device: vaapi_device_from_env()
                .or_else(intel_vaapi_render_node)
                .unwrap_or_else(|| PathBuf::from("/dev/dri/renderD128")),
        },
        _ => {
            if let Some(device) = vaapi_device_from_env() {
                DecodePath::Vaapi { device }
            } else if let Some(device) = intel_vaapi_render_node() {
                DecodePath::Vaapi { device }
            } else {
                DecodePath::Software
            }
        }
    }
}

fn vaapi_device_from_env() -> Option<PathBuf> {
    env::var("GIF_VAAPI_DEVICE")
        .ok()
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// DRM render node whose PCI vendor is Intel (`0x8086`), if one exists.
fn intel_vaapi_render_node() -> Option<PathBuf> {
    let dir = std::fs::read_dir("/dev/dri").ok()?;
    let mut nodes: Vec<PathBuf> = dir
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("renderD"))
        })
        .collect();
    nodes.sort();
    nodes
        .into_iter()
        .find(|path| drm_vendor(path).as_deref() == Some(INTEL_PCI_VENDOR))
}

fn drm_vendor(render_node: &Path) -> Option<String> {
    let name = render_node.file_name()?.to_str()?;
    let vendor = std::fs::read_to_string(format!("/sys/class/drm/{name}/device/vendor")).ok()?;
    Some(vendor.trim().to_string())
}

fn format_decode_path(decode: &DecodePath) -> String {
    match decode {
        DecodePath::Software => "software".to_string(),
        DecodePath::Vaapi { device } => format!("vaapi device={}", device.display()),
    }
}

async fn probe_scaled_dimensions(
    path: &Path,
    scale_width: u32,
    fps: u32,
    decode: &DecodePath,
) -> io::Result<(u16, u16)> {
    // Decode one frame to rawvideo and infer height from byte length.
    // Using the same filter as the stream avoids ffprobe DAR/SAR surprises.
    let inner = FfmpegInner {
        path: path.to_path_buf(),
        scale_width,
        fps,
        frame_len: 0,
        decode: decode.clone(),
        child: None,
        stdout: None,
    };
    let output = inner
        .ffmpeg_command(&["-frames:v", "1"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(io::Error::other(format!(
            "ffmpeg probe failed for {}: {stderr}",
            path.display()
        )));
    }

    let bytes = output.stdout.len();
    if bytes == 0 || bytes % 3 != 0 {
        return Err(io::Error::other(format!(
            "ffmpeg probe produced unexpected byte count {bytes}"
        )));
    }
    let pixels = bytes / 3;
    let width = scale_width as usize;
    if pixels % width != 0 {
        return Err(io::Error::other(format!(
            "ffmpeg probe size {bytes} is not divisible by width {width} * 3"
        )));
    }
    let height = pixels / width;
    let width = u16::try_from(width)
        .map_err(|_| io::Error::other(format!("scaled width {width} exceeds u16")))?;
    let height = u16::try_from(height)
        .map_err(|_| io::Error::other(format!("scaled height {height} exceeds u16")))?;
    Ok((width, height))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as StdCommand;
    use tempfile::tempdir;

    fn write_tiny_mp4(path: &Path, frames: u32) {
        // 32×32, few solid frames, no audio — small fixture for unit tests.
        let status = StdCommand::new("ffmpeg")
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "lavfi",
                "-i",
                &format!("color=c=red:s=32x32:d={}", frames as f64 / 10.0),
                "-r",
                "10",
                "-frames:v",
                &frames.to_string(),
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
            ])
            .arg(path)
            .status()
            .expect("spawn ffmpeg to write fixture");
        assert!(status.success(), "ffmpeg fixture encode failed");
    }

    mod ffmpeg_frame_source {
        use super::*;

        #[tokio::test]
        async fn open_rejects_missing_files() {
            let err = FfmpegFrameSource::open("/tmp/definitely-missing-video-xyz.mp4")
                .await
                .err()
                .expect("missing file");
            assert_eq!(err.kind(), io::ErrorKind::NotFound);
        }

        #[tokio::test]
        async fn reads_scaled_rgb_frames_from_a_tiny_mp4_then_returns_none() {
            let dir = tempdir().unwrap();
            let path = dir.path().join("tiny.mp4");
            write_tiny_mp4(&path, 3);

            let mut source = FfmpegFrameSource::open_with_options(&path, 32, 10)
                .await
                .expect("open");
            assert_eq!(source.width(), 32);
            assert_eq!(source.height(), 32);

            for _ in 0..3 {
                let frame = source.next_frame().await.expect("ok").expect("frame");
                assert_eq!(frame.pixels.len(), 32 * 32 * 3);
            }
            assert!(source.next_frame().await.expect("ok").is_none());
        }

        #[tokio::test]
        async fn reset_allows_reading_the_file_again() {
            let dir = tempdir().unwrap();
            let path = dir.path().join("tiny.mp4");
            write_tiny_mp4(&path, 2);

            let mut source = FfmpegFrameSource::open_with_options(&path, 32, 10)
                .await
                .expect("open");
            assert!(source.next_frame().await.unwrap().is_some());
            assert!(source.next_frame().await.unwrap().is_some());
            assert!(source.next_frame().await.unwrap().is_none());
            source.reset().unwrap();
            assert!(source.next_frame().await.unwrap().is_some());
        }

        #[tokio::test]
        async fn reads_frames_when_hardware_decode_is_forced_off() {
            let dir = tempdir().unwrap();
            let path = dir.path().join("tiny.mp4");
            write_tiny_mp4(&path, 2);

            let mut source =
                FfmpegFrameSource::open_with_decode_path(&path, 32, 10, DecodePath::Software)
                    .await
                    .expect("open software");
            assert_eq!(source.width(), 32);
            assert_eq!(source.height(), 32);
            assert!(source.next_frame().await.unwrap().is_some());
            assert!(source.next_frame().await.unwrap().is_some());
            assert!(source.next_frame().await.unwrap().is_none());
        }

        #[tokio::test]
        async fn reads_frames_via_vaapi_when_an_intel_render_node_exists() {
            let Some(device) = intel_vaapi_render_node() else {
                return;
            };
            let dir = tempdir().unwrap();
            let path = dir.path().join("tiny.mp4");
            write_tiny_mp4(&path, 2);

            let mut source = FfmpegFrameSource::open_with_decode_path(
                &path,
                32,
                10,
                DecodePath::Vaapi { device },
            )
            .await
            .expect("open vaapi");
            assert_eq!(source.width(), 32);
            assert_eq!(source.height(), 32);
            let frame = source.next_frame().await.expect("ok").expect("frame");
            assert_eq!(frame.pixels.len(), 32 * 32 * 3);
        }

        #[tokio::test]
        async fn decodes_example_mp4_via_vaapi_at_lab_scale_when_the_file_is_present() {
            let path = Path::new("static/example.mp4");
            if !path.is_file() {
                return;
            }
            let Some(device) = intel_vaapi_render_node() else {
                return;
            };

            let mut source = FfmpegFrameSource::open_with_decode_path(
                path,
                DEFAULT_SCALE_WIDTH,
                DEFAULT_FPS,
                DecodePath::Vaapi { device },
            )
            .await
            .expect("open example.mp4 via vaapi");
            assert_eq!(source.width(), DEFAULT_SCALE_WIDTH as u16);
            assert!(source.height() > 0);
            let frame = source.next_frame().await.expect("ok").expect("frame");
            assert_eq!(
                frame.pixels.len(),
                (source.width() as usize) * (source.height() as usize) * 3
            );
        }
    }

    mod render_node_selection {
        use super::*;

        #[test]
        fn points_at_a_drm_node_with_pci_vendor_8086_when_one_exists() {
            let Some(path) = intel_vaapi_render_node() else {
                return;
            };
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .expect("render node file name");
            let vendor = std::fs::read_to_string(format!("/sys/class/drm/{name}/device/vendor"))
                .expect("sysfs vendor");
            assert_eq!(
                vendor.trim(),
                "0x8086",
                "must pick the Intel Arc render node, not the AMD iGPU"
            );
        }
    }
}
