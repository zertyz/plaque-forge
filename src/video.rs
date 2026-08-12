//! FFmpeg and FFprobe adapter used by Rust workflows.
//!
//! Process invocation, video probing, raw-frame decoding, and encoding live here so
//! analysis and rendering do not depend on shell command construction.

use crate::surface::Surface;
use anyhow::{Context, Result, bail};
use opencv::{
    prelude::{VideoCaptureTrait, VideoCaptureTraitConst},
    videoio::{CAP_ANY, CAP_PROP_ORIENTATION_AUTO, VideoCapture},
};
use serde::Deserialize;
use std::{
    io::{ErrorKind, Read, Write},
    path::Path,
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
};

#[derive(Debug, Clone)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub fps_expression: String,
    pub frames: usize,
    pub duration_seconds: f64,
    pub start_time_seconds: f64,
    pub constant_frame_rate: bool,
    pub color_range: Option<String>,
    pub color_space: Option<String>,
    pub color_transfer: Option<String>,
    pub color_primaries: Option<String>,
    pub rotation_degrees: i32,
}

impl VideoInfo {
    pub fn ensure_supported_compositing_color(&self) -> Result<()> {
        if self
            .color_transfer
            .as_deref()
            .is_some_and(|value| matches!(value, "smpte2084" | "arib-std-b67"))
            || self
                .color_primaries
                .as_deref()
                .is_some_and(|value| value == "bt2020")
        {
            bail!(
                "HDR/wide-gamut input is not supported by the current 8-bit SDR compositor (primaries={}, transfer={}); normalize it to SDR BT.709 before analysis/rendering",
                self.color_primaries.as_deref().unwrap_or("unspecified"),
                self.color_transfer.as_deref().unwrap_or("unspecified")
            );
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
struct ProbeOutput {
    streams: Vec<ProbeStream>,
    format: ProbeFormat,
}
#[derive(Debug, Deserialize)]
struct ProbeStream {
    codec_type: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    avg_frame_rate: Option<String>,
    r_frame_rate: Option<String>,
    nb_frames: Option<String>,
    duration: Option<String>,
    nb_read_packets: Option<String>,
    start_time: Option<String>,
    color_range: Option<String>,
    color_space: Option<String>,
    color_transfer: Option<String>,
    color_primaries: Option<String>,
    #[serde(default)]
    tags: ProbeTags,
    #[serde(default)]
    side_data_list: Vec<ProbeSideData>,
}

#[derive(Debug, Default, Deserialize)]
struct ProbeTags {
    rotate: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProbeSideData {
    rotation: Option<i32>,
}
#[derive(Debug, Deserialize)]
struct ProbeFormat {
    duration: Option<String>,
}

pub fn probe(ffprobe: &Path, input: &Path) -> Result<VideoInfo> {
    let output = Command::new(ffprobe)
        .args([
            "-v",
            "error",
            "-count_packets",
            "-show_streams",
            "-show_format",
            "-of",
            "json",
        ])
        .arg(input)
        .output()
        .with_context(|| format!("failed to launch {}", ffprobe.display()))?;
    if !output.status.success() {
        bail!(
            "ffprobe failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let probe: ProbeOutput = serde_json::from_slice(&output.stdout)?;
    let stream = probe
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("video"))
        .context("input has no video stream")?;
    let fps_expression = stream
        .avg_frame_rate
        .as_deref()
        .filter(|value| *value != "0/0")
        .or_else(|| {
            stream
                .r_frame_rate
                .as_deref()
                .filter(|value| *value != "0/0")
        })
        .context("video stream does not report a usable frame rate")?
        .to_owned();
    let fps = parse_fraction(&fps_expression)?;
    let average_fps = stream
        .avg_frame_rate
        .as_deref()
        .filter(|value| *value != "0/0")
        .map(parse_fraction)
        .transpose()?;
    let nominal_fps = stream
        .r_frame_rate
        .as_deref()
        .filter(|value| *value != "0/0")
        .map(parse_fraction)
        .transpose()?;
    let constant_frame_rate = match (average_fps, nominal_fps) {
        (Some(average), Some(nominal)) => {
            (average - nominal).abs() <= nominal.abs().max(1.0) * 0.001
        }
        _ => true,
    };
    let duration_seconds = stream
        .duration
        .as_deref()
        .or(probe.format.duration.as_deref())
        .unwrap_or("0")
        .parse::<f64>()?;
    let metadata_frames = stream
        .nb_read_packets
        .as_deref()
        .and_then(|v| v.parse::<usize>().ok())
        .or_else(|| {
            stream
                .nb_frames
                .as_deref()
                .and_then(|v| v.parse::<usize>().ok())
        });
    let duration_frames = playable_frames_from_duration(duration_seconds, fps);
    let frames = select_playable_frame_count(metadata_frames, duration_frames);
    let start_time_seconds = stream.start_time.as_deref().unwrap_or("0").parse::<f64>()?;
    let rotation_degrees = stream
        .side_data_list
        .iter()
        .find_map(|side_data| side_data.rotation)
        .or_else(|| {
            stream
                .tags
                .rotate
                .as_deref()
                .and_then(|value| value.parse::<i32>().ok())
        })
        .unwrap_or(0);
    Ok(VideoInfo {
        width: stream.width.context("missing video width")?,
        height: stream.height.context("missing video height")?,
        fps,
        fps_expression,
        frames,
        duration_seconds,
        start_time_seconds,
        constant_frame_rate,
        color_range: stream.color_range.clone(),
        color_space: stream.color_space.clone(),
        color_transfer: stream.color_transfer.clone(),
        color_primaries: stream.color_primaries.clone(),
        rotation_degrees,
    })
}

/// OpenCV may otherwise autorotate frames while FFmpeg/raw geometry remains in
/// encoded coordinates. Keep every analysis backend in the same coordinate space.
pub fn open_capture(input: &Path) -> Result<VideoCapture> {
    let mut capture = VideoCapture::from_file(&input.to_string_lossy(), CAP_ANY)
        .with_context(|| format!("failed to open input video {}", input.display()))?;
    capture.set(CAP_PROP_ORIENTATION_AUTO, 0.0)?;
    if !capture.is_opened()? {
        bail!("input video could not be opened: {}", input.display());
    }
    Ok(capture)
}

pub struct Decoder {
    child: Child,
    stdout: ChildStdout,
    width: u32,
    height: u32,
    frame_size: usize,
}
impl Decoder {
    pub fn spawn(ffmpeg: &Path, input: &Path, info: &VideoInfo) -> Result<Self> {
        // A raw pipe carries no timestamps. Normalize them before muxing into that
        // pipe so broken/coarse container DTS cannot trigger FFmpeg diagnostics;
        // `fps_mode=passthrough` still preserves every decoded frame exactly once.
        let timestamp_filter = format!("setpts=N/({:.12}*TB)", info.fps);
        let mut command = Command::new(ffmpeg);
        command
            .args(["-hide_banner", "-loglevel", "error", "-noautorotate", "-i"])
            .arg(input)
            .args([
                "-map",
                "0:v:0",
                "-vf",
                &timestamp_filter,
                // Preserve the decoder's actual frame sequence. FFmpeg's default
                // output synchronization may duplicate frames to fill a nominal
                // container duration (for example 234 decodable frames advertised
                // as 240), which makes a verifier compare synthetic tail frames.
                "-fps_mode",
                "passthrough",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgba",
                "pipe:1",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        let mut child = command
            .spawn()
            .with_context(|| format!("failed to launch decoder {}", ffmpeg.display()))?;
        let stdout = child
            .stdout
            .take()
            .context("decoder stdout was not piped")?;
        Ok(Self {
            child,
            stdout,
            width: info.width,
            height: info.height,
            frame_size: info.width as usize * info.height as usize * 4,
        })
    }
    pub fn next_frame(&mut self) -> Result<Option<Surface>> {
        let mut bytes = vec![0u8; self.frame_size];
        let mut read = 0;
        while read < bytes.len() {
            match self.stdout.read(&mut bytes[read..]) {
                Ok(0) if read == 0 => return Ok(None),
                Ok(0) => bail!(
                    "decoder ended with a partial frame: {read}/{} bytes",
                    bytes.len()
                ),
                Ok(n) => read += n,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(e) => return Err(e).context("failed to read decoded frame"),
            }
        }
        Ok(Some(Surface::from_rgba(self.width, self.height, bytes)?))
    }
    pub fn finish(mut self) -> Result<()> {
        drop(self.stdout);
        let s = self.child.wait()?;
        if !s.success() {
            bail!("decoder exited with {s}")
        }
        Ok(())
    }
}

pub struct Encoder {
    child: Child,
    stdin: Option<ChildStdin>,
}
impl Encoder {
    pub fn spawn(
        ffmpeg: &Path,
        source_input: &Path,
        output: &Path,
        info: &VideoInfo,
        args: &[String],
    ) -> Result<Self> {
        if let Some(parent) = output
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            std::fs::create_dir_all(parent).with_context(|| {
                format!("failed to create output directory {}", parent.display())
            })?;
        }
        let mut command = Command::new(ffmpeg);
        command
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-y",
                "-f",
                "rawvideo",
                "-pix_fmt",
                "rgba",
                "-video_size",
            ])
            .arg(format!("{}x{}", info.width, info.height))
            .args(["-framerate", &info.fps_expression, "-i", "pipe:0", "-i"])
            .arg(source_input)
            .args([
                "-map",
                "0:v:0",
                "-map",
                "1:a:0?",
                "-map_metadata",
                "1",
                "-map_metadata:s:v:0",
                "1:s:v:0",
            ]);
        for (flag, value) in [
            ("-color_range", info.color_range.as_deref()),
            ("-colorspace", info.color_space.as_deref()),
            ("-color_trc", info.color_transfer.as_deref()),
            ("-color_primaries", info.color_primaries.as_deref()),
        ] {
            if let Some(value) = value {
                command.arg(flag).arg(value);
            }
        }
        if info.rotation_degrees != 0 {
            command
                .arg("-metadata:s:v:0")
                .arg(format!("rotate={}", info.rotation_degrees));
        }
        command
            .args(args)
            .args(["-avoid_negative_ts", "disabled"])
            .arg(output)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        let mut child = command.spawn().with_context(|| {
            format!(
                "failed to launch encoder {} for output {}",
                ffmpeg.display(),
                output.display()
            )
        })?;
        let stdin = child.stdin.take().context("encoder stdin not piped")?;
        Ok(Self {
            child,
            stdin: Some(stdin),
        })
    }
    pub fn write_frame(&mut self, frame: &Surface) -> Result<()> {
        self.stdin
            .as_mut()
            .context("encoder pipe is closed")?
            .write_all(frame.pixels())
            .context("failed to write raw frame to FFmpeg")?;
        Ok(())
    }
    pub fn finish(mut self) -> Result<()> {
        drop(self.stdin.take());
        let s = self.child.wait()?;
        if !s.success() {
            bail!("encoder exited with {s}")
        }
        Ok(())
    }
}

fn playable_frames_from_duration(duration_seconds: f64, fps: f64) -> usize {
    if !duration_seconds.is_finite() || duration_seconds <= 0.0 || !fps.is_finite() || fps <= 0.0 {
        return 0;
    }
    // For CFR video, stream duration describes the playable timeline more reliably
    // than packet metadata in some generated MP4 files. Floor rather than round: a
    // partial final frame interval is not itself a decodable frame.
    (duration_seconds * fps + 1.0e-6).floor().max(0.0) as usize
}

fn select_playable_frame_count(metadata_frames: Option<usize>, duration_frames: usize) -> usize {
    match (
        metadata_frames.filter(|&frames| frames > 0),
        duration_frames,
    ) {
        // Tolerate a one-frame timestamp/rounding discrepancy. Larger disagreement
        // means packet/frame metadata is advertising a stale, non-decodable tail.
        (Some(metadata), duration) if duration > 0 && metadata > duration + 1 => duration,
        (Some(metadata), _) => metadata,
        (None, duration) if duration > 0 => duration,
        (None, _) => 1,
    }
}

fn parse_fraction(value: &str) -> Result<f64> {
    if let Some((n, d)) = value.split_once('/') {
        let n: f64 = n.parse()?;
        let d: f64 = d.parse()?;
        if d == 0.0 {
            bail!("zero frame-rate denominator")
        }
        Ok(n / d)
    } else {
        Ok(value.parse()?)
    }
}

#[cfg(test)]
mod tests {
    use super::select_playable_frame_count;

    #[test]
    fn stale_packet_count_yields_to_shorter_playable_timeline() {
        assert_eq!(select_playable_frame_count(Some(240), 234), 234);
        assert_eq!(select_playable_frame_count(Some(240), 206), 206);
    }

    #[test]
    fn one_frame_duration_rounding_does_not_shorten_good_metadata() {
        assert_eq!(select_playable_frame_count(Some(240), 239), 240);
    }

    #[test]
    fn duration_is_used_when_frame_metadata_is_missing() {
        assert_eq!(select_playable_frame_count(None, 236), 236);
    }
}
