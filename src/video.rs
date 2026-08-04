use crate::surface::Surface;
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use sha2::{Digest, Sha256};
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
        .filter(|v| *v != "0/0")
        .or(stream.r_frame_rate.as_deref())
        .unwrap_or("24/1")
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
    let frames = stream
        .nb_read_packets
        .as_deref()
        .and_then(|v| v.parse::<usize>().ok())
        .or_else(|| {
            stream
                .nb_frames
                .as_deref()
                .and_then(|v| v.parse::<usize>().ok())
        })
        .unwrap_or_else(|| (duration_seconds * fps).round() as usize);
    let start_time_seconds = stream.start_time.as_deref().unwrap_or("0").parse::<f64>()?;
    Ok(VideoInfo {
        width: stream.width.context("missing video width")?,
        height: stream.height.context("missing video height")?,
        fps,
        fps_expression,
        frames,
        duration_seconds,
        start_time_seconds,
        constant_frame_rate,
    })
}

pub fn sha256(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path)
        .with_context(|| format!("failed to read file for SHA-256: {}", path.display()))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
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
        let mut child = Command::new(ffmpeg)
            .args(["-hide_banner", "-loglevel", "error", "-i"])
            .arg(input)
            .args([
                "-map", "0:v:0", "-f", "rawvideo", "-pix_fmt", "rgba", "pipe:1",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
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
            .args(["-map", "0:v:0", "-map", "1:a:0?"])
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
